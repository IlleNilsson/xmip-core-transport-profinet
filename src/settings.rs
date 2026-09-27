//! The settings a PROFINET Location takes, declared once and read through
//! (ADR-0064, amendment 2026-09-26).

use ethernet::{EthernetTransport, Mac};
use transport::Configured;
use transport::error::{Result, protocol_error};
use xcore::settings::{Applies, Fixed, Kind, Presence, Read, Setting, Settings};

use crate::cyclic::{RT_CLASS_1_FIRST, RT_CLASS_1_LAST};
use crate::{DEFAULT_TIMEOUT, ProfinetTransport};

impl Configured for ProfinetTransport {
    /// The address is the link the cycles go on, by its name, as the
    /// ethernet carrier reads it.
    const SETTINGS: &'static Settings = &Settings {
        technology: env!("CARGO_PKG_NAME"),
        settings: &[
            Setting {
                name: "controller",
                kind: Kind::Address,
                presence: Presence::Required,
                meaning: "The MAC address this controller cycles from and is cycled to.",
                applies: Applies::Both,
            },
            Setting {
                name: "device",
                kind: Kind::Address,
                presence: Presence::Required,
                meaning: "The MAC address of the IO device a Send Location cycles with unless \
                          the target names another.",
                applies: Applies::Send,
            },
            Setting {
                name: "frame_id",
                kind: Kind::Integer {
                    minimum: RT_CLASS_1_FIRST as i64,
                    maximum: RT_CLASS_1_LAST as i64,
                },
                presence: Presence::Default(Fixed::Integer(RT_CLASS_1_FIRST as i64)),
                meaning: "The real-time class 1 FrameID the cycles run under.",
                applies: Applies::Both,
            },
            Setting {
                name: "timeout",
                kind: Kind::Duration,
                presence: Presence::Default(Fixed::Duration(DEFAULT_TIMEOUT)),
                meaning: "How long a receive waits on a device that stops cycling.",
                applies: Applies::Receive,
            },
        ],
    };

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
        if let Some(timeout) = settings.optional_duration("timeout") {
            transport = transport.timing_out_after(timeout);
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
        assert_eq!(received.expect("built").timeout, DEFAULT_TIMEOUT);
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
