//! Unified input event queue (Phase 0.1 — see docs/PLAN.md).
//!
//! Keyboard (IRQ1) and mouse (IRQ12) interrupt handlers push [`InputEvent`]s here; consumers
//! (the GUI event loop, and later user programs) pull them with [`poll`]. This is *additive*:
//! the legacy raw-scancode buffer (`keyboard.rs`) and the mouse position/click APIs
//! (`mouse.rs`) still work for the existing text-mode shell/desktop, so nothing regresses.
//!
//! The ring buffer mirrors the lock-light pattern already used in `keyboard.rs` (atomic
//! head/tail + a short `spin::Mutex` for the storage), which is safe to touch from an
//! interrupt handler in this kernel.

use core::sync::atomic::{AtomicUsize, Ordering};
use spin::Mutex;

pub use crate::mouse::MouseButton;

/// A single input event. `Copy` so the ring buffer can be a plain array.
#[derive(Clone, Copy, Debug)]
pub enum InputEvent {
    /// A key transition. `scancode` is the raw Set-1 code; `pressed` is false on key release
    /// (raw code had bit 7 set). Decoding to characters is the consumer's job.
    Key { scancode: u8, pressed: bool },
    /// Relative mouse movement (raw PS/2 deltas; +dy means "up" per PS/2 convention).
    MouseMove { dx: i16, dy: i16 },
    /// A mouse button transition.
    MouseButton { button: MouseButton, pressed: bool },
    /// Scroll wheel delta (+ = up).
    MouseScroll { delta: i8 },
}

const QSIZE: usize = 256;

static QUEUE: Mutex<[InputEvent; QSIZE]> = Mutex::new([InputEvent::MouseScroll { delta: 0 }; QSIZE]);
static HEAD: AtomicUsize = AtomicUsize::new(0);
static TAIL: AtomicUsize = AtomicUsize::new(0);

/// Push an event (called from interrupt handlers). Drops the event if the queue is full.
pub fn push(ev: InputEvent) {
    let head = HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % QSIZE;
    if next == TAIL.load(Ordering::Acquire) {
        return; // full — drop, don't block an interrupt handler
    }
    QUEUE.lock()[head] = ev;
    HEAD.store(next, Ordering::Release);
}

/// Pull the next event, if any.
pub fn poll() -> Option<InputEvent> {
    let tail = TAIL.load(Ordering::Relaxed);
    if tail == HEAD.load(Ordering::Acquire) {
        return None;
    }
    let ev = QUEUE.lock()[tail];
    TAIL.store((tail + 1) % QSIZE, Ordering::Release);
    Some(ev)
}

/// True if at least one event is queued.
pub fn has_events() -> bool {
    TAIL.load(Ordering::Relaxed) != HEAD.load(Ordering::Acquire)
}
