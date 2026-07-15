#![no_std]

pub use postcard;

use serde::{Deserialize, Serialize};

// The parent package for all host to controller communication
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum Host2ControllerMessage {
    Ping,
}

// The parent package for all controller to host communication
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
#[cfg_attr(feature = "mcu", derive(defmt::Format))]
pub enum Controller2HostMessage {
    UnknownCommand,
    Pong,
}