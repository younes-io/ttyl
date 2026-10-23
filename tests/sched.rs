use std::time::{Duration, Instant};

use ttyl::api::Call;
use ttyl::clock::{Timer, Timers};
use ttyl::model::Log;
use ttyl::sched::{LIMIT, Priority, RESERVE, Scheduler, WINDOW};

fn calls(released: &[(ttyl::sched::Ticket, Call)]) -> Vec<&Call> {
    released.iter().map(|(_, call)| call).collect()
}

#[test]
fn the_users_call_goes_before_a_poll() {
    let now = Instant::now();
    let mut sched = Scheduler::new(2);
    sched.send(Call::Me, Priority::Later);
    sched.send(Call::Page(Log::Latest), Priority::Now);

    let released = sched.release(now);

    assert_eq!(calls(&released), [&Call::Page(Log::Latest), &Call::Me]);
}

#[test]
fn only_as_many_calls_fly_as_there_are_workers() {
    let now = Instant::now();
    let mut sched = Scheduler::new(2);
    for log in [Log::Following, Log::Latest, Log::Trending] {
        sched.send(Call::Page(log), Priority::Now);
    }

    let first = sched.release(now);
    assert_eq!(first.len(), 2);
    assert!(sched.release(now).is_empty());

    sched.finish(first[0].0);
    assert_eq!(calls(&sched.release(now)), [&Call::Page(Log::Trending)]);
}

#[test]
fn a_call_in_flight_is_not_sent_twice() {
    let now = Instant::now();
    let mut sched = Scheduler::new(2);
    sched.send(Call::Page(Log::Latest), Priority::Now);
    let released = sched.release(now);
    assert_eq!(released.len(), 1);

    sched.send(Call::Page(Log::Latest), Priority::Now);
    assert!(sched.release(now).is_empty());

    sched.finish(released[0].0);
    sched.send(Call::Page(Log::Latest), Priority::Now);
    assert_eq!(sched.release(now).len(), 1);
}

#[test]
fn a_queued_call_is_replaced_not_doubled() {
    let now = Instant::now();
    let mut sched = Scheduler::new(0);
    sched.send(Call::Me, Priority::Later);
    sched.send(Call::Me, Priority::Now);
    assert!(sched.release(now).is_empty());

    let mut sched = Scheduler::new(2);
    sched.send(Call::Me, Priority::Later);
    sched.send(Call::Me, Priority::Now);
    assert_eq!(sched.release(now).len(), 1);
}

#[test]
fn a_replaced_call_keeps_the_higher_priority() {
    let now = Instant::now();
    let mut sched = Scheduler::new(1);
    sched.send(Call::Page(Log::Latest), Priority::Later);
    sched.send(Call::Me, Priority::Now);
    sched.send(Call::Me, Priority::Later);
    assert_eq!(
        calls(&sched.release(now)),
        [&Call::Me],
        "a poll cannot demote the user's call"
    );

    let mut sched = Scheduler::new(1);
    sched.send(Call::Page(Log::Latest), Priority::Now);
    sched.send(Call::Me, Priority::Later);
    sched.send(Call::Me, Priority::Now);
    assert_eq!(
        calls(&sched.release(now)),
        [&Call::Page(Log::Latest)],
        "the promoted call joins the end of the queue"
    );
}

#[test]
fn a_reset_drops_the_queues_and_disowns_what_was_flying() {
    let now = Instant::now();
    let mut sched = Scheduler::new(1);
    sched.send(Call::Me, Priority::Now);
    sched.send(Call::Page(Log::Latest), Priority::Later);
    let released = sched.release(now);
    assert_eq!(released.len(), 1);

    sched.reset();
    assert!(sched.release(now).is_empty());
    assert!(sched.finish(released[0].0).is_none());

    sched.send(Call::Me, Priority::Now);
    let again = sched.release(now);
    assert_eq!(again.len(), 1);
    assert_ne!(
        again[0].0, released[0].0,
        "tickets stay unique across a reset"
    );
}

#[test]
fn a_hold_keeps_everything_until_it_passes() {
    let now = Instant::now();
    let mut sched = Scheduler::new(2);
    sched.send(Call::Me, Priority::Now);
    sched.hold(now + Duration::from_secs(7));

    assert!(sched.release(now).is_empty());
    assert_eq!(sched.next_wake(now), Some(now + Duration::from_secs(7)));
    assert!(sched.release(now + Duration::from_secs(6)).is_empty());
    assert_eq!(sched.release(now + Duration::from_secs(7)).len(), 1);
}

#[test]
fn polls_leave_a_reserve_for_the_user_and_the_window_slides() {
    let start = Instant::now();
    let mut sched = Scheduler::new(LIMIT);
    for _ in 0..LIMIT - RESERVE {
        sched.send(Call::Me, Priority::Later);
        let released = sched.release(start);
        assert_eq!(released.len(), 1);
        sched.finish(released[0].0);
    }

    sched.send(Call::Me, Priority::Later);
    assert!(
        sched.release(start).is_empty(),
        "a poll past the reserve waits"
    );
    assert_eq!(sched.next_wake(start), Some(start + WINDOW));

    for _ in 0..RESERVE {
        sched.send(Call::Page(Log::Latest), Priority::Now);
        let released = sched.release(start + Duration::from_secs(1));
        assert_eq!(released.len(), 1, "the user still gets the reserve");
        sched.finish(released[0].0);
    }
    sched.send(Call::Page(Log::Latest), Priority::Now);
    assert!(sched.release(start + Duration::from_secs(1)).is_empty());

    let released = sched.release(start + WINDOW);
    assert_eq!(released.len(), 2, "the window slid and both queues drain");
}

#[test]
fn timers_come_due_in_order_and_set_replaces() {
    let now = Instant::now();
    let mut timers = Timers::default();
    timers.set(Timer::Poll, now + Duration::from_secs(30));
    timers.set(Timer::Toast, now + Duration::from_secs(4));
    timers.set(Timer::Poll, now + Duration::from_secs(2));

    assert_eq!(timers.next(), Some(now + Duration::from_secs(2)));
    assert!(timers.due(now).is_empty());
    assert_eq!(
        timers.due(now + Duration::from_secs(5)),
        [Timer::Poll, Timer::Toast]
    );
    assert_eq!(timers.next(), None);
}
