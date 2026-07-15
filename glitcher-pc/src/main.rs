use anyhow::Result;
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use glitcher_rpc::Host2ControllerMessage;
use tracing::info;
use tracing_subscriber::EnvFilter;

mod cli;
mod console;

use cli::{Cli, Command};

fn main() -> Result<()> {
    let cli = Cli::parse();

    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    match cli.command {
        Command::Ping => {
            let ping = Host2ControllerMessage::Ping;
            info!(port = %cli.port.display(), message = ?ping, "Sending message");
            console::send(&cli.port, &ping)?;
            println!("Pong")
        }
        Command::GenerateCompletions { shell } => {
            generate(
                shell,
                &mut Cli::command(),
                "glitcher",
                &mut std::io::stdout(),
            );
        }
    }

    Ok(())
}
