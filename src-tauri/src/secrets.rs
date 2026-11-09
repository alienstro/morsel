//! API keys. They live only in the macOS Keychain.

use crate::types::{MorselError, Result};
use std::collections::HashMap;
use std::sync::Mutex;

pub const SERVICE: &str = "app.morsel";
pub const NOTES_KEY: &str = "notes_api_key";
pub const TRANSCRIBE_KEY: &str = "transcribe_api_key";
pub const COMPOSIO_KEY: &str = "composio_api_key";

/// Every Keychain account that Morsel uses.
pub const ACCOUNTS: &[&str] = &[NOTES_KEY, TRANSCRIBE_KEY, COMPOSIO_KEY];

/// True for an account that Morsel uses.
pub fn is_known_account(account: &str) -> bool {
    ACCOUNTS.contains(&account)
}

pub trait SecretStore: Send + Sync {
    fn get(&self, account: &str) -> Result<Option<String>>;
    fn set(&self, account: &str, value: &str) -> Result<()>;
    fn delete(&self, account: &str) -> Result<()>;
}

/// The real store: the macOS Keychain, service `app.morsel`.
pub struct KeychainStore;

impl KeychainStore {
    fn entry(account: &str) -> Result<keyring::Entry> {
        keyring::Entry::new(SERVICE, account).map_err(|e| MorselError::Keychain(e.to_string()))
    }
}

impl SecretStore for KeychainStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        match Self::entry(account)?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(e) => Err(MorselError::Keychain(e.to_string())),
        }
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        Self::entry(account)?
            .set_password(value)
            .map_err(|e| MorselError::Keychain(e.to_string()))
    }

    fn delete(&self, account: &str) -> Result<()> {
        match Self::entry(account)?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(MorselError::Keychain(e.to_string())),
        }
    }
}

/// An in-memory store for tests.
#[derive(Default)]
pub struct MemoryStore {
    values: Mutex<HashMap<String, String>>,
}

impl SecretStore for MemoryStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        Ok(self.values.lock().expect("lock").get(account).cloned())
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        self.values
            .lock()
            .expect("lock")
            .insert(account.to_owned(), value.to_owned());
        Ok(())
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.values.lock().expect("lock").remove(account);
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn memory_store_set_get_delete() {
        let store = MemoryStore::default();
        assert_eq!(store.get(NOTES_KEY).unwrap(), None);
        store.set(NOTES_KEY, "abc").unwrap();
        assert_eq!(store.get(NOTES_KEY).unwrap().as_deref(), Some("abc"));
        store.delete(NOTES_KEY).unwrap();
        assert_eq!(store.get(NOTES_KEY).unwrap(), None);
    }

    #[test]
    fn only_known_accounts_are_valid() {
        assert!(is_known_account("notes_api_key"));
        assert!(is_known_account("transcribe_api_key"));
        assert!(!is_known_account("other"));
    }
}
