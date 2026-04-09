/// Exporter configuration.
pub struct Config {
    pub listen_addr: String,
    pub max_concurrent: u32,
    pub max_query_rate: f64,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            listen_addr: "127.0.0.1:9814".to_string(),
            max_concurrent: 2,
            max_query_rate: 2.0,
        }
    }
}

/// Parse command-line arguments and environment variables into a Config.
pub fn parse_args() -> Result<Config, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut config = Config::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--listen" => {
                i += 1;
                config.listen_addr = args.get(i).ok_or("--listen requires a value")?.clone();
            }
            "--help" | "-h" => {
                print_usage();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown argument: {other}"));
            }
        }
        i += 1;
    }

    // Environment variable override for rate limit.
    if let Ok(val) = std::env::var("TCPSTATS_MAX_QUERY_RATE") {
        config.max_query_rate = val
            .parse()
            .map_err(|e| format!("invalid TCPSTATS_MAX_QUERY_RATE value: {e}"))?;
        if config.max_query_rate <= 0.0 {
            return Err("TCPSTATS_MAX_QUERY_RATE must be positive".into());
        }
    }

    Ok(config)
}

fn print_usage() {
    eprintln!(
        "Usage: tcpstats-exporter [OPTIONS]

Prometheus exporter for tcpstats kernel module statistics.

Serves metrics at http://<listen>/metrics in Prometheus text exposition format.

Options:
  --listen ADDR:PORT   Listen address (default: 127.0.0.1:9814)
  --help, -h           Show this help message

Environment:
  TCPSTATS_MAX_QUERY_RATE   Max requests per second (default: 2.0)"
    );
}
