use std::time::Duration;

/// Runtime configuration parsed from command-line arguments.
pub struct Config {
    pub interval: Duration,
    pub count: u64,
    pub pretty: bool,
}

impl Default for Config {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(1),
            count: 0, // 0 = infinite
            pretty: false,
        }
    }
}

impl Config {
    /// Parse configuration from command-line arguments.
    ///
    /// Supported flags:
    ///   --interval SECS   Collection interval in seconds (default: 1)
    ///   --count N          Number of collection passes, 0 = infinite (default: 0)
    ///   --pretty           Pretty-print JSON output
    ///   --help             Show usage and exit
    pub fn from_args() -> Result<Self, String> {
        let mut config = Config::default();
        let args: Vec<String> = std::env::args().skip(1).collect();
        let mut i = 0;

        while i < args.len() {
            match args[i].as_str() {
                "--interval" => {
                    i += 1;
                    let secs: f64 = args
                        .get(i)
                        .ok_or("--interval requires a value")?
                        .parse()
                        .map_err(|e| format!("invalid --interval value: {e}"))?;
                    if secs <= 0.0 {
                        return Err("--interval must be positive".into());
                    }
                    config.interval = Duration::from_secs_f64(secs);
                }
                "--count" => {
                    i += 1;
                    config.count = args
                        .get(i)
                        .ok_or("--count requires a value")?
                        .parse()
                        .map_err(|e| format!("invalid --count value: {e}"))?;
                }
                "--pretty" => {
                    config.pretty = true;
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

        Ok(config)
    }
}

fn print_usage() {
    eprintln!(
        "Usage: bsd-xtcp [OPTIONS]

Collect TCP socket statistics from the kernel and output JSON Lines to stdout.

Options:
  --interval SECS   Collection interval in seconds (default: 1)
  --count N         Number of collection passes, 0 = infinite (default: 0)
  --pretty          Pretty-print JSON output
  --help, -h        Show this help message"
    );
}
