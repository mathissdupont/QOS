use core::sync::atomic::{AtomicUsize, Ordering};

use spin::Mutex;

const BUF_SIZE: usize = 256;

static SCANCODE_BUF: Mutex<[u8; BUF_SIZE]> = Mutex::new([0; BUF_SIZE]);
static SCANCODE_HEAD: AtomicUsize = AtomicUsize::new(0);
static SCANCODE_TAIL: AtomicUsize = AtomicUsize::new(0);

pub fn push_scancode(scancode: u8) {
    let head = SCANCODE_HEAD.load(Ordering::Relaxed);
    let next = (head + 1) % BUF_SIZE;
    let tail = SCANCODE_TAIL.load(Ordering::Acquire);

    if next == tail {
        return;
    }

    SCANCODE_BUF.lock()[head] = scancode;
    SCANCODE_HEAD.store(next, Ordering::Release);
}

pub fn pop_scancode() -> Option<u8> {
    let tail = SCANCODE_TAIL.load(Ordering::Relaxed);
    let head = SCANCODE_HEAD.load(Ordering::Acquire);

    if tail == head {
        return None;
    }

    let scancode = SCANCODE_BUF.lock()[tail];
    SCANCODE_TAIL.store((tail + 1) % BUF_SIZE, Ordering::Release);
    Some(scancode)
}
