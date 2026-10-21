use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::Log;

pub const POLL: Duration = Duration::from_secs(30);
pub const RETRY: Duration = Duration::from_secs(30);
pub const MINUTE: Duration = Duration::from_secs(60);
pub const TOAST: Duration = Duration::from_secs(4);
pub const FRAME: Duration = Duration::from_millis(100);
pub const SECOND: Duration = Duration::from_secs(1);

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Timer {
    Poll,
    Retry,
    RelativeTimes,
    Toast,
    Reload(Log),
}

#[derive(Default)]
pub struct Timers(HashMap<Timer, Instant>);

impl Timers {
    pub fn set(&mut self, timer: Timer, at: Instant) {
        self.0.insert(timer, at);
    }

    pub fn clear(&mut self, timer: Timer) {
        self.0.remove(&timer);
    }

    pub fn is_set(&self, timer: Timer) -> bool {
        self.0.contains_key(&timer)
    }

    pub fn remaining(&self, timer: Timer, now: Instant) -> Option<Duration> {
        self.0
            .get(&timer)
            .map(|at| at.saturating_duration_since(now))
    }

    pub fn due(&mut self, now: Instant) -> Vec<Timer> {
        let mut due: Vec<Timer> = self
            .0
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(timer, _)| *timer)
            .collect();
        due.sort_by_key(|timer| self.0[timer]);
        for timer in &due {
            self.0.remove(timer);
        }
        due
    }

    pub fn next(&self) -> Option<Instant> {
        self.0.values().min().copied()
    }
}
