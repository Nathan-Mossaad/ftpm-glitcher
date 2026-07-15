#![no_std]

pub use postcard;

use serde::{Deserialize, Serialize};

/// The largest SPI transaction the tap currently captures.
///
/// SPI tap captures are deliberately limited to one USB/RPC packet for now.
pub const SPI_TAP_MAX_BYTES: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum SpiTapError {
    InvalidByteCount,
    ReadFailed,
}

impl core::fmt::Display for SpiTapError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Self::InvalidByteCount => {
                write!(f, "byte count must be between 1 and {SPI_TAP_MAX_BYTES}")
            }
            Self::ReadFailed => write!(f, "SPI read failed"),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub struct RpcMessage<T> {
    pub id: u32,
    pub message: T,
}

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
    Reboot,
    /// timeout in secs
    CountChipSelects {
        timeout_s: u32,
    },
    /// Capture one SPI0 slave transaction. `byte_count` must be 1..=32.
    TapSpi {
        byte_count: u8,
        timeout_s: u32,
    },
}

// The parent package for all controller to host communication
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum Controller2HostMessage {
    UnknownCommand,
    Pong,
    Rebooting,
    Version(FirmwareVersion),
    ChipSelectCount(u32),
    /// One SPI tap capture. `byte_count` indicates how many leading bytes are
    /// valid; `timed_out` distinguishes a partial capture from a full one.
    SpiTap {
        data: [u8; SPI_TAP_MAX_BYTES],
        byte_count: u8,
        timed_out: bool,
    },
    SpiTapError(SpiTapError),
}
