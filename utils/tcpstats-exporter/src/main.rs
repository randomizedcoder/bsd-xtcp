mod cli;
mod collector;
mod metrics;

use std::sync::atomic::{AtomicU32, AtomicU64, Ordering};
use std::sync::Mutex;
use std::time::Instant;

static HTTP_REQUESTS_TOTAL: AtomicU64 = AtomicU64::new(0);
static LAST_LATENCY_BITS: AtomicU64 = AtomicU64::new(0);
static ACTIVE_REQUESTS: AtomicU32 = AtomicU32::new(0);

fn main() {
    let config = match cli::parse_args() {
        Ok(c) => c,
        Err(e) => {
            eprintln!("error: {e}");
            std::process::exit(1);
        }
    };

    let min_interval_secs = 1.0 / config.max_query_rate;

    eprintln!(
        "tcpstats-exporter listening on http://{}/metrics",
        config.listen_addr
    );
    eprintln!(
        "  max_concurrent={} max_query_rate={}/s",
        config.max_concurrent, config.max_query_rate
    );

    let server = match tiny_http::Server::http(&config.listen_addr) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("failed to bind {}: {e}", config.listen_addr);
            std::process::exit(1);
        }
    };

    let last_request_time: Mutex<Option<Instant>> = Mutex::new(None);

    for request in server.incoming_requests() {
        // Rate limiting
        {
            let mut last = last_request_time.lock().unwrap();
            let now = Instant::now();
            if let Some(prev) = *last {
                let elapsed = now.duration_since(prev).as_secs_f64();
                if elapsed < min_interval_secs {
                    let _ = request.respond(
                        tiny_http::Response::from_string("rate limit exceeded\n")
                            .with_status_code(429),
                    );
                    continue;
                }
            }
            *last = Some(now);
        }

        // Concurrency limiting
        let active = ACTIVE_REQUESTS.fetch_add(1, Ordering::SeqCst);
        if active >= config.max_concurrent {
            ACTIVE_REQUESTS.fetch_sub(1, Ordering::SeqCst);
            let _ = request.respond(
                tiny_http::Response::from_string("too many concurrent requests\n")
                    .with_status_code(429),
            );
            continue;
        }

        HTTP_REQUESTS_TOTAL.fetch_add(1, Ordering::SeqCst);

        let url = request.url().to_string();
        let method = request.method().to_string();

        let response = match (method.as_str(), url.as_str()) {
            ("GET", "/metrics") => handle_metrics(),
            ("GET", "/") => handle_root(),
            _ => tiny_http::Response::from_string("not found\n").with_status_code(404),
        };

        let _ = request.respond(response);

        ACTIVE_REQUESTS.fetch_sub(1, Ordering::SeqCst);
    }
}

fn handle_metrics() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    match collector::collect() {
        Ok(snapshot) => {
            LAST_LATENCY_BITS.store(snapshot.duration_secs.to_bits(), Ordering::SeqCst);

            let http_requests = HTTP_REQUESTS_TOTAL.load(Ordering::SeqCst);
            let latency = f64::from_bits(LAST_LATENCY_BITS.load(Ordering::SeqCst));

            let body = metrics::render(&snapshot, http_requests, latency);

            tiny_http::Response::from_data(body.into_bytes())
                .with_header(
                    "Content-Type: text/plain; version=0.0.4; charset=utf-8"
                        .parse::<tiny_http::Header>()
                        .unwrap(),
                )
                .with_status_code(200)
        }
        Err(e) => {
            let body = format!("collection failed: {e}\n");
            tiny_http::Response::from_data(body.into_bytes()).with_status_code(500)
        }
    }
}

fn handle_root() -> tiny_http::Response<std::io::Cursor<Vec<u8>>> {
    let body = "tcpstats-exporter\nMetrics at /metrics\n";
    tiny_http::Response::from_data(body.as_bytes().to_vec()).with_status_code(200)
}
