use alloc::{boxed::Box, vec::Vec};

use pc_keyboard::{layouts, DecodedKey, HandleControl, Keyboard, ScancodeSet1};

use crate::{interrupts, keyboard};

pub trait Task {
    fn step(&mut self);
}

pub struct Scheduler {
    tasks: Vec<Box<dyn Task>>,
}

impl Scheduler {
    pub fn new() -> Self {
        Self { tasks: Vec::new() }
    }

    pub fn add_task<T: Task + 'static>(&mut self, task: T) {
        self.tasks.push(Box::new(task));
    }

    pub fn step(&mut self) {
        static STEP_COUNT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(0);
        let count = STEP_COUNT.fetch_add(1, core::sync::atomic::Ordering::Relaxed);
        if count == 0 {
            crate::serial::println!("[SCHEDULER] First step, {} tasks", self.tasks.len());
        }
        for (i, task) in self.tasks.iter_mut().enumerate() {
            if count == 0 {
                crate::serial::println!("[SCHEDULER] Calling task {}", i);
            }
            task.step();
        }
    }
}

pub struct HeartbeatTask {
    last_tick: u64,
    interval_ticks: u64,
}

impl HeartbeatTask {
    pub fn new(interval_ticks: u64) -> Self {
        Self {
            last_tick: 0,
            interval_ticks,
        }
    }
}

impl Task for HeartbeatTask {
    fn step(&mut self) {
        let tick = interrupts::TICKS.load(core::sync::atomic::Ordering::Relaxed);
        if tick == self.last_tick {
            return;
        }
        self.last_tick = tick;

        if self.interval_ticks != 0 && tick % self.interval_ticks == 0 {
            crate::serial_print!(".");
        }
    }
}

pub struct KeyboardEchoTask {
    kb: Keyboard<layouts::Us104Key, ScancodeSet1>,
}

impl KeyboardEchoTask {
    pub fn new() -> Self {
        let kb = Keyboard::new(ScancodeSet1::new(), layouts::Us104Key, HandleControl::Ignore);
        Self { kb }
    }
}

impl Task for KeyboardEchoTask {
    fn step(&mut self) {
        // Limit work per tick so a burst of scancodes doesn't starve other tasks.
        for _ in 0..16 {
            let Some(sc) = keyboard::pop_scancode() else {
                return;
            };

            if let Ok(Some(event)) = self.kb.add_byte(sc) {
                if let Some(key) = self.kb.process_keyevent(event) {
                    match key {
                        DecodedKey::Unicode(c) => {
                            crate::serial_print!("{}", c);
                            crate::print!("{}", c);
                        }
                        DecodedKey::RawKey(k) => {
                            crate::serial_println!("\n[{:?}]", k);
                        }
                    }
                }
            }
        }
    }
}
