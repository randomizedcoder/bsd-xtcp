mod cli;
mod client;
mod rate;
mod server;
mod shutdown;
mod stats;

use anyhow::Result;

fn main() -> Result<()> {
    shutdown::install_signal_handlers();

    let command = cli::parse_args().map_err(|e| anyhow::anyhow!("{e}"))?;

    match command {
        cli::Command::Server(config) => server::run(config),
        cli::Command::Client(config) => client::run(config),
    }
}
