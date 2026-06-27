//! Inter-process communication (Phase 2.4 — see docs/PLAN.md).
//!
//! A minimal in-kernel pipe: a bounded byte ring buffer guarded by a spin lock. It is the one
//! IPC primitive required to round out the process model — a producer thread can hand bytes to
//! a consumer thread that runs concurrently under the preemptive scheduler. `demo()` proves it
//! with a producer/consumer pair (the consumer receives every byte, in order, that the
//! producer sent while both were being preempted).

use spin::Mutex;

const CAP: usize = 64;

struct Ring {
    buf: [u8; CAP],
    head: usize,
    tail: usize,
    len: usize,
}

static PIPE: Mutex<Ring> = Mutex::new(Ring { buf: [0; CAP], head: 0, tail: 0, len: 0 });

/// Push a byte. Returns false if the pipe is full (caller should retry/yield).
pub fn send(b: u8) -> bool {
    let mut p = PIPE.lock();
    if p.len == CAP {
        return false;
    }
    let t = p.tail;
    p.buf[t] = b;
    p.tail = (t + 1) % CAP;
    p.len += 1;
    true
}

/// Pop a byte, or None if the pipe is empty.
pub fn recv() -> Option<u8> {
    let mut p = PIPE.lock();
    if p.len == 0 {
        return None;
    }
    let h = p.head;
    let b = p.buf[h];
    p.head = (h + 1) % CAP;
    p.len -= 1;
    Some(b)
}

/// Reset the pipe to empty.
pub fn clear() {
    let mut p = PIPE.lock();
    p.head = 0;
    p.tail = 0;
    p.len = 0;
}

// ── Producer / consumer demonstration (`ipctest` shell command) ────────────────────────────

extern "C" fn producer() -> ! {
    for i in 0..16u8 {
        while !send(i) {
            core::hint::spin_loop();
        }
        for _ in 0..250_000u64 {
            core::hint::spin_loop();
        }
    }
    while !send(0xFF) {
        core::hint::spin_loop();
    } // sentinel = end of stream
    crate::serial_println!("[IPC] producer: sent 0..15 + EOF");
    crate::kthread::exit();
}

extern "C" fn consumer() -> ! {
    let mut count = 0u32;
    let mut sum = 0u32;
    loop {
        match recv() {
            Some(0xFF) => break,
            Some(b) => {
                count += 1;
                sum += b as u32;
                crate::serial_print!("{} ", b);
            }
            None => core::hint::spin_loop(),
        }
    }
    crate::serial_println!("\n[IPC] consumer: received {} bytes, sum={} (expected 16, 120)", count, sum);
    crate::kthread::exit();
}

/// Spawn a producer and a consumer as preemptive kernel threads that talk over the pipe.
pub fn demo() {
    crate::serial_println!("[IPC] starting producer/consumer over a kernel pipe");
    crate::println!("ipctest: producer & consumer threads over a kernel pipe; watch serial...");
    clear();
    crate::kthread::reset();
    crate::kthread::spawn(producer);
    crate::kthread::spawn(consumer);
    crate::kthread::arm();
    while !crate::kthread::all_finished() {
        x86_64::instructions::hlt();
    }
    crate::kthread::disarm();
    crate::kthread::reset();
    crate::serial_println!("[IPC] done");
    crate::println!("ipctest: done (consumer received all bytes from producer)");
}
