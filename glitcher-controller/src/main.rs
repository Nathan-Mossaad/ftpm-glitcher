#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use glitcher_rpc::{Controller2HostMessage, Host2ControllerMessage, postcard};
use {defmt_rtt as _, panic_probe as _};

mod serial;

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting USB serial!");

    let p = embassy_rp::init(Default::default());

    let mut class = serial::init(spawner, p.USB);

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

            // Handle message
            let response = match postcard::from_bytes::<Host2ControllerMessage>(&buf[..n]) {
                Ok(message) => {
                    info!("Received: {:?}", message);
                    match message {
                        Host2ControllerMessage::Ping => Controller2HostMessage::Pong,
                    }
                }
                Err(error) => {
                    warn!("Unknown incoming message: {}", error);
                    Controller2HostMessage::UnknownCommand
                }
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
        }

        info!("Disconnected");
    }
}
