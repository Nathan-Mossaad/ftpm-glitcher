use glitcher_rpc::Host2ControllerMessage;
use tracing::info;
use tracing_subscriber::EnvFilter;

fn main() {
    let filter = EnvFilter::try_from_env("RUST_LOG").unwrap_or_else(|_| EnvFilter::new("info"));
    tracing_subscriber::fmt().with_env_filter(filter).init();

    let ping = Host2ControllerMessage::Ping;
    info!("Message: {:?}", ping)
}
