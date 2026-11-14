//! The times a commit can be scheduled for, read the way at(1) reads them: `14:30` for its next
//! occurrence, `tomorrow 9:00`, `fri 14:00`, `in 2h`, or an RFC 3339 time.

use std::fmt;

use chrono::{DateTime, Datelike, Days, Duration, LocalResult, NaiveTime, TimeZone, Weekday};

/// How far ahead dev-null.sh takes a scheduled commit.
pub const FURTHEST_DAYS: i64 = 30;
/// The forms `send at:` and `--at` read, as a reminder.
pub const FORMS: &str = "14:30 · tomorrow 9:00 · fri 14:00 · in 2h";

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Refusal {
    /// Not a time this grammar reads.
    Unreadable(String),
    Past,
    TooFar,
    /// The clocks skip that time on that day, as when summer time starts.
    Skipped,
}

impl fmt::Display for Refusal {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        match self {
            Refusal::Unreadable(text) => write!(
                f,
                "can't read '{text}' as a time. try {}, or 2026-10-24T09:00:00+02:00",
                FORMS.replace(" · ", ", ")
            ),
            Refusal::Past => f.write_str("that time has passed"),
            Refusal::TooFar => write!(f, "dev-null.sh schedules up to {FURTHEST_DAYS} days ahead"),
            Refusal::Skipped => f.write_str("the clocks skip that time there"),
        }
    }
}

/// When `text` falls after `now`, in `now`'s time zone, so a day ahead keeps that day's offset.
pub fn resolve<Tz: TimeZone>(text: &str, now: &DateTime<Tz>) -> Result<DateTime<Tz>, Refusal> {
    let unreadable = || Refusal::Unreadable(text.trim().to_owned());
    let lower = text.trim().to_lowercase();
    let words: Vec<&str> = lower.split_whitespace().collect();
    let at = match words.as_slice() {
        ["in", span @ ..] if !span.is_empty() => {
            now.clone() + duration(&span.concat()).ok_or_else(unreadable)?
        }
        [time] if let Some(time) = clock(time) => day_at(now, Day::Next, time)?,
        [day, time] => {
            let day = named_day(day).ok_or_else(unreadable)?;
            day_at(now, day, clock(time).ok_or_else(unreadable)?)?
        }
        [_] => DateTime::parse_from_rfc3339(text.trim())
            .map_err(|_| unreadable())?
            .with_timezone(&now.timezone()),
        _ => return Err(unreadable()),
    };
    if at <= *now {
        return Err(Refusal::Past);
    }
    if at.clone() - now.clone() > Duration::days(FURTHEST_DAYS) {
        return Err(Refusal::TooFar);
    }
    Ok(at)
}

/// The day a time of day falls on.
enum Day {
    /// Today if the time is still ahead, else tomorrow.
    Next,
    Today,
    Tomorrow,
    /// The next one of these, today included while the time is still ahead.
    Weekday(Weekday),
}

fn named_day(word: &str) -> Option<Day> {
    match word {
        "today" => Some(Day::Today),
        "tomorrow" => Some(Day::Tomorrow),
        _ => word.parse().ok().map(Day::Weekday),
    }
}

/// `14:30`, `9:00`, `9am` or `2:30pm`.
fn clock(word: &str) -> Option<NaiveTime> {
    let (word, afternoon) = match (word.strip_suffix("am"), word.strip_suffix("pm")) {
        (Some(word), _) => (word, Some(false)),
        (_, Some(word)) => (word, Some(true)),
        _ => (word, None),
    };
    let (hour, minute) = match (word.split_once(':'), afternoon) {
        (Some((hour, minute)), _) if minute.len() == 2 => (hour, minute),
        (None, Some(_)) => (word, "00"),
        _ => return None,
    };
    if hour.is_empty() || hour.len() > 2 {
        return None;
    }
    let (mut hour, minute): (u32, u32) = (hour.parse().ok()?, minute.parse().ok()?);
    if let Some(afternoon) = afternoon {
        if !(1..=12).contains(&hour) {
            return None;
        }
        hour = hour % 12 + if afternoon { 12 } else { 0 };
    }
    NaiveTime::from_hms_opt(hour, minute, 0)
}

/// `2h`, `30m`, `1d`, or several of them run together, as `1h30m` or `in 2 hours` give.
fn duration(span: &str) -> Option<Duration> {
    let mut total = Duration::zero();
    let mut rest = span;
    while !rest.is_empty() {
        let digits = rest.find(|ch: char| !ch.is_ascii_digit())?;
        let count: i64 = rest[..digits].parse().ok()?;
        rest = &rest[digits..];
        let unit = rest
            .find(|ch: char| ch.is_ascii_digit())
            .unwrap_or(rest.len());
        let minutes = match &rest[..unit] {
            "m" | "min" | "mins" | "minute" | "minutes" => 1,
            "h" | "hr" | "hrs" | "hour" | "hours" => 60,
            "d" | "day" | "days" => 60 * 24,
            _ => return None,
        };
        total += Duration::try_minutes(count.checked_mul(minutes)?)?;
        rest = &rest[unit..];
    }
    Some(total)
}

fn day_at<Tz: TimeZone>(
    now: &DateTime<Tz>,
    day: Day,
    time: NaiveTime,
) -> Result<DateTime<Tz>, Refusal> {
    let today = now.date_naive();
    let ahead = time > now.time();
    let days = match day {
        Day::Next | Day::Weekday(_) if ahead => 0,
        Day::Next | Day::Tomorrow => 1,
        Day::Today => 0,
        Day::Weekday(_) => 7,
    };
    let days = match day {
        Day::Weekday(weekday) => {
            let until = weekday.days_since(today.weekday());
            if until == 0 { days } else { until.into() }
        }
        _ => days,
    };
    let date = today + Days::new(days);
    match now.timezone().from_local_datetime(&date.and_time(time)) {
        LocalResult::Single(at) | LocalResult::Ambiguous(at, _) => Ok(at),
        LocalResult::None => Err(Refusal::Skipped),
    }
}

/// `Fri 24 Oct 09:00`, as the time shows wherever it is written.
pub fn shown<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: fmt::Display,
{
    at.format("%a %-d %b %H:%M").to_string()
}

/// How long until `at`, as `in 25m`, `in 3h 10m` or `in 2d 4h`.
pub fn until<Tz: TimeZone>(at: &DateTime<Tz>, now: &DateTime<Tz>) -> String {
    let minutes = (at.clone() - now.clone()).num_minutes().max(0);
    let (days, hours, minutes) = (minutes / (60 * 24), minutes / 60 % 24, minutes % 60);
    match (days, hours, minutes) {
        (0, 0, minutes) => format!("in {minutes}m"),
        (0, hours, 0) => format!("in {hours}h"),
        (0, hours, minutes) => format!("in {hours}h {minutes}m"),
        (days, 0, _) => format!("in {days}d"),
        (days, hours, _) => format!("in {days}d {hours}h"),
    }
}

/// What the API takes: `2026-10-24T09:00:00+02:00`.
pub fn wire<Tz: TimeZone>(at: &DateTime<Tz>) -> String
where
    Tz::Offset: fmt::Display,
{
    at.format("%Y-%m-%dT%H:%M:%S%:z").to_string()
}

#[cfg(test)]
mod tests {
    use chrono::{FixedOffset, MappedLocalTime, NaiveDate, NaiveDateTime, Offset};

    use super::*;

    /// Wednesday 21 October 2026, 10:00 in UTC+2.
    fn now() -> DateTime<FixedOffset> {
        DateTime::parse_from_rfc3339("2026-10-21T10:00:00+02:00").unwrap()
    }

    fn at(text: &str) -> String {
        match resolve(text, &now()) {
            Ok(at) => wire(&at),
            Err(refusal) => format!("{refusal:?}"),
        }
    }

    #[test]
    fn a_time_of_day_is_its_next_occurrence() {
        assert_eq!(at("14:30"), "2026-10-21T14:30:00+02:00");
        assert_eq!(at("9:00"), "2026-10-22T09:00:00+02:00");
        assert_eq!(at("10:00"), "2026-10-22T10:00:00+02:00");
        assert_eq!(at("2pm"), "2026-10-21T14:00:00+02:00");
        assert_eq!(at("12am"), "2026-10-22T00:00:00+02:00");
        assert_eq!(at("9:30AM"), "2026-10-22T09:30:00+02:00");
    }

    #[test]
    fn a_day_names_the_date() {
        assert_eq!(at("tomorrow 9:00"), "2026-10-22T09:00:00+02:00");
        assert_eq!(at("Tomorrow  23:59"), "2026-10-22T23:59:00+02:00");
        assert_eq!(at("today 18:00"), "2026-10-21T18:00:00+02:00");
        assert_eq!(at("fri 14:00"), "2026-10-23T14:00:00+02:00");
        assert_eq!(at("friday 14:00"), "2026-10-23T14:00:00+02:00");
        assert_eq!(at("mon 9:00"), "2026-10-26T09:00:00+02:00");
    }

    #[test]
    fn todays_weekday_is_today_while_the_time_is_ahead_and_next_week_after() {
        assert_eq!(at("wed 11:00"), "2026-10-21T11:00:00+02:00");
        assert_eq!(at("wed 9:00"), "2026-10-28T09:00:00+02:00");
    }

    #[test]
    fn in_counts_from_now() {
        assert_eq!(at("in 2h"), "2026-10-21T12:00:00+02:00");
        assert_eq!(at("in 30m"), "2026-10-21T10:30:00+02:00");
        assert_eq!(at("in 1h30m"), "2026-10-21T11:30:00+02:00");
        assert_eq!(at("in 2 hours"), "2026-10-21T12:00:00+02:00");
        assert_eq!(at("in 3d"), "2026-10-24T10:00:00+02:00");
    }

    #[test]
    fn rfc_3339_is_taken_as_written_and_shown_in_the_local_zone() {
        assert_eq!(at("2026-10-24T09:00:00Z"), "2026-10-24T11:00:00+02:00");
        assert_eq!(at("2026-10-24T09:00:00+02:00"), "2026-10-24T09:00:00+02:00");
    }

    #[test]
    fn past_and_too_far_are_refused() {
        assert_eq!(at("today 9:00"), "Past");
        assert_eq!(at("in 0m"), "Past");
        assert_eq!(at("2020-01-01T00:00:00Z"), "Past");
        assert_eq!(at("in 31d"), "TooFar");
        assert_eq!(at("in 30d"), "2026-11-20T10:00:00+02:00");
    }

    #[test]
    fn what_it_cannot_read_is_named_back() {
        for text in [
            "",
            "soon",
            "fri",
            "fri 1",
            "next friday",
            "25:00",
            "14:5",
            "13pm",
            "in",
            "in 2y",
            "in h",
            "in 99999999999999999d",
            "noon 12:00",
        ] {
            assert_eq!(
                resolve(text, &now()),
                Err(Refusal::Unreadable(text.trim().to_owned())),
                "{text}"
            );
        }
        assert_eq!(
            Refusal::Unreadable("soon".to_owned()).to_string(),
            "can't read 'soon' as a time. try 14:30, tomorrow 9:00, fri 14:00, in 2h, or 2026-10-24T09:00:00+02:00"
        );
    }

    #[test]
    fn shown_and_until_read_like_a_calendar() {
        let later = resolve("fri 9:00", &now()).unwrap();
        assert_eq!(shown(&later), "Fri 23 Oct 09:00");
        assert_eq!(until(&later, &now()), "in 1d 23h");
        let soon = resolve("in 25m", &now()).unwrap();
        assert_eq!(until(&soon, &now()), "in 25m");
        assert_eq!(
            until(&resolve("in 3h10m", &now()).unwrap(), &now()),
            "in 3h 10m"
        );
        assert_eq!(until(&resolve("in 2h", &now()).unwrap(), &now()), "in 2h");
        assert_eq!(until(&resolve("in 2d", &now()).unwrap(), &now()), "in 2d");
    }

    /// Paris in 2026: summer time ends at 03:00 on 25 October and starts at 02:00 on 29 March.
    #[derive(Debug, Clone, Copy)]
    struct Paris;

    impl Paris {
        fn summer(utc: NaiveDateTime) -> bool {
            let start = NaiveDate::from_ymd_opt(2026, 3, 29)
                .unwrap()
                .and_hms_opt(1, 0, 0);
            let end = NaiveDate::from_ymd_opt(2026, 10, 25)
                .unwrap()
                .and_hms_opt(1, 0, 0);
            start.unwrap() <= utc && utc < end.unwrap()
        }

        fn offset(summer: bool) -> FixedOffset {
            FixedOffset::east_opt(if summer { 7200 } else { 3600 }).unwrap()
        }
    }

    impl TimeZone for Paris {
        type Offset = FixedOffset;

        fn from_offset(_: &FixedOffset) -> Self {
            Paris
        }

        fn offset_from_local_date(&self, _: &NaiveDate) -> MappedLocalTime<FixedOffset> {
            unimplemented!("only times of day are resolved")
        }

        fn offset_from_local_datetime(
            &self,
            local: &NaiveDateTime,
        ) -> MappedLocalTime<FixedOffset> {
            let fits = |summer: bool| {
                let offset = Paris::offset(summer);
                (Paris::summer(*local - offset.fix()) == summer).then_some(offset)
            };
            match (fits(true), fits(false)) {
                (Some(summer), Some(winter)) => MappedLocalTime::Ambiguous(summer, winter),
                (Some(one), None) | (None, Some(one)) => MappedLocalTime::Single(one),
                (None, None) => MappedLocalTime::None,
            }
        }

        fn offset_from_utc_date(&self, _: &NaiveDate) -> FixedOffset {
            unimplemented!("only times of day are resolved")
        }

        fn offset_from_utc_datetime(&self, utc: &NaiveDateTime) -> FixedOffset {
            Paris::offset(Paris::summer(*utc))
        }
    }

    #[test]
    fn a_day_past_the_clock_change_keeps_that_days_offset() {
        let friday = Paris.with_ymd_and_hms(2026, 10, 23, 10, 0, 0).unwrap();
        assert_eq!(
            wire(&resolve("mon 9:00", &friday).unwrap()),
            "2026-10-26T09:00:00+01:00"
        );
        assert_eq!(
            wire(&resolve("sun 2:30", &friday).unwrap()),
            "2026-10-25T02:30:00+02:00"
        );
        let saturday = Paris.with_ymd_and_hms(2026, 3, 28, 10, 0, 0).unwrap();
        assert_eq!(resolve("sun 2:30", &saturday), Err(Refusal::Skipped));
    }
}
