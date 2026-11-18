//! Windows platform services: secrets in Credential Manager, stderr logging.
//!
//! Credential Manager cannot be enumerated through the `keyring` crate, so
//! `secret_set` also records the key name in an index file inside the data
//! directory. `secret_clear` (sign out and erase) walks that index to delete
//! every entry it wrote. The index never holds the secrets themselves.

use std::collections::HashSet;
use std::path::PathBuf;

use eddy_core::{LogLevel, Platform};

pub struct WindowsPlatform {
    index: PathBuf,
}

impl WindowsPlatform {
    pub fn new(data_dir: &std::path::Path) -> Self {
        let _ = std::fs::create_dir_all(data_dir);
        WindowsPlatform {
            index: data_dir.join("secret-keys.json"),
        }
    }

    fn entry(&self, key: &str) -> Option<keyring::Entry> {
        keyring::Entry::new("Eddy", key).ok()
    }

    fn known_keys(&self) -> HashSet<String> {
        std::fs::read(&self.index)
            .ok()
            .and_then(|b| serde_json::from_slice(&b).ok())
            .unwrap_or_default()
    }

    fn remember(&self, key: &str) {
        let mut keys = self.known_keys();
        if keys.insert(key.to_string()) {
            if let Ok(bytes) = serde_json::to_vec(&keys) {
                let _ = std::fs::write(&self.index, bytes);
            }
        }
    }
}

impl Platform for WindowsPlatform {
    fn secret_get(&self, key: &str) -> Option<String> {
        match self.entry(key) {
            Some(entry) => entry.get_password().ok(),
            None => None,
        }
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        let Some(entry) = self.entry(key) else {
            return false;
        };
        match entry.set_password(value) {
            Ok(()) => {
                self.remember(key);
                true
            }
            Err(_) => false,
        }
    }

    fn secret_delete(&self, key: &str) {
        if let Some(entry) = self.entry(key) {
            let _ = entry.delete_credential();
        }
        let mut keys = self.known_keys();
        if keys.remove(key) {
            if let Ok(bytes) = serde_json::to_vec(&keys) {
                let _ = std::fs::write(&self.index, bytes);
            }
        }
    }

    fn secret_clear(&self) {
        for key in self.known_keys() {
            if let Some(entry) = self.entry(&key) {
                let _ = entry.delete_credential();
            }
        }
        let _ = std::fs::remove_file(&self.index);
    }

    fn log(&self, level: LogLevel, message: &str) {
        eprintln!("[{level:?}] {message}");
    }
}
