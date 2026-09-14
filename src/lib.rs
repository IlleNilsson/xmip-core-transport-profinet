#![forbid(unsafe_code)]

//! Streams that ride PROFINET's cyclic IO data. One Stream is a run of
//! real-time class 1 frames: a chunk in each cycle's IO data, the cycle
//! counter stepping, the last cycle flagged, as many cycles as it takes.
//!
//! PROFINET is the process industry's Ethernet: the real-time classes
//! bypass IP altogether, a controller and a device exchanging one frame
//! each per cycle under `EtherType` `0x8892`, and DCP finding and naming
//! the devices before any cycle runs. What is here is DCP identify and set
//! ([`dcp`]), the cyclic frame with its APDU status trailer ([`cyclic`]),
//! a controller that speaks both, and [`Device`] — an IO device on an
//! in-process link that answers DCP and mirrors the outputs it is sent back
//! as its inputs, for tests and the loopback. The carrier is
//! `xmip-core-transport-ethernet`: this crate rides its [`Link`] and
//! [`Frame`] rather than knowing a wire of its own.
//!
//! The origin URI names the link, the device and the `FrameID`:
//! `profinet://<link>/<device mac>?frame=0x8000`. A target may name the
//! device, `profinet://<link>/<mac>`, or nothing for the configured one.

pub mod cyclic;
pub mod dcp;

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU16, AtomicU32, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

pub use cyclic::Cyclic;
pub use dcp::{Block, Dcp, Service};
use ethernet::{Frame, Link, Mac};
use transport::error::{Result, protocol_error};
use transport::loopback::{FarEnd, LOOPBACK_TIMEOUT, Loopback};
use transport::{Arrived, Directions, Transport};

use crate::cyclic::{CYCLE_STEP, RT_CLASS_1_FIRST};
use crate::dcp::{ETHERTYPE, MULTICAST};

/// The controller's side of one link, exchanging cycles with one device.
#[derive(Clone)]
pub struct ProfinetTransport {
    link: Arc<dyn Link>,
    controller: Mac,
    device: Mac,
    frame_id: u16,
    timeout: Duration,
    cycle: Arc<AtomicU16>,
    xid: Arc<AtomicU32>,
}

impl ProfinetTransport {
    /// A controller at `controller` on `link`, cycling with `device` under
    /// the first class 1 `FrameID`.
    #[must_use]
    pub fn new(link: Arc<dyn Link>, controller: Mac, device: Mac) -> Self {
        Self {
            link,
            controller,
            device,
            frame_id: RT_CLASS_1_FIRST,
            timeout: Duration::from_secs(1),
            cycle: Arc::new(AtomicU16::new(0)),
            xid: Arc::new(AtomicU32::new(1)),
        }
    }

    /// Cycle under another class 1 `FrameID`.
    #[must_use]
    pub const fn under(mut self, frame_id: u16) -> Self {
        self.frame_id = frame_id;
        self
    }

    /// Give up on a device that stops cycling.
    #[must_use]
    pub const fn timing_out_after(mut self, timeout: Duration) -> Self {
        self.timeout = timeout;
        self
    }

    /// `profinet://<link>/<mac>?frame=0x<frame id>`.
    #[must_use]
    pub fn origin(&self, device: Mac) -> String {
        format!(
            "profinet://{}/{device}?frame={:#06x}",
            self.link.name(),
            self.frame_id
        )
    }

    /// Who is on the wire: every device that answers an identify, with
    /// its name of station, until the link is quiet.
    ///
    /// # Errors
    /// Where the link could not be written or read.
    pub fn identify(&self) -> Result<Vec<(Mac, String)>> {
        let request = Dcp::identify_all(self.xid.fetch_add(1, Ordering::Relaxed));
        self.link.transmit(&Frame::new(
            MULTICAST,
            self.controller,
            ETHERTYPE,
            &request.encode(),
        )?)?;
        let mut found = Vec::new();
        while let Some(frame) = self.link.receive(self.timeout)? {
            if let Ok(Some(answer)) = Dcp::decode(&frame.payload)
                && answer.response
                && answer.xid == request.xid
            {
                found.push((frame.source, answer.name().unwrap_or_default()));
            }
        }
        Ok(found)
    }

    /// Name the device at `device` `name`, and wait for it to say so.
    ///
    /// # Errors
    /// A device that does not answer, or the link could not be used.
    pub fn set_name(&self, device: Mac, name: &str) -> Result<()> {
        let request = Dcp::set_name(self.xid.fetch_add(1, Ordering::Relaxed), name);
        self.link.transmit(&Frame::new(
            device,
            self.controller,
            ETHERTYPE,
            &request.encode(),
        )?)?;
        let deadline = Instant::now() + self.timeout;
        loop {
            if let Some(frame) = self.link.receive(self.timeout)? {
                if let Ok(Some(answer)) = Dcp::decode(&frame.payload)
                    && answer.response
                    && answer.xid == request.xid
                {
                    return Ok(());
                }
                continue;
            }
            if Instant::now() >= deadline {
                return Err(protocol_error("the device did not answer the set"));
            }
            std::thread::yield_now();
        }
    }

    /// `bytes` to `device` as cycles of output data.
    ///
    /// # Errors
    /// Where the link refused a frame.
    pub fn cycle_out(&self, device: Mac, bytes: &[u8]) -> Result<()> {
        let run = cyclic::cycles(self.frame_id, bytes, self.cycle.load(Ordering::Relaxed));
        let count = u16::try_from(run.len()).unwrap_or(u16::MAX);
        self.cycle
            .fetch_add(CYCLE_STEP.wrapping_mul(count), Ordering::Relaxed);
        for cycle in run {
            self.link.transmit(&Frame::new(
                device,
                self.controller,
                ETHERTYPE,
                &cycle.encode(),
            )?)?;
        }
        Ok(())
    }

    /// The next Stream the cycles on the link carry to this controller, or
    /// `None` when the link is quiet.
    ///
    /// # Errors
    /// A run of cycles that misses one or never ends, or a link that could
    /// not be read.
    pub fn cycle_in(&self) -> Result<Option<Arrived>> {
        let mut run = Vec::new();
        let deadline = Instant::now() + self.timeout;
        let from = loop {
            match self.link.receive(self.timeout)? {
                Some(frame)
                    if frame.ethertype == ETHERTYPE && frame.destination == self.controller =>
                {
                    if let Ok(cycle) = Cyclic::decode(&frame.payload) {
                        let last = cycle.chunk()?.1;
                        run.push(cycle);
                        if last {
                            break frame.source;
                        }
                    }
                }
                Some(_) => {}
                None if run.is_empty() => return Ok(None),
                None if Instant::now() >= deadline => {
                    return Err(protocol_error("the cycles stopped before the last"));
                }
                None => std::thread::yield_now(),
            }
        };
        let origin = self.origin(from);
        Ok(Some(Arrived::new(origin, cyclic::assemble(run)?)))
    }
}

impl Transport for ProfinetTransport {
    fn name(&self) -> &'static str {
        "profinet"
    }

    fn directions(&self) -> Directions {
        Directions::BOTH
    }

    /// Nothing on the link is not an error: an empty vector.
    fn receive(&self) -> Result<Vec<Arrived>> {
        Ok(self.cycle_in()?.into_iter().collect())
    }

    /// `target` may name a device, `profinet://eth0/02:00:00:00:00:02`,
    /// overriding the transport's.
    fn send(&self, target: &str, bytes: &[u8]) -> Result<()> {
        let device = match transport::socket::target("profinet", target) {
            Some((_, mac)) if !mac.is_empty() => mac.parse()?,
            _ => self.device,
        };
        self.cycle_out(device, bytes)
    }
}

/// An IO device on an in-process link: it answers DCP with its name, takes
/// a name it is set, and mirrors every cycle of outputs it is sent back to
/// the controller as its inputs.
pub struct Device {
    mac: Mac,
    name: Mutex<String>,
    to_controller: Mutex<VecDeque<Frame>>,
}

impl Device {
    #[must_use]
    pub fn new(mac: Mac, name: &str) -> Self {
        Self {
            mac,
            name: Mutex::new(name.to_string()),
            to_controller: Mutex::new(VecDeque::new()),
        }
    }

    /// The device's name of station, as it is now.
    #[must_use]
    pub fn name(&self) -> String {
        self.name
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn answer(&self, to: Mac, payload: &[u8]) -> Result<()> {
        let frame = Frame::new(to, self.mac, ETHERTYPE, payload)?;
        self.to_controller
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push_back(frame);
        Ok(())
    }

    fn take_dcp(&self, frame: &Frame, request: &Dcp) -> Result<()> {
        if request.response {
            return Ok(());
        }
        let blocks = match request.service {
            Service::Identify | Service::Get => {
                let mut data = vec![0, 0];
                data.extend_from_slice(self.name().as_bytes());
                vec![Block {
                    option: dcp::OPTION_DEVICE,
                    suboption: dcp::SUBOPTION_NAME,
                    data,
                }]
            }
            Service::Set => {
                if let Some(name) = request.name() {
                    *self.name.lock().unwrap_or_else(PoisonError::into_inner) = name;
                }
                Vec::new()
            }
        };
        self.answer(frame.source, &request.answered_with(blocks).encode())
    }
}

impl Link for Device {
    fn name(&self) -> &'static str {
        "loopback"
    }

    fn receive(&self, _timeout: Duration) -> Result<Option<Frame>> {
        Ok(self
            .to_controller
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front())
    }

    /// A frame that is not PROFINET, or not for this device, is dropped the
    /// way a port drops it.
    fn transmit(&self, frame: &Frame) -> Result<()> {
        if frame.ethertype != ETHERTYPE
            || !(frame.destination == self.mac || frame.destination.is_group())
        {
            return Ok(());
        }
        match Dcp::decode(&frame.payload) {
            Ok(Some(request)) => self.take_dcp(frame, &request),
            Ok(None) if frame.destination == self.mac => self.answer(frame.source, &frame.payload),
            _ => Ok(()),
        }
    }
}

impl ProfinetTransport {
    /// Both ends on one in-process link: a controller and one device named
    /// `loopback`, the loopback timeout on the controller.
    #[must_use]
    pub fn loopback() -> Self {
        let device = Mac([0x02, 0, 0, 0, 0, 2]);
        Self::new(
            Arc::new(Device::new(device, "loopback")),
            Mac([0x02, 0, 0, 0, 0, 1]),
            device,
        )
        .timing_out_after(LOOPBACK_TIMEOUT)
    }
}

/// The device mirroring the outputs it was sent, until they are read back
/// as its inputs.
struct Mirroring {
    controller: ProfinetTransport,
    address: String,
}

impl FarEnd for Mirroring {
    fn address(&self) -> &str {
        &self.address
    }

    fn take_one(self: Box<Self>) -> Result<Arrived> {
        self.controller
            .cycle_in()?
            .ok_or_else(|| protocol_error("no cycle came back from the device"))
    }
}

/// A Stream of any length rides as many cycles as it takes: no ceiling is a
/// fact of the protocol.
impl Loopback for ProfinetTransport {
    fn far_end(&self) -> Result<Box<dyn FarEnd>> {
        Ok(Box::new(Mirroring {
            controller: self.clone(),
            address: format!("profinet://{}/{}", self.link.name(), self.device),
        }))
    }

    fn send_to(&self, address: &str, payload: &[u8]) -> Result<()> {
        self.send(address, payload)
    }

    fn unblock(&self, _address: &str) {}

    /// In order on one thread: the device mirrors as the controller
    /// transmits, so the cycles go out first and the read-back takes them.
    fn round(&self, payload: &[u8]) -> Result<Arrived> {
        self.round_in_order(payload)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use transport::payload::edge_payloads;

    #[test]
    fn a_loopback_round_cycles_a_stream_out_and_mirrors_it_back() {
        let loopback = ProfinetTransport::loopback();
        let arrived = loopback.round(b"one cycle").expect("round");
        assert_eq!(arrived.bytes, b"one cycle");
        assert_eq!(
            arrived.origin_uri,
            "profinet://loopback/02:00:00:00:00:02?frame=0x8000"
        );
        let long = vec![0x2a; 1000];
        assert_eq!(loopback.round(&long).expect("28 cycles").bytes, long);
        assert!(loopback.ceiling().is_none());
        assert!(loopback.refuses(&long).is_none());
        assert_eq!(loopback.name(), "profinet");
        assert!(loopback.claims().is_none());
    }

    #[test]
    fn the_loopback_returns_the_edges_whole() {
        let loopback = ProfinetTransport::loopback();
        for (name, bytes) in edge_payloads() {
            let arrived = loopback
                .round(&bytes)
                .unwrap_or_else(|error| panic!("{name}: {error}"));
            assert_eq!(arrived.bytes, bytes, "{name}");
        }
    }

    #[test]
    fn dcp_finds_the_device_and_names_it() {
        let device = Arc::new(Device::new(Mac([2, 0, 0, 0, 0, 9]), "unnamed"));
        let controller = ProfinetTransport::new(
            Arc::clone(&device) as Arc<dyn Link>,
            Mac([2, 0, 0, 0, 0, 1]),
            Mac([2, 0, 0, 0, 0, 9]),
        )
        .timing_out_after(Duration::from_millis(100));
        assert_eq!(
            controller.identify().expect("identify"),
            vec![(Mac([2, 0, 0, 0, 0, 9]), "unnamed".to_string())]
        );
        controller
            .set_name(Mac([2, 0, 0, 0, 0, 9]), "valve-7")
            .expect("set");
        assert_eq!(device.name(), "valve-7");
        assert_eq!(controller.identify().expect("again")[0].1, "valve-7");
        let error = controller
            .set_name(Mac([2, 0, 0, 0, 0, 8]), "nobody")
            .expect_err("another device");
        assert!(error.message.contains("did not answer"), "{error}");
        assert!(controller.receive().expect("quiet").is_empty());
    }

    #[test]
    fn a_target_names_the_device_and_a_broken_run_is_refused() {
        let device = Arc::new(Device::new(Mac([2, 0, 0, 0, 0, 2]), "d"));
        let controller = ProfinetTransport::new(
            Arc::clone(&device) as Arc<dyn Link>,
            Mac([2, 0, 0, 0, 0, 1]),
            Mac([2, 0, 0, 0, 0, 3]),
        )
        .under(0x8010)
        .timing_out_after(Duration::from_millis(50));
        controller
            .send("profinet://loopback/02:00:00:00:00:02", &[1, 2, 3])
            .expect("named");
        let arrived = controller.cycle_in().expect("in").expect("one");
        assert_eq!(arrived.bytes, [1, 2, 3]);
        assert_eq!(
            arrived.origin_uri,
            "profinet://loopback/02:00:00:00:00:02?frame=0x8010"
        );
        controller
            .send("", &[4])
            .expect("the configured device, which is not there");
        assert!(controller.cycle_in().expect("quiet").is_none());
        assert!(controller.send("profinet://loopback/nope", &[]).is_err());
        // Two cycles of a run, the last never sent: the read-back times out.
        let run = cyclic::cycles(0x8010, &[9; 100], 0);
        for cycle in &run[..2] {
            device
                .transmit(
                    &Frame::new(
                        Mac([2, 0, 0, 0, 0, 2]),
                        Mac([2, 0, 0, 0, 0, 1]),
                        ETHERTYPE,
                        &cycle.encode(),
                    )
                    .expect("f"),
                )
                .expect("t");
        }
        assert!(controller.cycle_in().is_err(), "stopped before the last");
    }
}
