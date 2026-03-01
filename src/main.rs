use anyhow::{Context, Result};
use bsd_xtcp::config::Config;
use bsd_xtcp::convert;
use bsd_xtcp::output::json::JsonSink;
use bsd_xtcp::output::OutputSink;
use bsd_xtcp::platform;

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
