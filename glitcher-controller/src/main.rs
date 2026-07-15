#![no_std]
#![no_main]

use defmt::info;
use embassy_executor::Spawner;
// Don't remove
use embassy_rp as _;
use glitcher_rpc::Controller2HostMessage;
use {defmt_rtt as _, panic_probe as _};

#[embassy_executor::main]
async fn main(_spawner: Spawner) {
    let pong = Controller2HostMessage::Pong;
    info!("Message: {:?}", pong)
}
