use std::env;
use std::fmt::Write as _;
use std::fs;
use std::path::PathBuf;

use ssh_key::sha2::{Digest, Sha256};

use crate::api::Account;
use crate::device::DeviceKey;
use crate::draft::replace_private;
use crate::log::LOGS;
use crate::model::{Log, LogPage};

const ME: &str = "me.json";

/// The account and first log pages the server last sent, kept so the next launch draws them at once.
pub struct Cache {
    dir: PathBuf,
}

/// What a launch shows before the server answers.
pub struct Start {
    pub account: Account,
    pub pages: Vec<LogPage>,
}

impl Cache {
    /// `$XDG_CACHE_HOME/ttyl/<id>`, falling back to `~/.cache`, where the id is a hash of the server and
    /// device, so another account or server never shows this one's timeline.
    pub fn locate(base: &str, key: &DeviceKey) -> Option<Cache> {
        let root = env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))?;
        let digest = Sha256::digest(format!("{base}\n{}", key.fingerprint()));
        let id = digest[..16].iter().fold(String::new(), |mut id, byte| {
            let _ = write!(id, "{byte:02x}");
            id
        });
        Some(Cache::at(root.join("ttyl").join(id)))
    }

    pub fn at(dir: PathBuf) -> Cache {
        Cache { dir }
    }

    pub fn load(&self) -> Option<Start> {
        let account = serde_json::from_str(&fs::read_to_string(self.dir.join(ME)).ok()?).ok()?;
        let pages = LOGS
            .iter()
            .filter_map(|log| {
                let text = fs::read_to_string(self.dir.join(page_name(*log))).ok()?;
                LogPage::parse(&text).ok().filter(|page| page.log == *log)
            })
            .collect();
        Some(Start { account, pages })
    }

    pub(crate) fn keep_me(&self, text: &str) {
        let _ = replace_private(&self.dir, ME, text);
    }

    pub(crate) fn keep_page(&self, log: Log, text: &str) {
        let _ = replace_private(&self.dir, &page_name(log), text);
    }

    pub(crate) fn forget(&self) {
        let _ = fs::remove_dir_all(&self.dir);
    }
}

fn page_name(log: Log) -> String {
    format!("log-{}.json", log.slug())
}
