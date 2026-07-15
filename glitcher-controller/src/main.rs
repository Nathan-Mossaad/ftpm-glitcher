#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Flex, Pull};
use embassy_rp::watchdog::Watchdog;
use embassy_time::{Duration, Timer, with_timeout};
use glitcher_rpc::{
    Controller2HostMessage, FirmwareVersion, Host2ControllerMessage, RpcMessage, postcard,
};
use {defmt_rtt as _, panic_probe as _};

mod serial;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting USB serial!");

    let p = embassy_rp::init(Default::default());

    let mut class = serial::init(spawner, p.USB);
    let mut watchdog = Watchdog::new(p.WATCHDOG);

    // SPI0 chip-select monitor line (GPIO 5).
    let mut slave_cs_pin = Flex::new(p.PIN_5);

    loop {
        class.wait_connection().await;
        info!("Connected");

        let mut buf = [0; 64];
        loop {
            // Recieve
            let Ok(n) = class
                .read_packet(&mut buf)
                .await
                .map_err(serial::Disconnected::from)
            else {
                break;
            };

            // Handle message.
            let request =
                match postcard::from_bytes::<RpcMessage<Host2ControllerMessage>>(&buf[..n]) {
                    Ok(request) => request,
                    Err(error) => {
                        warn!("Unknown incoming message: {}", error);
                        continue;
                    }
                };

            let mut reboot_requested = false;
            info!("Received: {:?}", request.message);
            let message = match request.message {
                Host2ControllerMessage::Ping => Controller2HostMessage::Pong,
                Host2ControllerMessage::GetVersion => {
                    Controller2HostMessage::Version(firmware_version())
                }
                Host2ControllerMessage::Reboot => {
                    reboot_requested = true;
                    Controller2HostMessage::Rebooting
                }
                Host2ControllerMessage::CountChipSelects { timeout_s } => {
                    Controller2HostMessage::ChipSelectCount(
                        count_chip_selects(timeout_s, &mut slave_cs_pin).await,
                    )
                }
            };
            let response = RpcMessage {
                id: request.id,
                message,
            };
            info!("Sending: {:?}", response);

            // Serialize response
            let response_bytes = match postcard::to_slice(&response, &mut buf) {
                Ok(bytes) => bytes,
                Err(error) => {
                    warn!("Failed to serialize response: {}", error);
                    continue;
                }
            };

            // Send response
            if class
                .write_packet(response_bytes)
                .await
                .map_err(serial::Disconnected::from)
                .is_err()
            {
                break;
            }

            if reboot_requested {
                // Give the USB response time to reach the host before reset.
                Timer::after_millis(100).await;
                watchdog.trigger_reset();
            }
        }

        info!("Disconnected");
    }
}

fn firmware_version() -> FirmwareVersion {
    FirmwareVersion {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap(),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap(),
        patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap(),
    }
}

// Count chipselects with timeout in secs
async fn count_chip_selects(timeout_s: u32, slave_cs_pin: &mut Flex<'static>) -> u32 {
    // Another feature may have reconfigured this pin since the last count.
    slave_cs_pin.set_pull(Pull::None);
    slave_cs_pin.set_as_input();

    let mut count: u32 = 0;

    while with_timeout(
        Duration::from_secs(timeout_s as u64),
        slave_cs_pin.wait_for_falling_edge(),
    )
    .await
    .is_ok()
    {
        count = count.saturating_add(1);
    }

    count
}
