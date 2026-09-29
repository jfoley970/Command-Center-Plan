//! API keys and tokens live in a secret store, never in the database or config
//! files. The desktop app in local mode uses the OS keychain (macOS Keychain,
//! Windows Credential Manager); the server uses an encrypted file whose key is
//! handed to it at start-up (systemd credentials, a Docker secret, or a key file).

use std::collections::BTreeMap;
use std::path::{Path, PathBuf};
use std::sync::Mutex;

pub const CLAUDE_KEY: &str = "anthropic-api-key";
pub const ATERA_KEY: &str = "atera-api-key";
pub const UNIFI_KEY: &str = "unifi-site-manager-api-key";

pub fn mail_token_name(email: &str) -> String {
    format!("ms-refresh:{}", email.to_lowercase())
}

pub trait SecretStore: Send + Sync {
    fn get(&self, name: &str) -> Result<Option<String>, String>;
    fn set(&self, name: &str, value: &str) -> Result<(), String>;
    fn delete(&self, name: &str) -> Result<(), String>;
}

/// Wraps a store so a key set in the environment wins, which is handy for development.
pub struct Secrets(Box<dyn SecretStore>);

impl Secrets {
    pub fn new(store: impl SecretStore + 'static) -> Self {
        Self(Box::new(store))
    }

    fn env_name(name: &str) -> Option<&'static str> {
        match name {
            CLAUDE_KEY => Some("ANTHROPIC_API_KEY"),
            ATERA_KEY => Some("ATERA_API_KEY"),
            UNIFI_KEY => Some("UNIFI_API_KEY"),
            _ => None,
        }
    }

    pub fn get(&self, name: &str) -> Result<Option<String>, String> {
        if let Some(k) = Self::env_name(name).and_then(|v| std::env::var(v).ok()) {
            if !k.trim().is_empty() {
                return Ok(Some(k));
            }
        }
        self.0.get(name)
    }

    /// Saves a key; an empty value removes it.
    pub fn set(&self, name: &str, value: &str) -> Result<(), String> {
        let value = value.trim();
        if value.is_empty() {
            self.0.delete(name)
        } else {
            self.0.set(name, value)
        }
    }

    pub fn delete(&self, name: &str) -> Result<(), String> {
        self.0.delete(name)
    }
}

// ---------- In memory (tests) ----------

#[derive(Default)]
pub struct MemoryStore(Mutex<BTreeMap<String, String>>);

impl SecretStore for MemoryStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        Ok(self.0.lock().unwrap().get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.0.lock().unwrap().insert(name.into(), value.into());
        Ok(())
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.0.lock().unwrap().remove(name);
        Ok(())
    }
}

// ---------- Encrypted file (server) ----------

/// All secrets in one AES-256-GCM encrypted JSON file. The 32-byte key comes
/// from outside the data directory so a copied database folder is not enough
/// to read the keys.
pub struct FileStore {
    path: PathBuf,
    key: [u8; 32],
    lock: Mutex<()>,
}

impl FileStore {
    pub fn open(path: &Path, key: [u8; 32]) -> Result<Self, String> {
        let store = Self { path: path.to_path_buf(), key, lock: Mutex::new(()) };
        store.load()?; // Fail at start-up, not at first use, if the key is wrong.
        Ok(store)
    }

    /// Reads a key file holding 32 raw bytes, or 64 hex / 44 base64 characters.
    pub fn read_key_file(path: &Path) -> Result<[u8; 32], String> {
        use base64::{engine::general_purpose::STANDARD, Engine};
        let raw = std::fs::read(path).map_err(|e| format!("Could not read the secret key file {}: {e}", path.display()))?;
        let text = String::from_utf8_lossy(&raw).trim().to_string();
        let bytes = if raw.len() == 32 {
            raw
        } else if text.len() == 64 && text.chars().all(|c| c.is_ascii_hexdigit()) {
            (0..32).map(|i| u8::from_str_radix(&text[i * 2..i * 2 + 2], 16).unwrap()).collect()
        } else {
            STANDARD.decode(&text).map_err(|_| "The secret key file must hold 32 bytes, as raw bytes, hex or base64.".to_string())?
        };
        bytes.try_into().map_err(|_| "The secret key must be exactly 32 bytes.".to_string())
    }

    pub fn generate_key() -> [u8; 32] {
        use rand::RngCore;
        let mut key = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut key);
        key
    }

    fn cipher(&self) -> aes_gcm::Aes256Gcm {
        use aes_gcm::KeyInit;
        aes_gcm::Aes256Gcm::new(&self.key.into())
    }

    fn load(&self) -> Result<BTreeMap<String, String>, String> {
        use aes_gcm::aead::Aead;
        let data = match std::fs::read(&self.path) {
            Ok(d) => d,
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(BTreeMap::new()),
            Err(e) => return Err(format!("Could not read the secret store: {e}")),
        };
        if data.len() < 12 {
            return Err("The secret store file is damaged.".into());
        }
        let (nonce, body) = data.split_at(12);
        let plain = self
            .cipher()
            .decrypt(nonce.into(), body)
            .map_err(|_| "Could not unlock the secret store. Is this the right secret key?".to_string())?;
        serde_json::from_slice(&plain).map_err(|e| format!("The secret store is damaged: {e}"))
    }

    fn save(&self, map: &BTreeMap<String, String>) -> Result<(), String> {
        use aes_gcm::aead::{Aead, AeadCore, OsRng};
        let nonce = aes_gcm::Aes256Gcm::generate_nonce(&mut OsRng);
        let plain = serde_json::to_vec(map).map_err(|e| e.to_string())?;
        let body = self.cipher().encrypt(&nonce, plain.as_slice()).map_err(|_| "Could not encrypt the secret store.".to_string())?;
        let mut out = nonce.to_vec();
        out.extend(body);
        let tmp = self.path.with_extension("tmp");
        write_private(&tmp, &out)?;
        std::fs::rename(&tmp, &self.path).map_err(|e| format!("Could not save the secret store: {e}"))
    }

    fn update(&self, f: impl FnOnce(&mut BTreeMap<String, String>)) -> Result<(), String> {
        let _guard = self.lock.lock().unwrap();
        let mut map = self.load()?;
        f(&mut map);
        self.save(&map)
    }
}

/// Writes a file only its owner can read.
pub fn write_private(path: &Path, bytes: &[u8]) -> Result<(), String> {
    use std::io::Write;
    let mut opts = std::fs::OpenOptions::new();
    opts.write(true).create(true).truncate(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        opts.mode(0o600);
    }
    let mut f = opts.open(path).map_err(|e| format!("Could not write {}: {e}", path.display()))?;
    f.write_all(bytes).map_err(|e| format!("Could not write {}: {e}", path.display()))
}

impl SecretStore for FileStore {
    fn get(&self, name: &str) -> Result<Option<String>, String> {
        let _guard = self.lock.lock().unwrap();
        Ok(self.load()?.get(name).cloned())
    }
    fn set(&self, name: &str, value: &str) -> Result<(), String> {
        self.update(|m| {
            m.insert(name.into(), value.into());
        })
    }
    fn delete(&self, name: &str) -> Result<(), String> {
        self.update(|m| {
            m.remove(name);
        })
    }
}

// ---------- OS keychain (desktop local mode) ----------

#[cfg(feature = "keyring")]
pub use keychain::KeyringStore;

#[cfg(feature = "keyring")]
mod keychain {
    use super::SecretStore;

    const SERVICE: &str = "com.james.commandcenter";

    // Microsoft refresh tokens can exceed what one Windows Credential Manager entry
    // holds (2,560 bytes of UTF-16), so long values are stored in numbered parts,
    // with the base entry holding "parts:N".
    const PART_CHARS: usize = 1000;
    const PARTS_PREFIX: &str = "parts:";

    pub struct KeyringStore;

    fn entry(name: &str, part: Option<usize>) -> Result<keyring::Entry, String> {
        let user = match part {
            None => name.to_string(),
            Some(i) => format!("{name}:{i}"),
        };
        keyring::Entry::new(SERVICE, &user).map_err(|e| e.to_string())
    }

    fn read(entry: &keyring::Entry) -> Result<Option<String>, String> {
        match entry.get_password() {
            Ok(t) => Ok(Some(t)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("Could not read the keychain: {e}")),
        }
    }

    fn part_count(head: &str) -> Option<usize> {
        head.strip_prefix(PARTS_PREFIX).and_then(|n| n.parse().ok())
    }

    pub(super) fn split_parts(value: &str) -> Vec<String> {
        let chars: Vec<char> = value.chars().collect();
        chars.chunks(PART_CHARS).map(|c| c.iter().collect()).collect()
    }

    impl SecretStore for KeyringStore {
        fn get(&self, name: &str) -> Result<Option<String>, String> {
            let Some(head) = read(&entry(name, None)?)? else {
                return Ok(None);
            };
            let Some(count) = part_count(&head) else {
                return Ok(Some(head));
            };
            let mut value = String::new();
            for i in 0..count {
                match read(&entry(name, Some(i))?)? {
                    Some(part) => value.push_str(&part),
                    None => return Ok(None), // Incomplete; treat as missing.
                }
            }
            Ok(Some(value))
        }

        fn set(&self, name: &str, value: &str) -> Result<(), String> {
            let save = |e: keyring::Error| format!("Could not save to the keychain: {e}");
            self.delete(name)?;
            if value.chars().count() <= PART_CHARS {
                return entry(name, None)?.set_password(value).map_err(save);
            }
            let parts = split_parts(value);
            for (i, part) in parts.iter().enumerate() {
                entry(name, Some(i))?.set_password(part).map_err(save)?;
            }
            entry(name, None)?.set_password(&format!("{PARTS_PREFIX}{}", parts.len())).map_err(save)
        }

        fn delete(&self, name: &str) -> Result<(), String> {
            let remove = |entry: keyring::Entry| match entry.delete_credential() {
                Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
                Err(e) => Err(e.to_string()),
            };
            let head = entry(name, None)?;
            if let Some(count) = read(&head)?.as_deref().and_then(part_count) {
                for i in 0..count {
                    remove(entry(name, Some(i))?)?;
                }
            }
            remove(head)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "keyring")]
    #[test]
    fn long_values_split_into_small_parts() {
        let token = "a".repeat(2500) + "é";
        let parts = keychain::split_parts(&token);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= 1000));
        assert_eq!(parts.concat(), token);
        assert!(keychain::split_parts("").is_empty());
    }

    /// Touches the real OS keychain; run with `cargo test --features keyring -- --ignored`.
    #[cfg(feature = "keyring")]
    #[test]
    #[ignore]
    fn long_token_round_trips_through_the_os_keychain() {
        let name = mail_token_name("roundtrip-test@example.invalid");
        let store = KeyringStore;
        let token = "t".repeat(4000);
        store.set(&name, &token).unwrap();
        assert_eq!(store.get(&name).unwrap().as_deref(), Some(token.as_str()));
        store.set(&name, "short").unwrap();
        assert_eq!(store.get(&name).unwrap().as_deref(), Some("short"));
        store.delete(&name).unwrap();
        assert_eq!(store.get(&name).unwrap(), None);
    }

    fn temp_dir() -> PathBuf {
        let dir = std::env::temp_dir().join(format!("cc-secrets-{}", rand::random::<u64>()));
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    #[test]
    fn file_store_round_trips_and_is_encrypted() {
        let dir = temp_dir();
        let path = dir.join("secrets.enc");
        let key = FileStore::generate_key();
        let store = FileStore::open(&path, key).unwrap();
        store.set("atera-api-key", "super-secret-value").unwrap();
        store.set("ms-refresh:a@b.c", &"r".repeat(5000)).unwrap();
        assert_eq!(store.get("atera-api-key").unwrap().as_deref(), Some("super-secret-value"));

        let raw = std::fs::read(&path).unwrap();
        assert!(!String::from_utf8_lossy(&raw).contains("super-secret-value"));

        let reopened = FileStore::open(&path, key).unwrap();
        assert_eq!(reopened.get("ms-refresh:a@b.c").unwrap().unwrap().len(), 5000);
        reopened.delete("atera-api-key").unwrap();
        assert_eq!(reopened.get("atera-api-key").unwrap(), None);

        assert!(FileStore::open(&path, FileStore::generate_key()).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn key_file_accepts_hex_and_base64() {
        let dir = temp_dir();
        let hex = dir.join("hex");
        std::fs::write(&hex, format!("{}\n", "ab".repeat(32))).unwrap();
        assert_eq!(FileStore::read_key_file(&hex).unwrap(), [0xab; 32]);
        let b64 = dir.join("b64");
        std::fs::write(&b64, "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA=").unwrap();
        assert_eq!(FileStore::read_key_file(&b64).unwrap(), [0; 32]);
        let bad = dir.join("bad");
        std::fs::write(&bad, "short").unwrap();
        assert!(FileStore::read_key_file(&bad).is_err());
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn empty_value_deletes() {
        let s = Secrets::new(MemoryStore::default());
        s.set("x", " v ").unwrap();
        assert_eq!(s.get("x").unwrap().as_deref(), Some("v"));
        s.set("x", "  ").unwrap();
        assert_eq!(s.get("x").unwrap(), None);
    }
}
