use std::io::{Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use glitcher_rpc::{Controller2HostMessage, Host2ControllerMessage, postcard};
use tracing::info;

const BAUD_RATE: u32 = 115_200;
const BUFFER_SIZE: usize = 64;

/// Send a controller message to the Pico and wait for its response.
pub fn send(
    port_name: &Path,
    message: &Host2ControllerMessage,
) -> Result<Controller2HostMessage> {
    let mut port = serialport::new(port_name.to_string_lossy(), BAUD_RATE)
        .timeout(Duration::from_secs(1))
        .open()?;

    let mut buf = [0; BUFFER_SIZE];
    let message_bytes = postcard::to_slice(message, &mut buf)?;
    info!(message = ?message, "Sending message");

    port.write_all(message_bytes)?;
    port.flush()?;

    let num_bytes = port.read(&mut buf)?;
    let response = postcard::from_bytes(&buf[..num_bytes])?;
    info!(response = ?response, "Received response");

    Ok(response)
}
