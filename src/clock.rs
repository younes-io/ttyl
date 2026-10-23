use std::collections::HashMap;
use std::time::{Duration, Instant};

use crate::model::{CommitId, Filter, Log};

pub const POLL: Duration = Duration::from_secs(30);
pub const RETRY: Duration = Duration::from_secs(30);
pub const MINUTE: Duration = Duration::from_secs(60);
pub const TOAST: Duration = Duration::from_secs(4);
pub const GLOW: Duration = Duration::from_secs(5);
pub const FRAME: Duration = Duration::from_millis(100);
pub const SECOND: Duration = Duration::from_secs(1);
pub const SPARK_FRAME: Duration = Duration::from_millis(33);

/// A short animation on the selected card after the user stars it or gives it a pastel, or the first
/// time it lands on a poll whose results show.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Spark {
    Star,
    Pastel,
    Fill,
}

impl Spark {
    pub fn lasts(self) -> Duration {
        match self {
            Spark::Star | Spark::Pastel => Duration::from_millis(1500),
            Spark::Fill => Duration::from_millis(1300),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Timer {
    Poll,
    Retry,
    RelativeTimes,
    Toast,
    Glow,
    Reload(Log),
    Older(Log),
    Commit(CommitId),
    Replies(CommitId),
    More(CommitId),
    Inbox(Filter),
    OlderInbox(Filter),
    Spark(Spark, CommitId),
}

#[derive(Default)]
pub struct Timers(HashMap<Timer, Instant>);

impl Timers {
    pub fn set(&mut self, timer: Timer, at: Instant) {
        self.0.insert(timer, at);
    }

    pub fn clear(&mut self, timer: &Timer) {
        self.0.remove(timer);
    }

    pub fn is_set(&self, timer: &Timer) -> bool {
        self.0.contains_key(timer)
    }

    pub fn remaining(&self, timer: &Timer, now: Instant) -> Option<Duration> {
        self.0
            .get(timer)
            .map(|at| at.saturating_duration_since(now))
    }

    pub fn due(&mut self, now: Instant) -> Vec<Timer> {
        let mut due: Vec<Timer> = self
            .0
            .iter()
            .filter(|(_, at)| **at <= now)
            .map(|(timer, _)| timer.clone())
            .collect();
        due.sort_by_key(|timer| self.0[timer]);
        for timer in &due {
            self.0.remove(timer);
        }
        due
    }

    pub fn any(&self, matches: impl Fn(&Timer) -> bool) -> bool {
        self.0.keys().any(matches)
    }

    /// The sparks playing on `id`, and how long ago each began.
    pub fn sparks(&self, id: &CommitId, now: Instant) -> Vec<(Spark, Duration)> {
        [Spark::Fill, Spark::Star, Spark::Pastel]
            .into_iter()
            .filter_map(|spark| {
                let left = self.remaining(&Timer::Spark(spark, id.clone()), now)?;
                Some((spark, spark.lasts().saturating_sub(left)))
            })
            .collect()
    }

    pub fn next(&self) -> Option<Instant> {
        self.0.values().min().copied()
    }
}
