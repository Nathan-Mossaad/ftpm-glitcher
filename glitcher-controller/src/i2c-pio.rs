//! Blocking I2C master implemented with an RP2040 PIO state machine.
//!
//! This is adapted from Raspberry Pi's `pio/i2c` example. Both pins are
//! open-drain: the PIO output-enable signal is inverted, so a PINDIRS value of
//! zero drives the line low and a value of one releases it to the pull-up.

use embassy_rp::Peri;
use embassy_rp::clocks::clk_sys_freq;
use embassy_rp::gpio::{Level, Pull};
use embassy_rp::pio::{Common, Config, Direction, Instance, PioPin, ShiftDirection, StateMachine};
use fixed::traits::ToFixed;
use fixed::types::extra::U8;

const CYCLES_PER_I2C_BIT: u32 = 32;

/// PIO-backed I2C master.
///
/// NACKs are deliberately ignored by the PIO program. This is useful for the
/// controller's fault-injection traffic, where an absent or disrupted target
/// must not halt the state machine.
pub struct I2cPio<'d, PIO: Instance, const SM: usize> {
    sm: StateMachine<'d, PIO, SM>,
    bus_state_instructions: [u16; 4],
}

impl<'d, PIO: Instance, const SM: usize> I2cPio<'d, PIO, SM> {
    pub fn new(
        common: &mut Common<'d, PIO>,
        mut sm: StateMachine<'d, PIO, SM>,
        sda: Peri<'d, impl PioPin>,
        scl: Peri<'d, impl PioPin>,
        frequency_hz: u32,
    ) -> Self {
        assert!(frequency_hz != 0);

        let program = pio::pio_asm!(
            r#"
                .side_set 1 opt pindirs

                do_nack:
                    ; Fault-injection traffic intentionally ignores NACKs.
                    jmp entry_point

                do_byte:
                    set x, 7
                bitloop:
                    out pindirs, 1         [7]
                    nop             side 1 [2]
                    wait 1 pin, 1          [4]
                    in pins, 1             [7]
                    jmp x-- bitloop side 0 [7]

                    out pindirs, 1         [7]
                    nop             side 1 [7]
                    wait 1 pin, 1          [7]
                    jmp pin do_nack side 0 [2]

                public entry_point:
                .wrap_target
                    out x, 6
                    out y, 1
                    jmp !x do_byte
                    out null, 32
                do_exec:
                    out exec, 16
                    jmp x-- do_exec
                .wrap
            "#
        );
        let loaded = common.load_program(&program.program);

        // These instructions are not loaded as a runnable program. They are
        // fed through OUT EXEC to generate START and STOP transitions.
        let bus_states = pio::pio_asm!(
            r#"
                .side_set 1 opt
                set pindirs, 0 side 0 [7]
                set pindirs, 1 side 0 [7]
                set pindirs, 0 side 1 [7]
                set pindirs, 1 side 1 [7]
            "#
        );
        let bus_state_instructions = [
            bus_states.program.code[0],
            bus_states.program.code[1],
            bus_states.program.code[2],
            bus_states.program.code[3],
        ];

        let mut sda = common.make_pio_pin(sda);
        let mut scl = common.make_pio_pin(scl);
        sda.set_pull(Pull::Up);
        scl.set_pull(Pull::Up);
        sda.set_output_enable_inversion(true);
        scl.set_output_enable_inversion(true);

        let mut config = Config::default();
        config.use_program(&loaded, &[&scl]);
        config.set_out_pins(&[&sda]);
        config.set_set_pins(&[&sda]);
        config.set_in_pins(&[&sda, &scl]);
        config.set_jmp_pin(&sda);
        config.shift_out.direction = ShiftDirection::Left;
        config.shift_out.auto_fill = true;
        config.shift_out.threshold = 16;
        config.shift_in.direction = ShiftDirection::Left;
        config.shift_in.auto_fill = true;
        config.shift_in.threshold = 8;

        let sys = clk_sys_freq().to_fixed::<fixed::FixedU64<U8>>();
        let target = (frequency_hz * CYCLES_PER_I2C_BIT).to_fixed::<fixed::FixedU64<U8>>();
        config.clock_divider = (sys / target).to_fixed();

        sm.set_config(&config);
        // Keep output values low. PINDIRS selects between driving this low
        // value and releasing the line because OE is inverted above.
        sm.set_pins(Level::Low, &[&sda, &scl]);
        sm.set_pin_dirs(Direction::Out, &[&sda, &scl]);
        sm.set_enable(true);

        Self {
            sm,
            bus_state_instructions,
        }
    }

    /// Write one I2C transaction. The address is a 7-bit, unshifted address.
    pub fn blocking_write(&mut self, address: u8, bytes: &[u8]) {
        assert!(address < 0x80);

        self.start();
        self.write_byte(address << 1);
        for &byte in bytes {
            self.write_byte(byte);
        }
        self.stop();
    }

    fn start(&mut self) {
        // SDA falls while SCL is high, then SCL falls.
        self.exec(&[
            self.bus_state_instructions[2],
            self.bus_state_instructions[0],
        ]);
    }

    fn stop(&mut self) {
        // Ensure both are low, release SCL, then release SDA.
        self.exec(&[
            self.bus_state_instructions[0],
            self.bus_state_instructions[2],
            self.bus_state_instructions[3],
        ]);
    }

    fn write_byte(&mut self, byte: u8) {
        // Bit 0 releases SDA for the target's ACK/NACK. The PIO shifts the
        // data MSB-first from bits 8:1.
        let _ = self.sm.tx().stalled();
        self.push_record((u16::from(byte) << 1) | 1);

        // Every data record samples eight bits and autopushes one RX word.
        // Drain it immediately so a longer write cannot fill and stall RX.
        while self.sm.rx().empty() {}
        let _ = self.sm.rx().pull();
        self.wait_for_completion();
    }

    fn exec(&mut self, instructions: &[u16]) {
        // The command header plus at most three instructions fits in the four
        // word TX FIFO. Pausing prevents an interrupt between pushes from
        // leaving a stale TX-stall indication partway through the sequence.
        assert!(!instructions.is_empty() && instructions.len() <= 3);
        self.sm.set_enable(false);
        let _ = self.sm.tx().stalled();

        let instruction_count_minus_one = (instructions.len() - 1) as u16;
        self.push_record(instruction_count_minus_one << 10);
        for &instruction in instructions {
            self.push_record(instruction);
        }

        self.sm.set_enable(true);
        self.wait_for_completion();
    }

    fn push_record(&mut self, record: u16) {
        // The original C implementation uses a halfword FIFO store. RP2040's
        // narrow-write replication puts the record in both halfwords, which is
        // required because this program shifts the OSR left. Reproduce that
        // explicitly when using Embassy's u32 FIFO API.
        let word = u32::from(record) * 0x0001_0001;
        while !self.sm.tx().try_push(word) {}
    }

    fn wait_for_completion(&mut self) {
        // Wait until the state machine has consumed this record/sequence and
        // stalls fetching the next one.
        while !self.sm.tx().empty() {}
        while !self.sm.tx().stalled() {}
    }
}
