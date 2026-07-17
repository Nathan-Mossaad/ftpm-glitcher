use embassy_rp::gpio::{Flex, Input, Output};
use embassy_rp::pio::Instance;
use embassy_time::{Delay, Duration, Timer, with_timeout};
use embedded_hal::delay::DelayNs;
use glitcher_rpc::Controller2HostMessage;

use crate::i2c_pio::I2cPio;
use crate::svi2;

pub async fn disable_telemetry<PIO: Instance, const SM: usize>(
    timeout_s: u32,
    pin18: &mut Input<'_>,
    pin19: &Input<'_>,
    pin20: &mut Output<'_>,
    i2c: &mut I2cPio<'_, PIO, SM>,
) -> Controller2HostMessage {
    match with_timeout(Duration::from_secs(timeout_s as u64), pin18.wait_for_high()).await {
        Ok(()) => {
            // Blocking wait
            Delay.delay_us(50);

            pin20.set_high();
            // Wait for one toggle to ensure proper timing
            if (0..10_000).any(|_| pin19.is_low()) && (0..10_000).any(|_| pin19.is_low()) {
            } else {
                return Controller2HostMessage::TelemetryTimedOut;
            }
            pin20.set_low();

            if (0..10_000).any(|_| pin19.is_low()) {
                // No waiting delay of PIO is roughly equals to length of one packet
                pin20.set_high();
                svi2::disable_telemetry(i2c);
                pin20.set_low();
                Controller2HostMessage::TelemetryDisabled
            } else {
                Controller2HostMessage::TelemetryTimedOut
            }
        }
        Err(_) => Controller2HostMessage::TelemetryTimedOut,
    }
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
