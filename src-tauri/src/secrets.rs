//! API keys, in a file that only the owner can read. Older versions kept them in the macOS
//! Keychain, and the first start moves them to the file (see `open_file_store`).

use crate::types::{MorselError, Result};
use std::collections::HashMap;
use std::path::{Path, PathBuf};
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

/// The macOS Keychain, service `app.morsel`. Only the one-time move of old keys reads it.
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

/// A JSON file that only the owner can read (mode 0600). Morsel has an ad-hoc signature, which
/// changes with each build, so the Keychain asks for the password again after each update.
pub struct FileStore {
    path: PathBuf,
    lock: Mutex<()>,
}

impl FileStore {
    pub fn new(path: &Path) -> Self {
        Self {
            path: path.to_owned(),
            lock: Mutex::new(()),
        }
    }

    fn load(&self) -> Result<HashMap<String, String>> {
        match std::fs::read_to_string(&self.path) {
            Ok(raw) => serde_json::from_str(&raw)
                .map_err(|e| MorselError::Io(format!("the secrets file is not valid: {e}"))),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(HashMap::new()),
            Err(e) => Err(e.into()),
        }
    }

    /// Writes a temporary file and renames it, so a crash never leaves a half-written file.
    fn save(&self, values: &HashMap<String, String>) -> Result<()> {
        use std::io::Write;
        use std::os::unix::fs::OpenOptionsExt;
        let raw = serde_json::to_vec(values).map_err(|e| MorselError::Io(e.to_string()))?;
        let tmp = self.path.with_extension("json.tmp");
        let mut file = std::fs::OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(true)
            .mode(0o600)
            .open(&tmp)?;
        file.write_all(&raw)?;
        file.sync_all()?;
        std::fs::rename(&tmp, &self.path)?;
        Ok(())
    }

    fn change(&self, edit: impl FnOnce(&mut HashMap<String, String>)) -> Result<()> {
        let _guard = self.lock.lock().expect("lock");
        let mut values = self.load()?;
        edit(&mut values);
        self.save(&values)
    }
}

impl SecretStore for FileStore {
    fn get(&self, account: &str) -> Result<Option<String>> {
        let _guard = self.lock.lock().expect("lock");
        Ok(self.load()?.remove(account))
    }

    fn set(&self, account: &str, value: &str) -> Result<()> {
        self.change(|values| {
            values.insert(account.to_owned(), value.to_owned());
        })
    }

    fn delete(&self, account: &str) -> Result<()> {
        self.change(|values| {
            values.remove(account);
        })
    }
}

/// Opens the secrets file. When the file does not exist yet, it first copies the keys of
/// older versions from the Keychain, one time. A key that the Keychain refuses is skipped.
/// The Keychain copies stay, so an older build still finds them.
pub fn open_file_store(path: &Path, keychain: &dyn SecretStore) -> FileStore {
    let store = FileStore::new(path);
    if path.exists() {
        return store;
    }
    let mut moved = HashMap::new();
    for account in ACCOUNTS {
        match keychain.get(account) {
            Ok(Some(value)) => {
                moved.insert((*account).to_owned(), value);
            }
            Ok(None) => {}
            Err(err) => eprintln!("cannot copy {account} from the Keychain: {err}"),
        }
    }
    // Write the file even when it is empty, so that the Keychain is read only one time.
    if let Err(err) = store.change(|values| values.extend(moved)) {
        eprintln!("cannot write the secrets file: {err}");
    }
    store
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
    fn file_store_keeps_keys_after_a_restart() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        assert_eq!(FileStore::new(&path).get(NOTES_KEY).unwrap(), None);
        FileStore::new(&path).set(NOTES_KEY, "abc").unwrap();
        FileStore::new(&path).set(COMPOSIO_KEY, "ak_1").unwrap();
        let reopened = FileStore::new(&path);
        assert_eq!(reopened.get(NOTES_KEY).unwrap().as_deref(), Some("abc"));
        reopened.delete(NOTES_KEY).unwrap();
        assert_eq!(FileStore::new(&path).get(NOTES_KEY).unwrap(), None);
        assert_eq!(
            FileStore::new(&path).get(COMPOSIO_KEY).unwrap().as_deref(),
            Some("ak_1")
        );
    }

    #[test]
    fn file_store_file_is_readable_only_by_the_owner() {
        use std::os::unix::fs::PermissionsExt;
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        FileStore::new(&path).set(NOTES_KEY, "abc").unwrap();
        let mode = std::fs::metadata(&path).unwrap().permissions().mode();
        assert_eq!(mode & 0o777, 0o600);
    }

    #[test]
    fn only_known_accounts_are_valid() {
        assert!(is_known_account("notes_api_key"));
        assert!(is_known_account("transcribe_api_key"));
        assert!(!is_known_account("other"));
    }

    /// A Keychain stand-in that counts reads, and fails for one account (the user pressed Deny).
    #[derive(Default)]
    struct CountingStore {
        inner: MemoryStore,
        reads: std::sync::atomic::AtomicUsize,
        deny: Option<&'static str>,
    }

    impl SecretStore for CountingStore {
        fn get(&self, account: &str) -> Result<Option<String>> {
            self.reads.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
            if self.deny == Some(account) {
                return Err(MorselError::Keychain("denied".into()));
            }
            self.inner.get(account)
        }
        fn set(&self, account: &str, value: &str) -> Result<()> {
            self.inner.set(account, value)
        }
        fn delete(&self, account: &str) -> Result<()> {
            self.inner.delete(account)
        }
    }

    #[test]
    fn first_open_moves_the_keychain_keys_into_the_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let keychain = CountingStore::default();
        keychain.set(NOTES_KEY, "sk-1").unwrap();
        keychain.set(COMPOSIO_KEY, "ak_1").unwrap();

        let store = open_file_store(&path, &keychain);

        assert_eq!(store.get(NOTES_KEY).unwrap().as_deref(), Some("sk-1"));
        assert_eq!(store.get(COMPOSIO_KEY).unwrap().as_deref(), Some("ak_1"));
        assert_eq!(store.get(TRANSCRIBE_KEY).unwrap(), None);
        assert!(path.exists());
    }

    #[test]
    fn later_opens_do_not_read_the_keychain() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let keychain = CountingStore::default();
        open_file_store(&path, &keychain);
        let reads = keychain.reads.load(std::sync::atomic::Ordering::SeqCst);

        open_file_store(&path, &keychain);

        assert_eq!(keychain.reads.load(std::sync::atomic::Ordering::SeqCst), reads);
    }

    #[test]
    fn a_denied_keychain_read_skips_only_that_key() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("secrets.json");
        let keychain = CountingStore {
            deny: Some(NOTES_KEY),
            ..Default::default()
        };
        keychain.set(COMPOSIO_KEY, "ak_1").unwrap();

        let store = open_file_store(&path, &keychain);

        assert_eq!(store.get(NOTES_KEY).unwrap(), None);
        assert_eq!(store.get(COMPOSIO_KEY).unwrap().as_deref(), Some("ak_1"));
    }
}
