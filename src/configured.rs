//! How a PROFINET Location is configured: `Configured`, reading the
//! settings its [`declaration`](crate::declaration) declares once
//! (ADR-0064, amendments 2026-09-26 and 2026-11-26).

use ethernet::{EthernetTransport, Mac};
use transport::Configured;
use transport::error::{Result, protocol_error};
use xcore::settings::{Applies, Read, Settings};

use crate::{ProfinetTransport, declaration};

impl Configured for ProfinetTransport {
    const SETTINGS: &'static Settings = declaration::SETTINGS;
    const SCHEMES: &'static [&'static str] = declaration::SCHEMES;

    fn timeout_kept(&mut self) -> Option<&mut dyn transport::configured::TimeoutKept> {
        Some(&mut self.timeout)
    }

    /// The link is the carrier's: ethernet reads the address into one. A
    /// Receive Location cycles with whichever device answers it, so the
    /// unspecified address stands where a Send Location's device is.
    fn configured(address: &str, settings: &Read) -> Result<Self> {
        let carrier = EthernetTransport::open(address, Applies::Receive, &[])?;
        let mac = |name| {
            settings
                .optional_text(name)
                .map_or(Ok(Mac([0; 6])), str::parse::<Mac>)
        };
        let mut transport = Self::new(
            std::sync::Arc::clone(carrier.link()),
            mac("controller")?,
            mac("device")?,
        );
        if let Some(frame_id) = settings.optional_integer("frame_id") {
            let frame_id = u16::try_from(frame_id)
                .map_err(|_| protocol_error("a FrameID is at most 0xffff"))?;
            transport = transport.under(frame_id);
        }
        Ok(transport)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;
    use xcore::settings::Given;

    #[test]
    fn profinet_declares_its_settings_and_reads_through_them() {
        assert_eq!(ProfinetTransport::SETTINGS.problems(), Vec::<String>::new());
        let text = |name: &str, value: &str| (name.to_string(), Given::Text(value.to_string()));
        let given = [
            text("controller", "02:00:00:00:00:01"),
            text("device", "02:00:00:00:00:02"),
            ("frame_id".to_string(), Given::Integer(0x8001)),
        ];
        let built = ProfinetTransport::open("loopback", Applies::Send, &given).expect("built");
        assert_eq!(built.controller, Mac([2, 0, 0, 0, 0, 1]));
        assert_eq!(built.device, Mac([2, 0, 0, 0, 0, 2]));
        assert_eq!(built.frame_id, 0x8001);
        let received = ProfinetTransport::open("loopback", Applies::Receive, &given[..1]);
        assert_eq!(
            received.expect("built").timeout,
            declaration::DEFAULT_TIMEOUT
        );
        let late = [
            text("controller", "02:00:00:00:00:01"),
            text("timeout", "5s"),
        ];
        let built = ProfinetTransport::open("loopback", Applies::Receive, &late).expect("built");
        assert_eq!(built.timeout, Duration::from_secs(5));
        let Err(refused) = ProfinetTransport::open("loopback", Applies::Send, &given[..1]) else {
            panic!("device is required");
        };
        assert!(
            refused.message.contains("\"device\""),
            "{}",
            refused.message
        );
    }
}
