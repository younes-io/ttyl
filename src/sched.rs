//! Decides which call runs next and when, from the UI thread's clock. Workers only run what it releases.

use std::collections::{HashMap, VecDeque};
use std::time::{Duration, Instant};

use crate::api::Call;
use crate::model::Log;

/// The server's signed throttle: this many requests in any sliding window.
pub const LIMIT: usize = 120;
pub const WINDOW: Duration = Duration::from_secs(60);
/// Requests the window keeps back for key presses, so a poll never causes a 429 under the user's hands.
pub const RESERVE: usize = 40;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Ticket(u64);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Priority {
    /// The user asked for it.
    Now,
    /// A poll or a retry.
    Later,
}

/// The identity a call is coalesced on: a second call with the same key replaces a queued one and is dropped
/// while one is in flight.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum CallKey {
    Me,
    Redeem,
    Page(Log),
}

impl Call {
    pub fn key(&self) -> CallKey {
        match self {
            Call::Me => CallKey::Me,
            Call::Redeem { .. } => CallKey::Redeem,
            Call::Page(log) => CallKey::Page(*log),
        }
    }
}

pub struct Scheduler {
    now: VecDeque<Call>,
    later: VecDeque<Call>,
    flying: HashMap<Ticket, Call>,
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

    /// Queues a call unless the same one is already in flight.
    pub fn send(&mut self, call: Call, priority: Priority) {
        let key = call.key();
        if self.in_flight(key) {
            return;
        }
        self.now.retain(|queued| queued.key() != key);
        self.later.retain(|queued| queued.key() != key);
        match priority {
            Priority::Now => self.now.push_back(call),
            Priority::Later => self.later.push_back(call),
        }
    }

    pub fn in_flight(&self, key: CallKey) -> bool {
        self.flying.values().any(|call| call.key() == key)
    }

    pub fn busy(&self) -> bool {
        !self.flying.is_empty()
    }

    /// Stops releasing anything until the server's `Retry-After` has passed.
    pub fn hold(&mut self, until: Instant) {
        self.hold = Some(until);
    }

    /// Hands out the calls the window, the hold and the free workers allow right now.
    pub fn release(&mut self, now: Instant) -> Vec<(Ticket, Call)> {
        while self.sent.front().is_some_and(|at| now >= *at + WINDOW) {
            self.sent.pop_front();
        }
        let mut released = Vec::new();
        while self.flying.len() < self.workers && !self.held(now) {
            let Some(call) = self.pop(self.sent.len()) else {
                break;
            };
            let ticket = Ticket(self.next_ticket);
            self.next_ticket += 1;
            self.sent.push_back(now);
            self.flying.insert(ticket, call.clone());
            released.push((ticket, call));
        }
        released
    }

    fn pop(&mut self, spent: usize) -> Option<Call> {
        if spent < LIMIT
            && let Some(call) = self.now.pop_front()
        {
            return Some(call);
        }
        if spent + RESERVE < LIMIT {
            return self.later.pop_front();
        }
        None
    }

    fn held(&self, now: Instant) -> bool {
        self.hold.is_some_and(|until| now < until)
    }

    pub fn finish(&mut self, ticket: Ticket) -> Option<Call> {
        self.flying.remove(&ticket)
    }

    /// When something queued could be released next without another event.
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
