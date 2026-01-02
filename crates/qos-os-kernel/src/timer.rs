//! Timer utilities for QOS
//!
//! High-level timing functions built on PIT/RTC.

use core::sync::atomic::{AtomicU64, Ordering};

/// Tick counter (incremented by PIT interrupt)
static TICKS: AtomicU64 = AtomicU64::new(0);

/// PIT frequency (set by pit::init_timer)
static PIT_FREQ_HZ: AtomicU64 = AtomicU64::new(100);

/// Record a tick (called from PIT interrupt handler)
pub fn tick() {
    TICKS.fetch_add(1, Ordering::Relaxed);
}

/// Get current tick count
pub fn ticks() -> u64 {
    TICKS.load(Ordering::Relaxed)
}

/// Set PIT frequency (called during init)
pub fn set_frequency(hz: u64) {
    PIT_FREQ_HZ.store(hz, Ordering::SeqCst);
}

/// Get uptime in milliseconds
pub fn uptime_ms() -> u64 {
    let t = TICKS.load(Ordering::Relaxed);
    let freq = PIT_FREQ_HZ.load(Ordering::Relaxed);
    if freq == 0 { return 0; }
    t * 1000 / freq
}

/// Get uptime in seconds
pub fn uptime_secs() -> u64 {
    let t = TICKS.load(Ordering::Relaxed);
    let freq = PIT_FREQ_HZ.load(Ordering::Relaxed);
    if freq == 0 { return 0; }
    t / freq
}

/// Sleep for approximately the given number of milliseconds
/// Note: This is a busy-wait, not ideal but works for bare metal
pub fn sleep_ms(ms: u32) {
    let start = ticks();
    let freq = PIT_FREQ_HZ.load(Ordering::Relaxed);
    let wait_ticks = (ms as u64 * freq) / 1000;
    
    while ticks() < start + wait_ticks {
        // Hint to CPU we're spinning
        core::hint::spin_loop();
        // Enable interrupts briefly to allow timer to fire
        crate::arch::enable_interrupts();
        crate::arch::hlt();
        crate::arch::disable_interrupts();
    }
}

/// Sleep for approximately the given number of seconds
pub fn sleep_secs(secs: u32) {
    sleep_ms(secs * 1000);
}

/// Delay for very short periods (microseconds, approximate)
pub fn delay_us(us: u32) {
    // Very rough approximation using CPU cycles
    // Assumes ~1GHz CPU = 1000 cycles per microsecond
    let cycles = us as u64 * 1000;
    for _ in 0..cycles {
        core::hint::spin_loop();
    }
}

/// Simple stopwatch for measuring elapsed time
pub struct Stopwatch {
    start_ticks: u64,
}

impl Stopwatch {
    /// Create and start a new stopwatch
    pub fn start() -> Self {
        Self {
            start_ticks: ticks(),
        }
    }
    
    /// Get elapsed time in milliseconds
    pub fn elapsed_ms(&self) -> u64 {
        let elapsed = ticks() - self.start_ticks;
        let freq = PIT_FREQ_HZ.load(Ordering::Relaxed);
        if freq == 0 { return 0; }
        elapsed * 1000 / freq
    }
    
    /// Get elapsed time in seconds
    pub fn elapsed_secs(&self) -> u64 {
        self.elapsed_ms() / 1000
    }
    
    /// Reset the stopwatch
    pub fn reset(&mut self) {
        self.start_ticks = ticks();
    }
}

/// Format uptime as human-readable string
pub fn uptime_string() -> alloc::string::String {
    let total_secs = uptime_secs();
    let hours = total_secs / 3600;
    let minutes = (total_secs % 3600) / 60;
    let seconds = total_secs % 60;
    
    if hours > 0 {
        alloc::format!("{}h {}m {}s", hours, minutes, seconds)
    } else if minutes > 0 {
        alloc::format!("{}m {}s", minutes, seconds)
    } else {
        alloc::format!("{}s", seconds)
    }
}
