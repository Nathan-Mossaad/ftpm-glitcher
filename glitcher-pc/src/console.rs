use std::io::{ErrorKind, Read, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::Result;
use glitcher_rpc::{Controller2HostMessage, Host2ControllerMessage, RpcMessage, postcard};
use rand::random;
use tracing::{info, warn};

const BAUD_RATE: u32 = 115_200;
const BUFFER_SIZE: usize = 64;

/// Send a controller message to the Pico and wait for its response.
pub fn send(port_name: &Path, message: &Host2ControllerMessage) -> Result<Controller2HostMessage> {
    let mut port = serialport::new(port_name.to_string_lossy(), BAUD_RATE)
        .timeout(Duration::from_secs(1))
        .open()?;

    let request = RpcMessage {
        id: random::<u32>(),
        message: *message,
    };
    let mut buf = [0; BUFFER_SIZE];
    let message_bytes = postcard::to_slice(&request, &mut buf)?;
    info!(request_id = request.id, message = ?message, "Sending message");

    port.write_all(message_bytes)?;
    port.flush()?;

    loop {
        let num_bytes = loop {
            match port.read(&mut buf) {
                Ok(num_bytes) => break num_bytes,
                // The Pico may keep counting while CS remains active. Keep waiting
                // until it returns the final response after one second of inactivity.
                Err(error) if error.kind() == ErrorKind::TimedOut => continue,
                Err(error) => return Err(error.into()),
            }
        };
        let response =
            postcard::from_bytes::<RpcMessage<Controller2HostMessage>>(&buf[..num_bytes])?;

        if response.id == request.id {
            info!(request_id = request.id, response = ?response.message, "Received response");
            return Ok(response.message);
        }

        warn!(
            expected_request_id = request.id,
            received_request_id = response.id,
            "Discarding stale response"
        );
    }
}
