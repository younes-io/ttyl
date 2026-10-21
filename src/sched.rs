use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::api::Call;
use crate::model::Log;

/// The server's signed throttle: this many requests in any sliding window.
pub const LIMIT: usize = 120;
pub const WINDOW: Duration = Duration::from_secs(60);
pub const RESERVE: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    Now,
    Later,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKey {
    Me,
    Redeem,
    Page(Log),
    Older(Log),
}

impl Call {
    pub fn key(&self) -> CallKey {
        match self {
            Call::Me => CallKey::Me,
            Call::Redeem { .. } => CallKey::Redeem,
            Call::Page(log) => CallKey::Page(*log),
            Call::Older { log, .. } => CallKey::Older(*log),
        }
    }
}

pub struct Scheduler {
    now: VecDeque<Call>,
    later: VecDeque<Call>,
    flying: HashMap<Ticket, (Call, Priority)>,
    sent: VecDeque<Instant>,
    hold: Option<Instant>,
    next_ticket: u64,
    workers: usize,
}

impl Scheduler {
    pub fn new(workers: usize) -> Self {
        Scheduler {
            now: VecDeque::new(),
            later: VecDeque::new(),
            flying: HashMap::new(),
            sent: VecDeque::new(),
            hold: None,
            next_ticket: 0,
            workers,
        }
    }

    pub fn send(&mut self, call: Call, priority: Priority) {
        let key = call.key();
        if self.in_flight(key) {
            return;
        }
        if let Some(queued) = self.now.iter_mut().find(|queued| queued.key() == key) {
            *queued = call;
            return;
        }
        self.later.retain(|queued| queued.key() != key);
        match priority {
            Priority::Now => self.now.push_back(call),
            Priority::Later => self.later.push_back(call),
        }
    }

    pub fn in_flight(&self, key: CallKey) -> bool {
        self.flying.values().any(|(call, _)| call.key() == key)
    }

    pub fn busy(&self) -> bool {
        !self.flying.is_empty()
    }

    pub fn hold(&mut self, until: Instant) {
        self.hold = Some(until);
    }

    pub fn release(&mut self, now: Instant) -> Vec<(Ticket, Call)> {
        while self.sent.front().is_some_and(|at| now >= *at + WINDOW) {
            self.sent.pop_front();
        }
        let mut released = Vec::new();
        while self.flying.len() < self.workers && !self.held(now) {
            let Some((call, priority)) = self.pop(self.sent.len()) else {
                break;
            };
            let ticket = Ticket(self.next_ticket);
            self.next_ticket += 1;
            self.sent.push_back(now);
            self.flying.insert(ticket, (call.clone(), priority));
            released.push((ticket, call));
        }
        released
    }

    fn pop(&mut self, spent: usize) -> Option<(Call, Priority)> {
        if spent < LIMIT
            && let Some(call) = self.now.pop_front()
        {
            return Some((call, Priority::Now));
        }
        if spent + RESERVE < LIMIT {
            return self.later.pop_front().map(|call| (call, Priority::Later));
        }
        None
    }

    fn held(&self, now: Instant) -> bool {
        self.hold.is_some_and(|until| now < until)
    }

    pub fn finish(&mut self, ticket: Ticket) -> Option<(Call, Priority)> {
        self.flying.remove(&ticket)
    }

    pub fn reset(&mut self) {
        self.now.clear();
        self.later.clear();
        self.flying.clear();
    }

    pub fn next_wake(&self, now: Instant) -> Option<Instant> {
        if self.now.is_empty() && self.later.is_empty() {
            return None;
        }
        if self.held(now) {
            return self.hold;
        }
        if self.flying.len() >= self.workers {
            return None;
        }
        let spent = self.sent.len();
        let window_full = (!self.now.is_empty() && spent >= LIMIT)
            || (!self.later.is_empty() && spent + RESERVE >= LIMIT);
        window_full
            .then(|| self.sent.front().map(|at| *at + WINDOW))
            .flatten()
    }
}
