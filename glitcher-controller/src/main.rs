#![no_std]
#![no_main]

use defmt::{error, info, warn};
use embassy_executor::Spawner;
use embassy_rp::{
    bind_interrupts,
    gpio::{Flex, Input, Level, Output, Pull},
    peripherals::PIO0,
    pio,
    watchdog::Watchdog,
};
use embassy_time::Timer;
use glitcher_rpc::{
    ChunkStatus, Controller2HostMessage, FirmwareVersion, Host2ControllerMessage, RpcMessage,
    SPI_TAP_MAX_BYTES, postcard,
};
use static_cell::StaticCell;
use {defmt_rtt as _, panic_probe as _};

mod attack;
mod chip_select;
#[path = "i2c-pio.rs"]
mod i2c_pio;
mod serial;
mod spi_tap;
mod svi2;

bind_interrupts!(struct PioIrqs {
    PIO0_IRQ_0 => pio::InterruptHandler<PIO0>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    // Warning when running in debug
    if cfg!(debug_assertions) {
        for _ in 0..10 {
            error!("Warning: Running debug build, timings will be off!");
        }
    }

    info!("Starting USB serial!");

    let p = embassy_rp::init(Default::default());

    let mut class = serial::init(spawner, p.USB);
    let mut watchdog = Watchdog::new(p.WATCHDOG);

    // Retain the peripherals and lend short-lived reborrows to either the
    // chip-select counter or the SPI tap.
    let mut spi0 = p.SPI0;
    let mut spi_slave_clk = p.PIN_2;
    let mut spi_slave_miso = p.PIN_4;
    let mut spi_slave_cs_pin = p.PIN_5;
    let mut spi_tx_dma = p.DMA_CH2;
    let mut spi_rx_dma = p.DMA_CH3;
    static SPI_CAPTURE: StaticCell<[u8; SPI_TAP_MAX_BYTES]> = StaticCell::new();
    let capture = SPI_CAPTURE.init([0; SPI_TAP_MAX_BYTES]);

    // PIO I2C: SDA = GPIO16, SCL = GPIO17.
    let pio::Pio {
        mut common, sm0, ..
    } = pio::Pio::new(p.PIO0, PioIrqs);
    let mut i2c = i2c_pio::I2cPio::new(&mut common, sm0, p.PIN_16, p.PIN_17, 2_000_000);

    let mut svd_in = Input::new(p.PIN_18, Pull::None);
    let mut svc_in = Input::new(p.PIN_19, Pull::None);
    let mut logic_analyzer_trigger = Output::new(p.PIN_20, Level::Low);
    let mut target_reboot_pin = Flex::new(p.PIN_15);
    target_reboot_pin.set_pull(Pull::None);
    target_reboot_pin.set_as_input();
    let mut power_button_pin = Flex::new(p.PIN_14);
    power_button_pin.set_pull(Pull::None);
    power_button_pin.set_as_input();

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
                Host2ControllerMessage::RebootTarget => {
                    attack::reboot_target(&mut target_reboot_pin).await;
                    Controller2HostMessage::TargetRebooted
                }
                Host2ControllerMessage::PressPowerButton { duration_ms } => {
                    attack::press_power_button(&mut power_button_pin, duration_ms).await;
                    Controller2HostMessage::PowerButtonPressed
                }
                Host2ControllerMessage::GlitchAttack {
                    spi_byte_count,
                    vid,
                    chip_select_count,
                    wait_duration_ns,
                    dip_duration_ns,
                } => match attack::single_attack(
                    &mut target_reboot_pin,
                    &mut svd_in,
                    &mut svc_in,
                    &mut logic_analyzer_trigger,
                    &mut i2c,
                    &mut spi_slave_cs_pin,
                    &mut spi0,
                    &mut spi_slave_clk,
                    &mut spi_slave_miso,
                    &mut spi_tx_dma,
                    &mut spi_rx_dma,
                    capture,
                    spi_byte_count,
                    chip_select_count,
                    vid,
                    wait_duration_ns,
                    dip_duration_ns,
                )
                .await
                {
                    Ok(result) => match write_spi_tap_capture(
                        &mut class, request.id, capture, result, &mut buf,
                    )
                    .await
                    {
                        Ok(()) => continue,
                        Err(serial::ChunkWriteError::Disconnected) => break,
                        Err(serial::ChunkWriteError::Serialize(error)) => {
                            warn!("Failed to serialize chunked response: {}", error);
                            continue;
                        }
                    },
                    Err(message) => message,
                },
                Host2ControllerMessage::CountChipSelects { timeout_s, reboot } => {
                    if reboot {
                        attack::reboot_target(&mut target_reboot_pin).await;
                    }
                    Controller2HostMessage::ChipSelectCount(
                        chip_select::count_chip_selects(timeout_s, &mut spi_slave_cs_pin).await,
                    )
                }
                Host2ControllerMessage::TapSpi {
                    byte_count,
                    timeout_s,
                    reboot,
                } => {
                    if reboot {
                        attack::reboot_target(&mut target_reboot_pin).await;
                    }
                    let result = spi_tap::tap_spi(
                        &mut spi0,
                        &mut spi_slave_clk,
                        &mut spi_slave_miso,
                        &mut spi_slave_cs_pin,
                        &mut spi_tx_dma,
                        &mut spi_rx_dma,
                        capture,
                        byte_count,
                        timeout_s,
                    )
                    .await;

                    match result {
                        Ok(result) => match write_spi_tap_capture(
                            &mut class, request.id, capture, result, &mut buf,
                        )
                        .await
                        {
                            Ok(()) => continue,
                            Err(serial::ChunkWriteError::Disconnected) => break,
                            Err(serial::ChunkWriteError::Serialize(error)) => {
                                warn!("Failed to serialize chunked response: {}", error);
                                continue;
                            }
                        },
                        Err(error) => Controller2HostMessage::SpiTapError(error),
                    }
                }
                Host2ControllerMessage::SetVid { vid } => {
                    info!("Setting SVI2 VID to {:?}", vid);
                    svi2::set_vid(&mut i2c, vid);
                    Controller2HostMessage::VidSet
                }
                Host2ControllerMessage::DisableTelemetry { timeout_ms, reboot } => {
                    if reboot {
                        attack::reboot_target_with_trigger(
                            &mut target_reboot_pin,
                            &mut logic_analyzer_trigger,
                        )
                        .await;
                    }
                    let message = attack::wait_boot_and_disable_telemetry(
                        timeout_ms,
                        &mut svd_in,
                        &mut svc_in,
                        &mut logic_analyzer_trigger,
                        &mut i2c,
                    )
                    .await;
                    message
                }
            };
            let response = RpcMessage {
                id: request.id,
                message,
            };
            info!("Sending: {:?}", response);

            // Serialize response
            let response_bytes = match postcard::to_slice_cobs(&response, &mut buf) {
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

async fn write_spi_tap_capture(
    class: &mut serial::UsbSerial,
    request_id: u32,
    capture: &[u8; SPI_TAP_MAX_BYTES],
    result: spi_tap::SpiTapResult,
    buffer: &mut [u8; 64],
) -> Result<(), serial::ChunkWriteError> {
    let status = if result.timed_out {
        ChunkStatus::TimedOut
    } else {
        ChunkStatus::Complete
    };
    serial::write_chunked(
        class,
        request_id,
        &capture[..result.byte_count],
        status,
        buffer,
    )
    .await
}

fn firmware_version() -> FirmwareVersion {
    FirmwareVersion {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap(),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap(),
        patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap(),
    }
}
