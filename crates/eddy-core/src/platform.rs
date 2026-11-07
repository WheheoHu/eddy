/// Services the host platform provides to the engine. Secrets go to the
/// platform keychain; the engine never stores them in its database.
pub trait Platform: Send + Sync {
    fn secret_get(&self, key: &str) -> Option<String>;
    fn secret_set(&self, key: &str, value: &str) -> bool;
    fn secret_delete(&self, key: &str);
    fn log(&self, _level: LogLevel, _message: &str) {}
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LogLevel {
    Debug,
    Info,
    Warn,
    Error,
}

/// In-memory platform for tests and headless tools.
#[derive(Default)]
pub struct MemoryPlatform {
    secrets: parking_lot::Mutex<std::collections::HashMap<String, String>>,
}

impl Platform for MemoryPlatform {
    fn secret_get(&self, key: &str) -> Option<String> {
        self.secrets.lock().get(key).cloned()
    }

    fn secret_set(&self, key: &str, value: &str) -> bool {
        self.secrets
            .lock()
            .insert(key.to_string(), value.to_string());
        true
    }

    fn secret_delete(&self, key: &str) {
        self.secrets.lock().remove(key);
    }
}

pub fn account_secret_key(account_id: &str) -> String {
    format!("account:{account_id}")
}
