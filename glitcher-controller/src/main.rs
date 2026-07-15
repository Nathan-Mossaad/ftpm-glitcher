#![no_std]
#![no_main]

use defmt::{info, warn};
use embassy_executor::Spawner;
use embassy_rp::gpio::{Flex, Pull};
use embassy_rp::peripherals::{DMA_CH0, DMA_CH1, DMA_CH2, DMA_CH3, PIN_2, PIN_4, PIN_5, SPI0};
use embassy_rp::spi::{Config, Spi};
use embassy_rp::watchdog::Watchdog;
use embassy_rp::{Peri, bind_interrupts, dma};
use embassy_time::{Duration, Timer, with_timeout};
use glitcher_rpc::{
    Controller2HostMessage, FirmwareVersion, Host2ControllerMessage, RpcMessage, SPI_TAP_MAX_BYTES,
    SpiTapError, postcard,
};
use {defmt_rtt as _, panic_probe as _};

mod serial;

bind_interrupts!(struct Irqs {
    DMA_IRQ_0 => dma::InterruptHandler<DMA_CH0>, dma::InterruptHandler<DMA_CH1>, dma::InterruptHandler<DMA_CH2>, dma::InterruptHandler<DMA_CH3>;
});

#[embassy_executor::main]
async fn main(spawner: Spawner) {
    info!("Starting USB serial!");

    let p = embassy_rp::init(Default::default());

    let mut class = serial::init(spawner, p.USB);
    let mut watchdog = Watchdog::new(p.WATCHDOG);

    // Retain the peripherals and lend short-lived reborrows to either the
    // chip-select counter or the SPI tap.
    let mut spi0 = p.SPI0;
    let mut slave_clk = p.PIN_2;
    let mut slave_miso = p.PIN_4;
    let mut slave_cs_pin = p.PIN_5;
    let mut spi_tx_dma = p.DMA_CH2;
    let mut spi_rx_dma = p.DMA_CH3;

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
                Host2ControllerMessage::TapSpi {
                    byte_count,
                    timeout_s,
                } => {
                    tap_spi(
                        &mut spi0,
                        &mut slave_clk,
                        &mut slave_miso,
                        &mut slave_cs_pin,
                        &mut spi_tx_dma,
                        &mut spi_rx_dma,
                        byte_count,
                        timeout_s,
                    )
                    .await
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

async fn count_chip_selects(timeout_s: u32, slave_cs_pin: &mut Peri<'static, PIN_5>) -> u32 {
    let mut slave_cs_pin = Flex::new(slave_cs_pin.reborrow());
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

async fn tap_spi(
    spi0: &mut Peri<'static, SPI0>,
    slave_clk: &mut Peri<'static, PIN_2>,
    slave_miso: &mut Peri<'static, PIN_4>,
    slave_cs_pin: &mut Peri<'static, PIN_5>,
    spi_tx_dma: &mut Peri<'static, DMA_CH2>,
    spi_rx_dma: &mut Peri<'static, DMA_CH3>,
    byte_count: u8,
    timeout_s: u32,
) -> Controller2HostMessage {
    let byte_count = usize::from(byte_count);
    if !(1..=SPI_TAP_MAX_BYTES).contains(&byte_count) {
        return Controller2HostMessage::SpiTapError(SpiTapError::InvalidByteCount);
    }

    let mut capture = [0; SPI_TAP_MAX_BYTES];
    let mut config = Config::default();
    config.phase = embassy_rp::spi::Phase::CaptureOnSecondTransition;
    config.polarity = embassy_rp::spi::Polarity::IdleHigh;
    let mut spi = Spi::new_slave_rxonly(
        spi0.reborrow(),
        slave_clk.reborrow(),
        slave_miso.reborrow(),
        slave_cs_pin.reborrow(),
        spi_tx_dma.reborrow(),
        spi_rx_dma.reborrow(),
        Irqs,
        config,
    );
    let result = with_timeout(
        Duration::from_secs(timeout_s as u64),
        spi.read(&mut capture[..byte_count]),
    )
    .await;

    // On timeout, dropping the SPI read future aborts the RX DMA transfer.
    // The RP2040 DMA transfer count then reports how many bytes were still
    // outstanding, so the leading bytes in `capture` are valid received data.
    drop(spi);
    let remaining = embassy_rp::pac::DMA.ch(3).trans_count().read() as usize;

    match result {
        Ok(Ok(())) => Controller2HostMessage::SpiTap {
            data: capture,
            byte_count: byte_count as u8,
            timed_out: false,
        },
        Ok(Err(_)) => Controller2HostMessage::SpiTapError(SpiTapError::ReadFailed),
        Err(_) => Controller2HostMessage::SpiTap {
            data: capture,
            byte_count: byte_count.saturating_sub(remaining.min(byte_count)) as u8,
            timed_out: true,
        },
    }
}
