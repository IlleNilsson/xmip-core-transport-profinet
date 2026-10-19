//! An IO device on an in-process link, what a PROFINET controller's
//! loopback stands up (ADR-0051).

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};
use std::time::Duration;

use ethernet::{Frame, Link, Mac};
use transport::error::Result;

use crate::dcp::{self, Block, Dcp, ETHERTYPE, Service};

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
