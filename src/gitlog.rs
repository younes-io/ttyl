use std::fmt::Write as _;

use chrono::{DateTime, TimeZone};

use crate::model::{Row, Status};

const YELLOW: &str = "\x1b[33m";
const RESET: &str = "\x1b[m";

/// How `ttyl log` prints each commit, after the `git log` formats of the same names.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Format {
    Medium,
    Oneline,
}

/// The rows as `git log` would print commits, dated in `zone`, with the hashes in yellow when `color`.
pub fn print<Tz: TimeZone>(rows: &[Row], format: Format, zone: &Tz, color: bool) -> String
where
    Tz::Offset: std::fmt::Display,
{
    let paint = |hash: &str| match color {
        true => format!("{YELLOW}{hash}{RESET}"),
        false => hash.to_owned(),
    };
    let mut out = String::new();
    for (at, row) in rows.iter().enumerate() {
        let commit = &row.commit;
        let decoration = match commit.status {
            Status::Approved => "",
            Status::Pending => " (in review)",
            Status::Rejected => " (rejected)",
        };
        match format {
            Format::Oneline => {
                let subject = commit.body.lines().find(|line| !line.trim().is_empty());
                let _ = writeln!(
                    out,
                    "{}{decoration} @{} {}",
                    paint(&commit.short_hash),
                    commit.author.handle,
                    subject.unwrap_or("").trim()
                );
            }
            Format::Medium => {
                if at > 0 {
                    out.push('\n');
                }
                let date = DateTime::with_timezone(&commit.created_at, zone);
                let _ = writeln!(
                    out,
                    "{}{decoration}",
                    paint(&format!("commit {}", commit.id.0))
                );
                let _ = writeln!(
                    out,
                    "Author: {} <@{}>",
                    commit.author.name, commit.author.handle
                );
                let _ = writeln!(out, "Date:   {}", date.format("%a %b %-d %H:%M:%S %Y %z"));
                out.push('\n');
                for line in commit.body.trim_end().lines() {
                    match line.is_empty() {
                        true => out.push('\n'),
                        false => {
                            let _ = writeln!(out, "    {line}");
                        }
                    }
                }
            }
        }
    }
    out
}
