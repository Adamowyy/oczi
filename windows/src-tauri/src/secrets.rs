// API keys live in the Windows Credential Manager, never on disk and never in
// the front end, the island can only ask whether a key is present.

use keyring::Entry;

const SERVICE: &str = "oczi";
/// Service name used by earlier builds. Read once, on startup, by `migrate`.
const LEGACY_SERVICE: &str = "fr.louisraille.coucou";

/// Every key Oczi may store. Anything outside this list is refused.
pub const KNOWN_KEYS: &[&str] = &[
    "deepseek-api-key",
    "n8n-url",
    "n8n-api-key",
    "vercel-token",
    "github-token",
    "stripe-api-key",
    "resend-api-key",
    "notion-api-key",
    "calcom-api-key",
];

fn entry(key: &str) -> Option<Entry> {
    if !KNOWN_KEYS.contains(&key) {
        return None;
    }
    Entry::new(SERVICE, key).ok()
}

pub fn get(key: &str) -> Option<String> {
    entry(key)?.get_password().ok().filter(|v| !v.is_empty())
}

pub fn set(key: &str, value: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    if value.is_empty() {
        let _ = entry.delete_credential();
        return Ok(());
    }
    entry.set_password(value).map_err(|e| e.to_string())
}

pub fn clear(key: &str) -> Result<(), String> {
    let entry = entry(key).ok_or_else(|| format!("unknown key {key}"))?;
    match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn present(key: &str) -> bool {
    get(key).is_some()
}

/// Moves keys left under the old service name to the current one, so an existing
/// install keeps its keys. Only copies; a legacy entry is deleted once its value
/// is safely stored again. Returns how many moved.
pub fn migrate() -> usize {
    let mut moved = 0;
    for key in KNOWN_KEYS {
        if get(key).is_some() {
            continue;
        }
        let Ok(old) = Entry::new(LEGACY_SERVICE, key) else {
            continue;
        };
        let Ok(value) = old.get_password() else {
            continue;
        };
        if value.is_empty() {
            continue;
        }
        if set(key, &value).is_ok() {
            let _ = old.delete_credential();
            moved += 1;
        }
    }
    moved
}
