use std::io::{ErrorKind, Write};
use std::path::Path;
use std::time::Duration;

use anyhow::{Result, bail};
use glitcher_rpc::{
    CHUNK_BYTES, Chunk, ChunkStatus, Controller2HostMessage, Host2ControllerMessage, RpcMessage,
    SPI_TAP_MAX_BYTES, postcard,
};
use rand::random;
use serialport::SerialPort;
use tracing::{info, warn};

const BAUD_RATE: u32 = 115_200;
const BUFFER_SIZE: usize = 64;

pub struct SpiTapCapture {
    pub data: Vec<u8>,
    pub timed_out: bool,
}

pub struct ChunkTransfer {
    pub data: Vec<u8>,
    pub status: ChunkStatus,
}

/// Send a controller message to the Pico and wait for its response.
pub fn send(port_name: &Path, message: &Host2ControllerMessage) -> Result<Controller2HostMessage> {
    let (mut port, request_id) = open_and_send(port_name, message)?;
    let mut reader = ResponseReader::new();
    loop {
        let response = reader.next(&mut *port)?;
        if response.id == request_id {
            info!(request_id, response = ?response.message, "Received response");
            return Ok(response.message);
        }

        warn!(
            expected_request_id = request_id,
            received_request_id = response.id,
            "Discarding stale response"
        );
    }
}

/// Capture an SPI transaction and reassemble its COBS-framed response chunks.
pub fn tap_spi(
    port_name: &Path,
    byte_count: u16,
    timeout_s: u32,
    reboot: bool,
) -> Result<SpiTapCapture> {
    let message = Host2ControllerMessage::TapSpi {
        byte_count,
        timeout_s,
        reboot,
    };
    let (mut port, request_id) = open_and_send(port_name, &message)?;
    let mut reader = ResponseReader::new();
    let transfer = receive_chunks(
        &mut *port,
        &mut reader,
        request_id,
        SPI_TAP_MAX_BYTES,
        |message| match message {
            Controller2HostMessage::Chunk(chunk) => Ok(chunk),
            Controller2HostMessage::SpiTapError(error) => bail!("SPI tap failed: {error}"),
            message => {
                bail!("Pico returned an unexpected response to an SPI tap request: {message:?}")
            }
        },
    )?;
    Ok(SpiTapCapture {
        data: transfer.data,
        timed_out: matches!(transfer.status, ChunkStatus::TimedOut),
    })
}

/// Run a glitch attack and reassemble its post-attack SPI capture.
pub fn glitch_attack(
    port_name: &Path,
    spi_byte_count: u16,
    vid: u8,
    chip_select_count: u32,
    wait_duration_ns: u32,
    dip_duration_ns: u32,
) -> Result<SpiTapCapture> {
    let message = Host2ControllerMessage::GlitchAttack {
        spi_byte_count,
        vid,
        chip_select_count,
        wait_duration_ns,
        dip_duration_ns,
    };
    let (mut port, request_id) = open_and_send(port_name, &message)?;
    let mut reader = ResponseReader::new();
    let transfer = receive_chunks(
        &mut *port,
        &mut reader,
        request_id,
        SPI_TAP_MAX_BYTES,
        |message| match message {
            Controller2HostMessage::Chunk(chunk) => Ok(chunk),
            Controller2HostMessage::SpiTapError(error) => bail!("attack SPI tap failed: {error}"),
            Controller2HostMessage::GlitchAttackFailed => bail!("glitch attack failed"),
            Controller2HostMessage::GlitchAttackFailedTargetNotRunning => {
                bail!("glitch attack failed because the target was not running")
            }
            message => {
                bail!("Pico returned an unexpected response to a glitch attack: {message:?}")
            }
        },
    )?;
    Ok(SpiTapCapture {
        data: transfer.data,
        timed_out: matches!(transfer.status, ChunkStatus::TimedOut),
    })
}

pub fn receive_chunks<F>(
    port: &mut dyn SerialPort,
    reader: &mut ResponseReader,
    request_id: u32,
    max_len: usize,
    chunk_from_message: F,
) -> Result<ChunkTransfer>
where
    F: Fn(Controller2HostMessage) -> Result<Chunk>,
{
    let mut data = Vec::new();
    loop {
        let response = reader.next(port)?;
        if response.id != request_id {
            warn!(
                expected_request_id = request_id,
                received_request_id = response.id,
                "Discarding stale response"
            );
            continue;
        }

        let chunk = chunk_from_message(response.message)?;
        let offset = usize::from(chunk.offset);
        let chunk_len = usize::from(chunk.byte_count);
        if chunk_len > CHUNK_BYTES {
            bail!("controller sent an oversized chunk");
        }
        if offset != data.len() {
            bail!("chunks arrived out of order");
        }
        if data.len() + chunk_len > max_len {
            bail!("controller sent more than {max_len} chunked bytes");
        }
        data.extend_from_slice(&chunk.data[..chunk_len]);

        if chunk.is_last {
            return Ok(ChunkTransfer {
                data,
                status: chunk.status,
            });
        }
    }
}

fn open_and_send(
    port_name: &Path,
    message: &Host2ControllerMessage,
) -> Result<(Box<dyn SerialPort>, u32)> {
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
    Ok((port, request.id))
}

pub(crate) struct ResponseReader {
    encoded: Vec<u8>,
    pending: Vec<u8>,
}

impl ResponseReader {
    fn new() -> Self {
        Self {
            encoded: Vec::with_capacity(BUFFER_SIZE),
            pending: Vec::with_capacity(BUFFER_SIZE),
        }
    }

    fn next(&mut self, port: &mut dyn SerialPort) -> Result<RpcMessage<Controller2HostMessage>> {
        let mut read_buf = [0; BUFFER_SIZE];

        loop {
            if let Some(frame_end) = self.pending.iter().position(|&byte| byte == 0) {
                self.encoded.extend(self.pending.drain(..=frame_end));
                let response = postcard::from_bytes_cobs(&mut self.encoded)?;
                self.encoded.clear();
                return Ok(response);
            }
            self.encoded.append(&mut self.pending);
            if self.encoded.len() >= BUFFER_SIZE {
                bail!("controller sent an oversized COBS response frame");
            }

            let num_bytes = loop {
                match port.read(&mut read_buf) {
                    Ok(num_bytes) => break num_bytes,
                    Err(error) if error.kind() == ErrorKind::TimedOut => continue,
                    Err(error) => return Err(error.into()),
                }
            };
            self.pending.extend_from_slice(&read_buf[..num_bytes]);
        }
    }
}
