//! AMD SVI2 command encoding and injection.
//!
//! The wire format matches the command layout used by the original Teensy
//! firmware in `target/teensy_firmware/amd_svi2.hpp`.

use embassy_rp::pio::Instance;

use crate::i2c_pio::I2cPio;

pub const CORE_BOOT_VID: u8 = 0x59;
pub const SOC_BOOT_VID: u8 = 0x60;

const SVI2_ADDRESS_PREFIX: u8 = 0b11000 << 2;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum OffsetTrim {
    Off = 0,
    Sub25Mv = 1,
    NoChange = 2,
    Add25Mv = 3,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum LoadLineSlopeTrim {
    Off = 0,
    Sub40Percent = 1,
    Sub20Percent = 2,
    NoChange = 3,
    Add20Percent = 4,
    Add40Percent = 5,
    Add60Percent = 6,
    Add80Percent = 7,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(u8)]
#[allow(dead_code)]
pub enum PowerLevel {
    Low = 0,
    Mid = 1,
    FullAlternate = 2,
    Full = 3,
}

/// Structured AMD SVI2 command.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Command {
    pub offset_trim: OffsetTrim,
    pub load_line_slope_trim: LoadLineSlopeTrim,
    pub disable_telemetry: bool,
    pub power_level: PowerLevel,
    pub vid: u8,
    pub soc: bool,
    pub core: bool,
}

impl Default for Command {
    fn default() -> Self {
        Self {
            offset_trim: OffsetTrim::NoChange,
            load_line_slope_trim: LoadLineSlopeTrim::NoChange,
            disable_telemetry: false,
            power_level: PowerLevel::Full,
            vid: 0xff,
            soc: false,
            core: false,
        }
    }
}

impl Command {
    pub const fn core(mut self, enabled: bool) -> Self {
        self.core = enabled;
        self
    }

    pub const fn soc(mut self, enabled: bool) -> Self {
        self.soc = enabled;
        self
    }

    pub const fn vid(mut self, vid: u8) -> Self {
        self.vid = vid;
        self
    }

    #[allow(dead_code)]
    pub const fn telemetry(mut self, disabled: bool) -> Self {
        self.disable_telemetry = disabled;
        self
    }

    #[allow(dead_code)]
    pub const fn power_level(mut self, power_level: PowerLevel) -> Self {
        self.power_level = power_level;
        self
    }

    #[allow(dead_code)]
    pub const fn load_line(mut self, load_line_slope_trim: LoadLineSlopeTrim) -> Self {
        self.load_line_slope_trim = load_line_slope_trim;
        self
    }

    #[allow(dead_code)]
    pub const fn offset(mut self, offset_trim: OffsetTrim) -> Self {
        self.offset_trim = offset_trim;
        self
    }

    /// Return the seven-bit SVI2 address encoded by the rail-select bits.
    pub const fn address(self) -> u8 {
        SVI2_ADDRESS_PREFIX | (self.soc as u8) | ((self.core as u8) << 1)
    }

    /// Return the two bytes in their on-wire order.
    pub const fn payload(self) -> [u8; 2] {
        let power = self.power_level as u16;
        let data = (self.offset_trim as u16)
            | ((self.load_line_slope_trim as u16) << 2)
            | ((self.disable_telemetry as u16) << 5)
            | (((power >> 1) & 1) << 6)
            | ((self.vid as u16) << 7)
            | ((power & 1) << 15);

        // The Teensy implementation swaps the packed u16 in `to_raw()` and
        // then sends that u16 least-significant byte first. This is equivalent
        // to sending the packed command in big-endian order.
        data.to_be_bytes()
    }

    pub fn send<PIO: Instance, const SM: usize>(self, i2c: &mut I2cPio<'_, PIO, SM>) {
        i2c.blocking_write(self.address(), &self.payload());
    }
}

/// Apply a VID to both rails, or restore the Teensy firmware's boot VIDs.
pub fn set_vid<PIO: Instance, const SM: usize>(i2c: &mut I2cPio<'_, PIO, SM>, vid: Option<u8>) {
    if let Some(vid) = vid {
        Command::default().soc(true).core(true).vid(vid).send(i2c);
    } else {
        // Match the recovery order used by the Teensy glitch firmware.
        Command::default().soc(true).vid(SOC_BOOT_VID).send(i2c);
        // Not needed on ryzen (may be needed on epyc)
        // Command::default().core(true).vid(CORE_BOOT_VID).send(i2c);
    }
}

/// Set the SVI2 telemetry-function bit to disable telemetry.
///
/// This mirrors the Teensy firmware's restart path: SVI2 uses the `core`
/// select bit together with `TFN=1` to disable telemetry, while restoring the
/// VCore boot VID as part of the same command.
#[allow(dead_code)]
pub fn disable_telemetry<PIO: Instance, const SM: usize>(i2c: &mut I2cPio<'_, PIO, SM>) {
    Command::default()
        .core(true)
        .vid(CORE_BOOT_VID)
        .telemetry(true)
        .send(i2c);
}
