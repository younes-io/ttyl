use std::collections::HashMap;
use std::env;
use std::fs::{self, DirBuilder, OpenOptions};
use std::io::{self, Write as _};
use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
use std::path::{Path, PathBuf};

use crate::attach::Attachment;
use crate::model::CommitId;

/// The attachment sits beside the text in a file of its own, so a text-only draft stays plain text.
const ATTACHMENT: &str = ".attach";

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

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Draft {
    pub text: String,
    pub attachment: Attachment,
}

impl Draft {
    pub fn text(text: &str) -> Self {
        Draft {
            text: text.to_owned(),
            attachment: Attachment::None,
        }
    }

    fn is_empty(&self) -> bool {
        self.text.trim().is_empty() && self.attachment == Attachment::None
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Write {
    Save(Target, Draft),
    Drop(Target),
}

#[derive(Debug, Default)]
pub struct Drafts {
    texts: HashMap<Target, Draft>,
}

impl Drafts {
    pub fn new(texts: HashMap<Target, Draft>) -> Self {
        Drafts { texts }
    }

    pub fn get(&self, target: &Target) -> Option<&Draft> {
        self.texts.get(target)
    }

    pub fn write(&mut self, target: &Target, draft: &Draft) -> Option<Write> {
        if draft.is_empty() {
            self.texts
                .remove(target)
                .map(|_| Write::Drop(target.clone()))
        } else if self.get(target) == Some(draft) {
            None
        } else {
            self.texts.insert(target.clone(), draft.clone());
            Some(Write::Save(target.clone(), draft.clone()))
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
        .map(|state| state.join("ttyl/drafts"))
}

pub fn load(dir: &Path) -> HashMap<Target, Draft> {
    let Ok(entries) = fs::read_dir(dir) else {
        return HashMap::new();
    };
    entries
        .flatten()
        .filter_map(|entry| {
            let name = entry.file_name().into_string().ok()?;
            let target = Target::from_file_name(&name)?;
            let text = fs::read_to_string(entry.path()).ok()?;
            let attachment = fs::read_to_string(dir.join(name + ATTACHMENT))
                .ok()
                .and_then(|json| serde_json::from_str(&json).ok())
                .unwrap_or_default();
            let draft = Draft { text, attachment };
            (!draft.is_empty()).then_some((target, draft))
        })
        .collect()
}

pub(crate) fn replace_private(dir: &Path, name: &str, text: &str) -> io::Result<()> {
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

fn remove(path: &Path) -> io::Result<()> {
    match fs::remove_file(path) {
        Err(error) if error.kind() != io::ErrorKind::NotFound => Err(error),
        _ => Ok(()),
    }
}

pub fn store(dir: &Path, write: &Write) -> io::Result<()> {
    let (Write::Save(target, _) | Write::Drop(target)) = write;
    let Some(name) = target.file_name() else {
        return Ok(());
    };
    let attachment = format!("{name}{ATTACHMENT}");
    match write {
        Write::Save(_, draft) => {
            if draft.attachment == Attachment::None {
                remove(&dir.join(&attachment))?;
            } else {
                let json = serde_json::to_string(&draft.attachment).map_err(io::Error::other)?;
                replace_private(dir, &attachment, &json)?;
            }
            replace_private(dir, &name, &draft.text)
        }
        Write::Drop(_) => {
            remove(&dir.join(attachment))?;
            remove(&dir.join(name))
        }
    }
}
