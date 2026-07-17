use anyhow::{Result, bail};
use clap::{CommandFactory, Parser};
use clap_complete::generate;
use glitcher_rpc::{Controller2HostMessage, FirmwareVersion, Host2ControllerMessage};
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
        Command::CheckVersion => {
            let desktop_version = desktop_version();
            let response = console::send(&cli.port, &Host2ControllerMessage::GetVersion)?;
            let Controller2HostMessage::Version(firmware_version) = response else {
                bail!("Pico returned an unexpected response to a version request");
            };

            println!("Desktop application version: {desktop_version}");
            println!("Pico firmware version: {firmware_version}");

            if desktop_version != firmware_version {
                eprintln!("\x1b[31mFAIL: desktop and Pico firmware versions do not match\x1b[0m");
                bail!("version mismatch");
            }
        }
        Command::Reboot => {
            let response = console::send(&cli.port, &Host2ControllerMessage::Reboot)?;
            if !matches!(response, Controller2HostMessage::Rebooting) {
                bail!("Pico returned an unexpected response to a reboot request");
            }
            println!("Pico is rebooting");
        }
        Command::CountChipSelects { timeout_s } => {
            let response = console::send(
                &cli.port,
                &Host2ControllerMessage::CountChipSelects { timeout_s },
            )?;
            let Controller2HostMessage::ChipSelectCount(count) = response else {
                bail!("Pico returned an unexpected response to a chip-select count request");
            };

            println!("Chip-select falling edges: {count}");
        }
        Command::TapSpi {
            byte_count,
            timeout_s,
        } => {
            let capture = console::tap_spi(&cli.port, byte_count, timeout_s)?;
            if capture.timed_out {
                eprintln!("SPI tap timed out after {timeout_s} seconds; returning partial capture");
            }
            println!("SPI RX: {:02x?}", capture.data);
            if capture.timed_out {
                bail!("SPI tap timed out after {timeout_s} seconds; Partial capture!");
            }
        }
        Command::SetVid { vid } => {
            let response = console::send(&cli.port, &Host2ControllerMessage::SetVid { vid })?;
            match response {
                Controller2HostMessage::VidSet => {
                    if let Some(vid) = vid {
                        println!("Applied VID {vid} to VSoc and VCore");
                    } else {
                        println!("Applied the default SVI2 VSoc and VCore VIDs");
                    }
                }
                Controller2HostMessage::Svi2Error(error) => {
                    bail!("SVI2 voltage command failed: {error}");
                }
                _ => bail!("Pico returned an unexpected response to an SVI2 voltage request"),
            }
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

fn desktop_version() -> FirmwareVersion {
    FirmwareVersion {
        major: env!("CARGO_PKG_VERSION_MAJOR").parse().unwrap(),
        minor: env!("CARGO_PKG_VERSION_MINOR").parse().unwrap(),
        patch: env!("CARGO_PKG_VERSION_PATCH").parse().unwrap(),
    }
}
