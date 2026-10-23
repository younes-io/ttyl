use std::fmt;
use std::fs::DirBuilder;
use std::io;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::{env, fs};

use ssh_key::rand_core::{OsRng, RngCore};
use ssh_key::{Algorithm, HashAlg, LineEnding, PrivateKey};

const NAMESPACE: &str = "devnull-api";

/// The ed25519 key this install signs every API request with. The server knows it by its fingerprint once the user
/// approves it.
pub struct DeviceKey(PrivateKey);

impl DeviceKey {
    pub fn generate() -> ssh_key::Result<Self> {
        PrivateKey::random(&mut OsRng, Algorithm::Ed25519).map(DeviceKey)
    }

    pub fn load(path: &Path) -> ssh_key::Result<Self> {
        PrivateKey::read_openssh_file(path).map(DeviceKey)
    }

    /// Loads the key at `path`, or makes one there on the first run, in a directory only the owner can open.
    pub fn open(path: &Path) -> ssh_key::Result<Self> {
        if fs::exists(path)? {
            return Self::load(path);
        }
        if let Some(dir) = path.parent() {
            DirBuilder::new().recursive(true).mode(0o700).create(dir)?;
        }
        let key = Self::generate()?;
        key.save(path)?;
        Ok(key)
    }

    /// Writes the key in OpenSSH format, readable only by the owner.
    pub fn save(&self, path: &Path) -> ssh_key::Result<()> {
        self.0.write_openssh_file(path, LineEnding::LF)
    }

    pub fn public_key(&self) -> String {
        self.0
            .public_key()
            .to_openssh()
            .expect("an ed25519 public key encodes")
    }

    /// `SHA256:` followed by 43 base64 characters, the form the server matches devices by.
    pub fn fingerprint(&self) -> String {
        self.0.fingerprint(HashAlg::Sha256).to_string()
    }

    /// Signs one request the way the server rebuilds it: timestamp, nonce, method, path with query, and raw body,
    /// joined by newlines.
    pub fn sign(
        &self,
        method: &str,
        path_and_query: &str,
        body: &str,
        timestamp: i64,
        nonce: Nonce,
    ) -> Signed {
        let timestamp = timestamp.to_string();
        let nonce = nonce.to_string();
        let message = [timestamp.as_str(), &nonce, method, path_and_query, body].join("\n");
        let pem = self
            .0
            .sign(NAMESPACE, HashAlg::Sha512, message.as_bytes())
            .and_then(|signature| signature.to_pem(LineEnding::LF))
            .expect("an ed25519 key signs any message");
        Signed {
            timestamp,
            nonce,
            signature: pem
                .lines()
                .filter(|line| !line.starts_with("-----"))
                .collect(),
        }
    }
}

/// `$XDG_CONFIG_HOME/ttyl/device_ed25519`, falling back to `~/.config`.
pub fn key_path() -> io::Result<PathBuf> {
    env::var_os("XDG_CONFIG_HOME")
        .map(PathBuf::from)
        .or_else(|| env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .map(|config| config.join("ttyl/device_ed25519"))
        .ok_or_else(|| io::Error::other("no home directory"))
}

/// 16 random bytes the server accepts once, sent as 32 lowercase hex characters.
#[derive(Clone, Copy)]
pub struct Nonce([u8; 16]);

impl Nonce {
    pub fn random() -> Self {
        let mut bytes = [0; 16];
        OsRng.fill_bytes(&mut bytes);
        Nonce(bytes)
    }
}

impl fmt::Display for Nonce {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        self.0.iter().try_for_each(|byte| write!(f, "{byte:02x}"))
    }
}

/// The values of a signed request's three headers. The signature is the SSHSIG base64 body on one line, since a
/// header cannot carry the armored block's line breaks.
pub struct Signed {
    pub timestamp: String,
    pub nonce: String,
    pub signature: String,
}

impl Signed {
    pub fn headers(&self) -> [(&'static str, &str); 3] {
        [
            ("X-Devnull-Timestamp", &self.timestamp),
            ("X-Devnull-Nonce", &self.nonce),
            ("X-SSH-Signature", &self.signature),
        ]
    }
}
