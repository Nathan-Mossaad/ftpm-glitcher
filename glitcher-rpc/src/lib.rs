#![no_std]

pub use postcard;

use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub struct FirmwareVersion {
    pub major: u16,
    pub minor: u16,
    pub patch: u16,
}

impl core::fmt::Display for FirmwareVersion {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch)
    }
}

// The parent package for all host to controller communication
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum Host2ControllerMessage {
    Ping,
    GetVersion,
}

// The parent package for all controller to host communication
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum Controller2HostMessage {
    UnknownCommand,
    Pong,
    Version(FirmwareVersion),
}
