use serde::{Deserialize, Serialize};

pub const MIN_OPTIONS: usize = 2;
pub const MAX_OPTIONS: usize = 4;
/// The server counts an option's length in characters.
pub const OPTION_LIMIT: usize = 50;

/// What a commit carries besides its body.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attachment {
    #[default]
    None,
    Poll(Poll),
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Poll {
    pub options: Vec<String>,
    pub duration: Duration,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Duration {
    Hour,
    #[default]
    Day,
    ThreeDays,
    Week,
}

const DURATIONS: [Duration; 4] = [
    Duration::Hour,
    Duration::Day,
    Duration::ThreeDays,
    Duration::Week,
];

impl Duration {
    pub fn minutes(self) -> u32 {
        match self {
            Duration::Hour => 60,
            Duration::Day => 1440,
            Duration::ThreeDays => 4320,
            Duration::Week => 10080,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Duration::Hour => "1 hour",
            Duration::Day => "1 day",
            Duration::ThreeDays => "3 days",
            Duration::Week => "7 days",
        }
    }

    /// The next duration up, or down, coming round at the ends.
    pub fn step(self, longer: bool) -> Duration {
        let at = DURATIONS.iter().position(|d| *d == self).unwrap_or(1);
        let by = if longer { 1 } else { DURATIONS.len() - 1 };
        DURATIONS[(at + by) % DURATIONS.len()]
    }
}

impl Poll {
    /// The poll as the server takes it: options trimmed and the empty ones left out.
    pub fn sent(&self) -> Poll {
        Poll {
            options: self
                .options
                .iter()
                .map(|option| option.trim().to_owned())
                .filter(|option| !option.is_empty())
                .collect(),
            duration: self.duration,
        }
    }

    /// Why the server would turn this poll down, in the words the composer shows.
    pub fn refusal(&self) -> Option<String> {
        // Numbered as the composer shows them, empty rows included.
        let filled: Vec<(usize, String)> = (self.options.iter().enumerate())
            .map(|(at, option)| (at + 1, option.trim().to_lowercase()))
            .filter(|(_, option)| !option.is_empty())
            .collect();
        if filled.len() < MIN_OPTIONS {
            return Some("a poll needs at least two options.".to_owned());
        }
        if let Some((at, _)) =
            (filled.iter()).find(|(_, option)| option.chars().count() > OPTION_LIMIT)
        {
            return Some(format!("option {at} is over {OPTION_LIMIT} characters."));
        }
        for (later, (at, option)) in filled.iter().enumerate() {
            if let Some((first, _)) = filled[..later].iter().find(|(_, o)| o == option) {
                return Some(format!("options {first} and {at} are the same."));
            }
        }
        None
    }
}
