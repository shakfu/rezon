// Secrets storage backed by the OS-native credential store
// (Keychain / Credential Manager / secret-service via the `keyring`
// crate). Used today for the user-configurable "Other" provider's
// API key, which previously lived only in React state and didn't
// survive a restart. The Tauri command surface is generic over
// `account`, so future secrets (extra cloud-provider overrides,
// future agent-tool credentials, etc.) can use the same plumbing
// without touching this file.
//
// Service identifier is `rezon-tui` to match the existing
// ProjectDirs application id; namespaces our entries away from any
// other app a user has on their machine. The keyring's `service`
// + `user` (account) pair is a primary key — different `account`
// values are independent slots in the same service.

use keyring::Entry;

/// Service identifier passed to `keyring::Entry::new`. Stable
/// across versions — changing it would orphan every previously-
/// saved secret.
const SERVICE: &str = "rezon-tui";

/// Read a secret. Returns `Ok(None)` when no entry exists for the
/// account (the keyring crate signals this via `NoEntry`), and an
/// error string for unexpected failures (corrupt store, denied
/// access). Callers should treat `None` as "the user hasn't saved
/// one yet."
#[tauri::command]
pub fn keychain_get(account: String) -> Result<Option<String>, String> {
    let entry = Entry::new(SERVICE, &account).map_err(|e| format!("keyring: {e}"))?;
    match entry.get_password() {
        Ok(s) => Ok(Some(s)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("keyring get {account}: {e}")),
    }
}

/// Write (or overwrite) a secret. Empty values are treated as a
/// delete so the UI doesn't have to thread a separate "clear" code
/// path — typing into the field and erasing it intuitively removes
/// the saved value.
#[tauri::command]
pub fn keychain_set(account: String, value: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE, &account).map_err(|e| format!("keyring: {e}"))?;
    if value.is_empty() {
        // `delete_credential` errors with NoEntry when there's
        // nothing to delete; surface as Ok(()) so the caller's
        // happy path doesn't have to distinguish.
        return match entry.delete_credential() {
            Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(format!("keyring delete {account}: {e}")),
        };
    }
    entry
        .set_password(&value)
        .map_err(|e| format!("keyring set {account}: {e}"))
}

/// Explicit delete. Idempotent on missing entries (same NoEntry-as-
/// success behaviour as the empty-set path).
#[tauri::command]
pub fn keychain_delete(account: String) -> Result<(), String> {
    let entry = Entry::new(SERVICE, &account).map_err(|e| format!("keyring: {e}"))?;
    match entry.delete_credential() {
        Ok(_) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(format!("keyring delete {account}: {e}")),
    }
}
