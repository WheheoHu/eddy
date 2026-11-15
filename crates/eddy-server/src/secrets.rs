use std::collections::HashMap;
use std::path::PathBuf;

use aes_gcm::aead::{Aead, KeyInit, Payload};
use aes_gcm::{Aes256Gcm, Nonce};
use base64::Engine as _;
use base64::engine::general_purpose::STANDARD;
use eddy_core::{LogLevel, Platform};
use parking_lot::Mutex;

/// Server-side keychain: secrets (the FreshRSS password) encrypted with
/// AES-256-GCM under the master key from `EDDY_MASTER_KEY`, stored in
/// `secrets.json` in the data directory. Each value is bound to its key name
/// as associated data.
pub struct EncryptedSecrets {
    path: PathBuf,
    cipher: Aes256Gcm,
    values: Mutex<HashMap<String, String>>,
}

impl EncryptedSecrets {
    pub fn open(path: PathBuf, master_key: &[u8; 32]) -> std::io::Result<Self> {
        let values = match std::fs::read(&path) {
            Ok(bytes) => serde_json::from_slice(&bytes).unwrap_or_default(),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => HashMap::new(),
            Err(e) => return Err(e),
        };
        let cipher = Aes256Gcm::new_from_slice(master_key).expect("32-byte key");
        Ok(EncryptedSecrets {
            path,
            cipher,
            values: Mutex::new(values),
        })
    }

    fn persist(&self, values: &HashMap<String, String>) -> bool {
        let tmp = self.path.with_extension("tmp");
        let Ok(bytes) = serde_json::to_vec(values) else {
            return false;
        };
        std::fs::write(&tmp, bytes).is_ok() && std::fs::rename(&tmp, &self.path).is_ok()
    }
}

fn random_bytes<const N: usize>() -> [u8; N] {
    let mut out = [0u8; N];
    getrandom::fill(&mut out).expect("system randomness");
    out
}

impl Platform for EncryptedSecrets {
    fn secret_get(&self, key: &str) -> Option<String> {
        let sealed = self.values.lock().get(key).cloned()?;
        let raw = STANDARD.decode(sealed).ok()?;
        if raw.len() < 12 {
            return None;
        }
        let (nonce, ciphertext) = raw.split_at(12);
        let plain = self
            .cipher
            .decrypt(
                Nonce::from_slice(nonce),
                Payload {
                    msg: ciphertext,
                    aad: key.as_bytes(),
                },
            )
            .ok()?;
        String::from_utf8(plain).ok()
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        let nonce: [u8; 12] = random_bytes();
        let Ok(ciphertext) = self.cipher.encrypt(
            Nonce::from_slice(&nonce),
            Payload {
                msg: value.as_bytes(),
                aad: key.as_bytes(),
            },
        ) else {
            return false;
        };
        let mut sealed = nonce.to_vec();
        sealed.extend(ciphertext);
        let mut values = self.values.lock();
        values.insert(key.to_string(), STANDARD.encode(sealed));
        self.persist(&values)
    }

    fn secret_delete(&self, key: &str) {
        let mut values = self.values.lock();
        if values.remove(key).is_some() {
            self.persist(&values);
        }
    }

    fn secret_clear(&self) {
        let mut values = self.values.lock();
        values.clear();
        self.persist(&values);
    }

    fn log(&self, level: LogLevel, message: &str) {
        match level {
            LogLevel::Error => tracing::error!("{message}"),
            LogLevel::Warn => tracing::warn!("{message}"),
            LogLevel::Info => tracing::info!("{message}"),
            LogLevel::Debug => tracing::debug!("{message}"),
        }
    }
}

/// Random token for a paired device (256 bits, hex).
pub fn new_token() -> String {
    let bytes: [u8; 32] = random_bytes();
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}

/// Human-friendly one-time pairing code, e.g. `K7QM-2XPA`.
pub fn new_pairing_code() -> String {
    const ALPHABET: &[u8] = b"ABCDEFGHJKLMNPQRSTUVWXYZ23456789";
    let bytes: [u8; 8] = random_bytes();
    let chars: String = bytes
        .iter()
        .map(|b| ALPHABET[(*b as usize) % ALPHABET.len()] as char)
        .collect();
    format!("{}-{}", &chars[..4], &chars[4..])
}

pub fn parse_master_key(value: &str) -> Result<[u8; 32], String> {
    let bytes = STANDARD.decode(value.trim()).map_err(|_| {
        "EDDY_MASTER_KEY 必须是 base64 编码的 32 字节随机值（openssl rand -base64 32）".to_string()
    })?;
    bytes
        .try_into()
        .map_err(|_| "EDDY_MASTER_KEY 解码后必须正好 32 字节".to_string())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn secrets_round_trip_encrypted_on_disk() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let key = [7u8; 32];
        let s = EncryptedSecrets::open(path.clone(), &key).unwrap();
        assert!(s.secret_set("account:x", "hunter2"));
        assert_eq!(s.secret_get("account:x").as_deref(), Some("hunter2"));
        let on_disk = std::fs::read_to_string(&path).unwrap();
        assert!(!on_disk.contains("hunter2"));
        // Reopen with the same key works; another key cannot decrypt.
        assert_eq!(
            EncryptedSecrets::open(path.clone(), &key)
                .unwrap()
                .secret_get("account:x")
                .as_deref(),
            Some("hunter2")
        );
        assert!(
            EncryptedSecrets::open(path.clone(), &[8u8; 32])
                .unwrap()
                .secret_get("account:x")
                .is_none()
        );
        // Values are bound to their key name.
        let raw: HashMap<String, String> = serde_json::from_str(&on_disk).unwrap();
        let mut swapped = raw.clone();
        swapped.insert("other".into(), raw["account:x"].clone());
        std::fs::write(&path, serde_json::to_vec(&swapped).unwrap()).unwrap();
        assert!(
            EncryptedSecrets::open(path, &key)
                .unwrap()
                .secret_get("other")
                .is_none()
        );
    }

    #[test]
    fn tokens_and_codes_look_random() {
        assert_eq!(new_token().len(), 64);
        assert_ne!(new_token(), new_token());
        let code = new_pairing_code();
        assert_eq!(code.len(), 9);
        assert!(parse_master_key("short").is_err());
        assert!(parse_master_key(&STANDARD.encode([1u8; 32])).is_ok());
    }
}
