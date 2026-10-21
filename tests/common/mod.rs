use std::fs;
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Stdio};

use devnull::device::DeviceKey;

pub fn scratch(name: &str) -> PathBuf {
    let dir = PathBuf::from(env!("CARGO_TARGET_TMPDIR")).join(name);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}

/// Runs `ssh-keygen -Y verify` the way an SSHSIG verifier independent of this crate would check the request.
pub fn openssh_accepts(
    key: &DeviceKey,
    signature: &str,
    message: &str,
    namespace: &str,
    dir: &str,
) -> bool {
    let dir = scratch(dir);
    fs::write(
        dir.join("allowed"),
        format!("devnull {}\n", key.public_key()),
    )
    .unwrap();
    let body: Vec<&str> = signature
        .as_bytes()
        .chunks(70)
        .map(|chunk| std::str::from_utf8(chunk).unwrap())
        .collect();
    fs::write(
        dir.join("sig"),
        format!(
            "-----BEGIN SSH SIGNATURE-----\n{}\n-----END SSH SIGNATURE-----\n",
            body.join("\n")
        ),
    )
    .unwrap();
    let mut child = Command::new("ssh-keygen")
        .args(["-Y", "verify", "-I", "devnull", "-n", namespace, "-f"])
        .arg(dir.join("allowed"))
        .arg("-s")
        .arg(dir.join("sig"))
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("ssh-keygen is installed");
    child
        .stdin
        .take()
        .unwrap()
        .write_all(message.as_bytes())
        .unwrap();
    child.wait().unwrap().success()
}
