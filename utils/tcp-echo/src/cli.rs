use std::net::SocketAddr;
use std::time::Duration;

/// Maximum number of ports allowed.
const MAX_PORTS: usize = 10;

/// Maximum number of connections allowed.
const MAX_CONNECTIONS: u32 = 1000;

/// Server mode configuration.
pub struct ServerConfig {
    pub ports: Vec<u16>,
    pub bind_addr: String,
    pub report_interval: Duration,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            ports: Vec::new(),
            bind_addr: "0.0.0.0".to_string(),
            report_interval: Duration::from_secs(10),
        }
    }
}

/// Client mode configuration.
pub struct ClientConfig {
    pub host: String,
    pub ports: Vec<u16>,
    pub connections: u32,
    pub rate: u64,
    pub ramp_duration: Duration,
    pub report_interval: Duration,
    pub duration: Duration,
    pub payload_size: usize,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            host: "127.0.0.1".to_string(),
            ports: Vec::new(),
            connections: 10,
            rate: 1024,
            ramp_duration: Duration::from_secs(10),
            report_interval: Duration::from_secs(10),
            duration: Duration::ZERO, // 0 = infinite
            payload_size: 1024,
        }
    }
}

impl ServerConfig {
    /// Return bind addresses for all configured ports.
    pub fn bind_addrs(&self) -> Vec<SocketAddr> {
        self.ports
            .iter()
            .map(|p| {
                format!("{}:{}", self.bind_addr, p)
                    .parse()
                    .expect("invalid bind address")
            })
            .collect()
    }
}

/// Parsed subcommand.
pub enum Command {
    Server(ServerConfig),
    Client(ClientConfig),
}

/// Parse command-line arguments into a Command.
pub fn parse_args() -> Result<Command, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();

    if args.is_empty() {
        print_main_usage();
        std::process::exit(0);
    }

    match args[0].as_str() {
        "server" => parse_server_args(&args[1..]),
        "client" => parse_client_args(&args[1..]),
        "--help" | "-h" => {
            print_main_usage();
            std::process::exit(0);
        }
        other => Err(format!("unknown subcommand: {other}")),
    }
}

fn parse_server_args(args: &[String]) -> Result<Command, String> {
    let mut config = ServerConfig::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--ports" => {
                i += 1;
                let val = args.get(i).ok_or("--ports requires a value")?;
                config.ports = parse_ports(val)?;
            }
            "--bind" => {
                i += 1;
                config.bind_addr = args.get(i).ok_or("--bind requires a value")?.clone();
            }
            "--report-interval" => {
                i += 1;
                let secs: u64 = args
                    .get(i)
                    .ok_or("--report-interval requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --report-interval value: {e}"))?;
                if secs == 0 {
                    return Err("--report-interval must be positive".into());
                }
                config.report_interval = Duration::from_secs(secs);
            }
            "--help" | "-h" => {
                print_server_usage();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown server argument: {other}"));
            }
        }
        i += 1;
    }

    if config.ports.is_empty() {
        return Err("--ports is required".into());
    }

    Ok(Command::Server(config))
}

fn parse_client_args(args: &[String]) -> Result<Command, String> {
    let mut config = ClientConfig::default();
    let mut i = 0;

    while i < args.len() {
        match args[i].as_str() {
            "--host" => {
                i += 1;
                config.host = args.get(i).ok_or("--host requires a value")?.clone();
            }
            "--ports" => {
                i += 1;
                let val = args.get(i).ok_or("--ports requires a value")?;
                config.ports = parse_ports(val)?;
            }
            "--connections" => {
                i += 1;
                let n: u32 = args
                    .get(i)
                    .ok_or("--connections requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --connections value: {e}"))?;
                if n == 0 {
                    return Err("--connections must be positive".into());
                }
                if n > MAX_CONNECTIONS {
                    return Err(format!("--connections max is {MAX_CONNECTIONS}"));
                }
                config.connections = n;
            }
            "--rate" => {
                i += 1;
                let r: u64 = args
                    .get(i)
                    .ok_or("--rate requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --rate value: {e}"))?;
                if r == 0 {
                    return Err("--rate must be positive".into());
                }
                config.rate = r;
            }
            "--ramp-duration" => {
                i += 1;
                let secs: u64 = args
                    .get(i)
                    .ok_or("--ramp-duration requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --ramp-duration value: {e}"))?;
                config.ramp_duration = Duration::from_secs(secs);
            }
            "--report-interval" => {
                i += 1;
                let secs: u64 = args
                    .get(i)
                    .ok_or("--report-interval requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --report-interval value: {e}"))?;
                if secs == 0 {
                    return Err("--report-interval must be positive".into());
                }
                config.report_interval = Duration::from_secs(secs);
            }
            "--duration" => {
                i += 1;
                let secs: u64 = args
                    .get(i)
                    .ok_or("--duration requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --duration value: {e}"))?;
                config.duration = Duration::from_secs(secs);
            }
            "--payload-size" => {
                i += 1;
                let sz: usize = args
                    .get(i)
                    .ok_or("--payload-size requires a value")?
                    .parse()
                    .map_err(|e| format!("invalid --payload-size value: {e}"))?;
                if sz == 0 {
                    return Err("--payload-size must be positive".into());
                }
                config.payload_size = sz;
            }
            "--help" | "-h" => {
                print_client_usage();
                std::process::exit(0);
            }
            other => {
                return Err(format!("unknown client argument: {other}"));
            }
        }
        i += 1;
    }

    if config.ports.is_empty() {
        return Err("--ports is required".into());
    }

    Ok(Command::Client(config))
}

/// Parse a comma-separated list of port numbers.
fn parse_ports(s: &str) -> Result<Vec<u16>, String> {
    let ports: Vec<u16> = s
        .split(',')
        .map(|p| {
            p.trim()
                .parse::<u16>()
                .map_err(|e| format!("invalid port '{p}': {e}"))
        })
        .collect::<Result<Vec<_>, _>>()?;

    if ports.is_empty() {
        return Err("at least one port is required".into());
    }
    if ports.len() > MAX_PORTS {
        return Err(format!("max {MAX_PORTS} ports allowed"));
    }
    for &p in &ports {
        if p == 0 {
            return Err("port 0 is not allowed".into());
        }
    }

    Ok(ports)
}

fn print_main_usage() {
    eprintln!(
        "Usage: tcp-echo <COMMAND> [OPTIONS]

TCP echo server+client for testing bsd-xtcp socket stats collection.

Commands:
  server    Start echo server on one or more ports
  client    Connect to echo server and generate traffic

Options:
  --help, -h    Show this help message

Run 'tcp-echo <COMMAND> --help' for subcommand options."
    );
}

fn print_server_usage() {
    eprintln!(
        "Usage: tcp-echo server [OPTIONS]

Start a TCP echo server that accepts connections and echoes data back.

Options:
  --ports PORTS          Comma-separated ports to listen on (required, max 10)
  --bind ADDR            Bind address (default: 0.0.0.0)
  --report-interval SECS Reporting interval in seconds (default: 10)
  --help, -h             Show this help message"
    );
}

fn print_client_usage() {
    eprintln!(
        "Usage: tcp-echo client [OPTIONS]

Connect to a TCP echo server and generate controlled traffic.

Options:
  --host HOST              Target host (default: 127.0.0.1)
  --ports PORTS            Comma-separated server ports (required, max 10)
  --connections N          Total TCP connections to open (default: 10, max: 1000)
  --rate BYTES             Total bytes/sec across all connections (default: 1024)
  --ramp-duration SECS     Duration to ramp up connections (default: 10)
  --report-interval SECS   Stats reporting interval in seconds (default: 10)
  --duration SECS          Total runtime, 0 = infinite (default: 0)
  --payload-size BYTES     Size of each write call (default: 1024)
  --help, -h               Show this help message"
    );
}
