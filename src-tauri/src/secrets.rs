//! OS keychain-backed storage for LLM API keys.
//!
//! Secrets must never be serialized into the settings store JSON. The
//! persistence layer (`settings::write_settings`) strips the key fields from
//! the JSON at runtime, and this module is the single source of truth for
//! their values.
//!
//! Keychain access is cached in-process: `settings::get_settings` is called
//! once per transcription, and a keychain read on every call would be both
//! wasteful and user-visible on systems that prompt for keychain access.

use std::collections::HashMap;
use std::sync::atomic::{AtomicU8, Ordering};
use std::sync::{Arc, RwLock};

use log::{debug, warn};
use once_cell::sync::Lazy;

/// Keychain service name. Must stay stable across releases — changing it
/// orphans every stored secret.
pub const KEYRING_SERVICE: &str = "com.melvynx.parler";

/// Logical name of the Gemini API key inside the secret store.
pub const GEMINI_API_KEY: &str = "gemini_api_key";

/// Logical name of a post-processing provider's API key.
pub fn provider_api_key_name(provider_id: &str) -> String {
    format!("llm_api_key:{provider_id}")
}

/// Abstraction over the OS keychain so tests can run against an in-memory store.
pub trait SecretStore: Send + Sync {
    fn get(&self, key: &str) -> Result<Option<String>, String>;
    fn set(&self, key: &str, value: &str) -> Result<(), String>;
    fn delete(&self, key: &str) -> Result<(), String>;
}

/// Real OS keychain backend (macOS Keychain, Windows Credential Manager,
/// Secret Service on Linux).
pub struct OsKeyringStore;

impl OsKeyringStore {
    fn entry(key: &str) -> Result<keyring::Entry, String> {
        keyring::Entry::new(KEYRING_SERVICE, key)
            .map_err(|e| format!("keychain entry '{key}' unavailable: {e}"))
    }
}

impl SecretStore for OsKeyringStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        match Self::entry(key)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(format!("keychain read failed for '{key}': {e}")),
        }
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        Self::entry(key)?
            .set_password(value)
            .map_err(|e| format!("keychain write failed for '{key}': {e}"))
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        match Self::entry(key)?.delete_credential() {
            Ok(()) => Ok(()),
            // Deleting an absent secret is success.
            Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keychain delete failed for '{key}': {e}")),
        }
    }
}

/// In-memory store used by tests and by the plaintext fallback path.
#[derive(Default)]
pub struct MemoryStore {
    entries: RwLock<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, key: &str) -> Result<Option<String>, String> {
        Ok(self.entries.read().unwrap().get(key).cloned())
    }

    fn set(&self, key: &str, value: &str) -> Result<(), String> {
        self.entries
            .write()
            .unwrap()
            .insert(key.to_string(), value.to_string());
        Ok(())
    }

    fn delete(&self, key: &str) -> Result<(), String> {
        self.entries.write().unwrap().remove(key);
        Ok(())
    }
}

/// Active backend. Swapped for a mock in tests.
static STORE: Lazy<RwLock<Arc<dyn SecretStore>>> =
    Lazy::new(|| RwLock::new(Arc::new(OsKeyringStore)));

/// Cache of secret values read from / written to the backend.
/// `Some(None)` = known absent, `None` = not loaded yet.
static CACHE: Lazy<RwLock<HashMap<String, Option<String>>>> =
    Lazy::new(|| RwLock::new(HashMap::new()));

/// 0 = unknown, 1 = available, 2 = unavailable (decided once per process).
static AVAILABLE: AtomicU8 = AtomicU8::new(0);

fn store() -> Arc<dyn SecretStore> {
    STORE.read().unwrap().clone()
}

/// Replaces the backend (tests only).
#[cfg(test)]
pub fn set_store_for_tests(new_store: Arc<dyn SecretStore>) {
    *STORE.write().unwrap() = new_store;
    CACHE.write().unwrap().clear();
    AVAILABLE.store(0, Ordering::Relaxed);
}

/// Clears the in-process cache (tests only).
#[cfg(test)]
pub fn clear_cache_for_tests() {
    CACHE.write().unwrap().clear();
}

/// Reads a secret, going through the cache. `None` when absent or when the
/// backend is explicitly unavailable.
pub fn get_secret(key: &str) -> Option<String> {
    {
        let cache = CACHE.read().unwrap();
        if let Some(cached) = cache.get(key) {
            return cached.clone();
        }
    }

    let value = match store().get(key) {
        Ok(value) => value,
        Err(e) => {
            warn!("Secret read failed for '{}': {}", key, e);
            return None;
        }
    };
    CACHE
        .write()
        .unwrap()
        .insert(key.to_string(), value.clone());
    value
}

/// Writes a secret to the backend and refreshes the cache.
/// An empty value deletes the secret (matches the UI's empty-string semantics).
pub fn set_secret(key: &str, value: &str) -> Result<(), String> {
    if value.is_empty() {
        return delete_secret(key);
    }
    store().set(key, value)?;
    CACHE
        .write()
        .unwrap()
        .insert(key.to_string(), Some(value.to_string()));
    debug!("Secret '{}' stored in OS keychain", key);
    Ok(())
}

/// Deletes a secret from the backend and refreshes the cache.
pub fn delete_secret(key: &str) -> Result<(), String> {
    store().delete(key)?;
    CACHE.write().unwrap().insert(key.to_string(), None);
    debug!("Secret '{}' removed from OS keychain", key);
    Ok(())
}

/// Reads straight from the backend, bypassing the cache. Used by the legacy
/// migration to verify what the keychain actually stored.
pub fn read_through(key: &str) -> Result<Option<String>, String> {
    store().get(key)
}

/// Returns the value for every requested key, going through the cache.
pub fn get_many(keys: &[String]) -> HashMap<String, String> {
    let mut out = HashMap::new();
    for key in keys {
        if let Some(value) = get_secret(key) {
            out.insert(key.clone(), value);
        }
    }
    out
}

/// Probes the backend once per process. `false` means the keychain is
/// unusable (e.g. Linux without Secret Service) and the plaintext fallback
/// must take over.
pub fn is_available() -> bool {
    match AVAILABLE.load(Ordering::Relaxed) {
        1 => return true,
        2 => return false,
        _ => {}
    }

    // A read of a non-existent sentinel proves the backend answers without
    // mutating anything.
    let available = match store().get("availability-probe") {
        Ok(_) => true,
        Err(e) => {
            warn!(
                "OS keychain unavailable, LLM API keys will be stored in the settings file: {}",
                e
            );
            false
        }
    };
    AVAILABLE.store(if available { 1 } else { 2 }, Ordering::Relaxed);
    available
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Mutex;

    /// The store/cache are process globals; tests must not interleave.
    static TEST_LOCK: Mutex<()> = Mutex::new(());

    /// Serializes a test and installs a fresh in-memory backend.
    fn test_guard() -> std::sync::MutexGuard<'static, ()> {
        let guard = TEST_LOCK.lock().unwrap_or_else(|e| e.into_inner());
        *STORE.write().unwrap() = Arc::new(MemoryStore::default());
        CACHE.write().unwrap().clear();
        AVAILABLE.store(0, Ordering::Relaxed);
        guard
    }

    #[test]
    fn set_get_delete_roundtrip() {
        let _g = test_guard();
        set_secret("roundtrip-key", "secret-value").unwrap();
        assert_eq!(get_secret("roundtrip-key").as_deref(), Some("secret-value"));

        delete_secret("roundtrip-key").unwrap();
        assert_eq!(get_secret("roundtrip-key"), None);
    }

    #[test]
    fn empty_value_deletes() {
        let _g = test_guard();
        set_secret("empty-key", "value").unwrap();
        set_secret("empty-key", "").unwrap();
        assert_eq!(get_secret("empty-key"), None);
    }

    #[test]
    fn deletion_of_absent_key_is_ok() {
        let _g = test_guard();
        delete_secret("never-set").unwrap();
        assert_eq!(get_secret("never-set"), None);
    }

    #[test]
    fn cache_avoids_backend_reads() {
        let _g = test_guard();
        let mem = Arc::new(MemoryStore::default());
        set_store_for_tests(mem.clone());
        mem.set("cached-key", "v1").unwrap();

        assert_eq!(get_secret("cached-key").as_deref(), Some("v1"));

        // Mutate the backend behind the cache's back: the second read must
        // still serve the cached value.
        mem.set("cached-key", "v2").unwrap();
        assert_eq!(
            get_secret("cached-key").as_deref(),
            Some("v1"),
            "cache must serve the first read, not re-hit the backend"
        );
    }

    #[test]
    fn availability_probe_uses_backend() {
        let _g = test_guard();
        assert!(is_available());
    }
}
