use std::collections::HashMap;
use std::env;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::model::CommitId;

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Target {
    New,
    Reply(CommitId),
    Quote(CommitId),
}

impl Target {
    fn file_name(&self) -> Option<String> {
        let (kind, id) = match self {
            Target::New => return Some("new".to_owned()),
            Target::Reply(id) => ("reply", id),
            Target::Quote(id) => ("quote", id),
        };
        let safe = !id.0.is_empty()
            && id
                .0
                .chars()
                .all(|ch| ch.is_ascii_alphanumeric() || ch == '-' || ch == '_');
        safe.then(|| format!("{kind}-{}", id.0))
    }

    fn from_file_name(name: &str) -> Option<Target> {
        if name == "new" {
            return Some(Target::New);
        }
        let (kind, id) = name.split_once('-')?;
        let target = match kind {
            "reply" => Target::Reply(CommitId(id.to_owned())),
            "quote" => Target::Quote(CommitId(id.to_owned())),
            _ => return None,
        };
        (target.file_name().as_deref() == Some(name)).then_some(target)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    Save(Target, String),
    Drop(Target),
}

#[derive(Debug, Default)]
pub struct Drafts {
    texts: HashMap<Target, String>,
}

impl Drafts {
    pub fn new(texts: HashMap<Target, String>) -> Self {
        Drafts { texts }
    }

    pub fn get(&self, target: &Target) -> Option<&str> {
        self.texts.get(target).map(String::as_str)
    }

    pub fn write(&mut self, target: &Target, text: &str) -> Option<Write> {
        if text.trim().is_empty() {
            self.texts
                .remove(target)
                .map(|_| Write::Drop(target.clone()))
        } else if self.get(target) == Some(text) {
            None
        } else {
            self.texts.insert(target.clone(), text.to_owned());
            Some(Write::Save(target.clone(), text.to_owned()))
        }
    }

    pub fn discard(&mut self, target: &Target) -> Option<Write> {
        self.texts
            .remove(target)
            .map(|_| Write::Drop(target.clone()))
    }
}

pub fn dir() -> Option<PathBuf> {
    env::var_os("XDG_STATE_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".local/state")))
        .map(|state| state.join("devnull-tui/drafts"))
}

pub fn load(dir: &Path) -> HashMap<Target, String> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let target = Target::from_file_name(entry.file_name().to_str()?)?;
            let text = fs::read_to_string(entry.path()).ok()?;
            (!text.trim().is_empty()).then_some((target, text))
        })
        .collect()
}

fn replace_private(dir: &Path, name: &str, text: &str) -> io::Result<()> {
    DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
    let temp = dir.join(format!(".{name}.tmp"));
    let mut file = OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&temp)?;
    file.write_all(text.as_bytes())?;
    fs::rename(&temp, dir.join(name))
}

pub fn store(dir: &Path, write: &Write) -> io::Result<()> {
    match write {
        Write::Save(target, text) => {
            let Some(name) = target.file_name() else {
                return Ok(());
            };
            replace_private(dir, &name, text)
        }
        Write::Drop(target) => {
            let Some(name) = target.file_name() else {
                return Ok(());
            };
            match fs::remove_file(dir.join(name)) {
                Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
                _ => Ok(()),
            }
        }
    }
}
