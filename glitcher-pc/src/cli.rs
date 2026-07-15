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

    /// Write shell completions to standard output.
    GenerateCompletions {
        /// Shell to generate completions for.
        #[arg(value_enum, default_value_t = Shell::Bash)]
        shell: Shell,
    },
}
