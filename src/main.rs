use anyhow::{Context, Result};
use tcpstats_reader::config::Config;
use tcpstats_reader::convert;
use tcpstats_reader::output::json::JsonSink;
use tcpstats_reader::output::OutputSink;
use tcpstats_reader::platform;

fn main() -> Result<()> {
    let config = Config::from_args().map_err(|e| anyhow::anyhow!("{e}"))?;

    let stdout = std::io::stdout().lock();
    let mut sink = JsonSink::new(stdout, config.pretty);
    let mut sequence: u64 = 0;
    let interval_ms = config.interval.as_millis() as u32;

    loop {
        sequence += 1;

        let result = platform::collect_tcp_sockets().context("failed to collect TCP sockets")?;

        let batch = convert::build_batch(
            &result.records,
            result.generation,
            result.collection_duration_ns,
            sequence,
            interval_ms,
        );

        sink.emit(&batch).context("failed to write output")?;
        sink.flush().context("failed to flush output")?;

        if config.count > 0 && sequence >= config.count {
            break;
        }

        std::thread::sleep(config.interval);
    }

    Ok(())
}
