use std::collections::VecDeque;
use uuid::Uuid;

pub trait Scheduler: Send + Sync {
    fn enqueue(&mut self, job_id: Uuid);
    fn select_next(&mut self) -> Option<Uuid>;
    fn len(&self) -> usize;
}

pub struct FifoScheduler {
    q: VecDeque<Uuid>,
}

impl FifoScheduler {
    pub fn new() -> Self {
        Self { q: VecDeque::new() }
    }
}

impl Scheduler for FifoScheduler {
    fn enqueue(&mut self, job_id: Uuid) {
        self.q.push_back(job_id);
    }

    fn select_next(&mut self) -> Option<Uuid> {
        self.q.pop_front()
    }

    fn len(&self) -> usize {
        self.q.len()
    }
}
