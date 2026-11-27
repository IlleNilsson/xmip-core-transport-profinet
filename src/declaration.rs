//! This technology's declaration: the transport guide's "The declaration",
//! `module/core/capability/transport/doc/adding-a-transport.md`. It is the
//! same on every operating system, those without the kernel's raw socket
//! included: a Location's settings are suggested and checked wherever it is
//! designed.

use std::time::Duration;

use xcore::settings::{Applies, Fixed, Kind, Presence, Setting, Settings};

/// The first `FrameID` of real-time class 1, IEC 61158-6-10 clause 4.7.
pub const RT_CLASS_1_FIRST: u16 = 0x8000;
/// The last `FrameID` of real-time class 1.
pub const RT_CLASS_1_LAST: u16 = 0xbbff;

/// How long a controller waits on a device unless a Location says.
pub const DEFAULT_TIMEOUT: Duration = Duration::from_secs(1);

/// Every setting a Location takes beyond its address, the link the cycles
/// go on, by its name, as the ethernet carrier reads it.
pub const SETTINGS: &Settings = &Settings {
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
        transport::configured::timeout(
            Applies::Receive,
            Some(DEFAULT_TIMEOUT),
            "How long a receive waits on a device that stops cycling.",
        ),
    ],
};

/// This technology's schemes, as [`transport::Configured::SCHEMES`] defines
/// them.
pub const SCHEMES: &[&str] = &["profinet"];
