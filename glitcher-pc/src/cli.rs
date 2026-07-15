use std::path::PathBuf;

use clap::{Parser, Subcommand};
use clap_complete::Shell;

#[derive(Debug, Parser)]
#[command(name = "glitcher", about = "Control the glitcher")]
pub struct Cli {
    /// Serial port connected to the controller.
    #[arg(long, global = true, default_value = "/dev/ttyACM0")]
    pub port: PathBuf,

    #[command(subcommand)]
    pub command: Command,
}

#[derive(Debug, Subcommand)]
pub enum Command {
    /// Send a ping to the controller.
    Ping,

    /// Check that the Pico firmware matches this application's version.
    CheckVersion,

    /// Reboot the Pico controller.
    Reboot,

    /// Count chip-select falling edges, default 1s.
    CountChipSelects {
        #[arg(default_value_t = 1)]
        timeout_s: u32,
    },

    /// Capture a single SPI0 transaction (up to 16 KiB).
    TapSpi {
        /// Number of bytes to capture (1 through 16384).
        #[arg(long, default_value_t = 32, value_parser = clap::value_parser!(u16).range(1..=16384))]
        byte_count: u16,

        /// Seconds to wait for the transaction.
        #[arg(long, default_value_t = 1)]
        timeout_s: u32,
    },

    /// Write shell completions to standard output.
    GenerateCompletions {
        /// Shell to generate completions for.
        #[arg(value_enum, default_value_t = Shell::Bash)]
        shell: Shell,
    },
}
