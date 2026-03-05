use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::Arc;
use std::thread;
use std::time::Duration;

use crate::shutdown;

/// Refill interval for the token bucket.
const REFILL_INTERVAL: Duration = Duration::from_millis(50);

/// Token-bucket rate limiter shared across all connection writer threads.
///
/// A background refill thread adds tokens every 50ms. Writer threads
/// atomically acquire tokens before each write.
pub struct RateLimiter {
    tokens: AtomicU64,
    rate: u64,
}

impl RateLimiter {
    /// Create a new rate limiter with the given total bytes/sec budget.
    pub fn new(rate: u64) -> Self {
        Self {
            tokens: AtomicU64::new(0),
            rate,
        }
    }

    /// Try to acquire `amount` tokens. Returns true if tokens were acquired,
    /// false if insufficient tokens are available.
    pub fn try_acquire(&self, amount: u64) -> bool {
        loop {
            let current = self.tokens.load(Ordering::Relaxed);
            if current < amount {
                return false;
            }
            match self.tokens.compare_exchange_weak(
                current,
                current - amount,
                Ordering::Relaxed,
                Ordering::Relaxed,
            ) {
                Ok(_) => return true,
                Err(_) => continue, // CAS failed, retry
            }
        }
    }

    /// Spawn a background thread that refills tokens at the configured rate.
    /// Returns a JoinHandle for the refill thread.
    pub fn start_refill_thread(self: &Arc<Self>) -> thread::JoinHandle<()> {
        let limiter = Arc::clone(self);
        let tokens_per_refill = (limiter.rate as f64 * REFILL_INTERVAL.as_secs_f64()).ceil() as u64;
        // Cap at 1 second of tokens — large enough for any single write
        // but prevents unbounded accumulation during idle periods.
        let max_tokens = limiter.rate;

        thread::Builder::new()
            .name("rate-refill".into())
            .spawn(move || {
                while !shutdown::is_shutdown() {
                    let current = limiter.tokens.load(Ordering::Relaxed);
                    let new = current.saturating_add(tokens_per_refill).min(max_tokens);
                    limiter.tokens.store(new, Ordering::Relaxed);
                    thread::sleep(REFILL_INTERVAL);
                }
            })
            .expect("failed to spawn refill thread")
    }
}
