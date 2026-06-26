//! Time abstraction.
//!
//! The core runtime is platform-agnostic, so it does not assume a wall clock. Each
//! embodiment supplies a [`Clock`]: the kernel wires this to the RTC/PIT, the host daemon
//! to the system clock ([`StdClock`]). Tests and the bare-metal default can use
//! [`ZeroClock`].

pub trait Clock: Send + Sync {
    /// Monotonic-ish timestamp in microseconds. Only differences are meaningful.
    fn now_micros(&self) -> u64;
}

/// A clock that always returns 0. Useful as a default and in tests where timestamps do not
/// matter.
#[derive(Debug, Default, Clone, Copy)]
pub struct ZeroClock;

impl Clock for ZeroClock {
    fn now_micros(&self) -> u64 {
        0
    }
}

/// System-clock-backed implementation, available with the `std` feature.
#[cfg(feature = "std")]
#[derive(Debug, Default, Clone, Copy)]
pub struct StdClock;

#[cfg(feature = "std")]
impl Clock for StdClock {
    fn now_micros(&self) -> u64 {
        std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map(|d| d.as_micros() as u64)
            .unwrap_or(0)
    }
}
