use std::env;
use std::fs::{self, OpenOptions};
use std::io::{self, Write};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU32, Ordering};

pub fn chosen() -> String {
    ["VISUAL", "EDITOR"]
        .into_iter()
        .filter_map(|name| env::var(name).ok())
        .find(|editor| !editor.trim().is_empty())
        .unwrap_or_else(|| "vi".to_owned())
}

pub fn edit(editor: &str, text: &str) -> Option<String> {
    let path = temp_path();
    let edited = create_private(&path, text)
        .ok()
        .and_then(|()| run(editor, &path));
    let _ = fs::remove_file(&path);
    edited
}

fn temp_path() -> PathBuf {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let next = NEXT.fetch_add(1, Ordering::Relaxed);
    env::temp_dir().join(format!("ttyl-commit-{}-{next}.md", std::process::id()))
}

fn create_private(path: &Path, text: &str) -> io::Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)?;
    file.write_all(text.as_bytes())?;
    if !text.is_empty() {
        file.write_all(b"\n")?;
    }
    Ok(())
}

/// Through `sh` like git, so an editor with arguments such as `code --wait` works.
fn run(editor: &str, path: &Path) -> Option<String> {
    let status = Command::new("sh")
        .arg("-c")
        .arg(format!("{editor} \"$@\""))
        .arg(editor)
        .arg(path)
        .status()
        .ok()?;
    if !status.success() {
        return None;
    }
    fs::read_to_string(path).ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_editor_gets_the_draft_and_its_saved_text_comes_back() {
        let edited = edit("printf 'second line\\n' >>", "first line");
        assert_eq!(edited.as_deref(), Some("first line\nsecond line\n"));
    }

    #[test]
    fn an_editor_that_fails_gives_nothing_back() {
        assert_eq!(edit("false", "draft"), None);
        assert_eq!(edit("no-such-editor-anywhere", "draft"), None);
    }

    #[test]
    fn the_file_is_private_and_gone_afterwards() {
        let record = env::temp_dir().join(format!("ttyl-editor-test-{}", std::process::id()));
        let editor = format!("stat -c '%a %n' \"$1\" >'{}'; true", record.display());
        assert_eq!(edit(&editor, "x").as_deref(), Some("x\n"));
        let seen = fs::read_to_string(&record).unwrap();
        let _ = fs::remove_file(&record);
        let (mode, path) = seen.trim_end().split_once(' ').unwrap();
        assert_eq!(mode, "600");
        assert!(!Path::new(path).exists(), "{path}");
    }
}
