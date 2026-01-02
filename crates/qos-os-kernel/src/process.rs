use core::sync::atomic::{AtomicU64, Ordering};

use spin::Mutex;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ProcState {
    None,
    Running,
    Exited,
}

#[derive(Clone, Copy, Debug)]
pub struct ProcInfo {
    pub pid: u64,
    pub state: ProcState,
    pub exit_code: u64,
    pub image_hash: u64,
}

static NEXT_PID: AtomicU64 = AtomicU64::new(1);
static CURRENT: Mutex<ProcInfo> = Mutex::new(ProcInfo {
    pid: 0,
    state: ProcState::None,
    exit_code: 0,
    image_hash: 0,
});

pub fn begin_foreground(image_tag: &[u8]) -> u64 {
    let pid = NEXT_PID.fetch_add(1, Ordering::Relaxed);
    let mut cur = CURRENT.lock();
    cur.pid = pid;
    cur.state = ProcState::Running;
    cur.exit_code = 0;
    cur.image_hash = fnv1a64(image_tag);
    pid
}

pub fn exit_foreground(exit_code: u64) {
    let mut cur = CURRENT.lock();
    cur.exit_code = exit_code;
    cur.state = ProcState::Exited;
}

pub fn current() -> ProcInfo {
    *CURRENT.lock()
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf29ce484222325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x100000001b3);
    }
    h
}
