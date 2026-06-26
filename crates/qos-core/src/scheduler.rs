//! Scheduler strategy (the "run queue").
//!
//! A trait so different policies can be plugged in later (priority, deadline, qubit-aware
//! placement per ADR-0006) without touching the manager.

use alloc::collections::VecDeque;
use qos_abi::JobHandle;

pub trait Scheduler: Send + Sync {
    fn enqueue(&mut self, job: JobHandle);
    fn select_next(&mut self) -> Option<JobHandle>;
    fn len(&self) -> usize;
    fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

/// First-in-first-out scheduler.
#[derive(Default)]
pub struct FifoScheduler {
    q: VecDeque<JobHandle>,
}

impl FifoScheduler {
    pub fn new() -> Self {
        Self { q: VecDeque::new() }
    }
}

impl Scheduler for FifoScheduler {
    fn enqueue(&mut self, job: JobHandle) {
        self.q.push_back(job);
    }

    fn select_next(&mut self) -> Option<JobHandle> {
        self.q.pop_front()
    }

    fn len(&self) -> usize {
        self.q.len()
    }
}
