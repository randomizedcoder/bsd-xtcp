use std::sync::atomic::{AtomicBool, Ordering};

/// Global shutdown flag, set by signal handlers.
static SHUTDOWN: AtomicBool = AtomicBool::new(false);

/// Returns true if shutdown has been requested.
pub fn is_shutdown() -> bool {
    SHUTDOWN.load(Ordering::Relaxed)
}

/// Request shutdown (callable from any thread).
pub fn request_shutdown() {
    SHUTDOWN.store(true, Ordering::Relaxed);
}

/// Install SIGINT and SIGTERM handlers that set the shutdown flag.
///
/// Uses `libc::sigaction` for portable signal handling. The handler is
/// async-signal-safe because it only writes to an AtomicBool.
pub fn install_signal_handlers() {
    unsafe {
        let mut sa: libc::sigaction = std::mem::zeroed();
        sa.sa_sigaction = signal_handler as *const () as usize;
        sa.sa_flags = libc::SA_RESTART;
        libc::sigemptyset(&mut sa.sa_mask);

        libc::sigaction(libc::SIGINT, &sa, std::ptr::null_mut());
        libc::sigaction(libc::SIGTERM, &sa, std::ptr::null_mut());
    }
}

extern "C" fn signal_handler(_sig: libc::c_int) {
    SHUTDOWN.store(true, Ordering::Relaxed);
}
