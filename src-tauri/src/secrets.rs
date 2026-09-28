//! API keys live in the OS keychain (macOS Keychain, Windows Credential Manager,
//! Secret Service on Linux), never in the database or config files.

const SERVICE: &str = "com.james.commandcenter";
const CLAUDE_KEY: &str = "anthropic-api-key";

fn entry() -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, CLAUDE_KEY).map_err(|e| e.to_string())
}

pub fn get_api_key() -> Result<Option<String>, String> {
    // A key in the environment wins, which is handy for development.
    if let Ok(k) = std::env::var("ANTHROPIC_API_KEY") {
        if !k.trim().is_empty() {
            return Ok(Some(k));
        }
    }
    match entry()?.get_password() {
        Ok(k) => Ok(Some(k)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the keychain: {e}")),
    }
}

fn mail_entry(email: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new(SERVICE, &format!("ms-refresh:{}", email.to_lowercase())).map_err(|e| e.to_string())
}

/// The Microsoft refresh token for a connected inbox.
pub fn get_mail_token(email: &str) -> Result<Option<String>, String> {
    match mail_entry(email)?.get_password() {
        Ok(t) => Ok(Some(t)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(e) => Err(format!("Could not read the keychain: {e}")),
    }
}

pub fn set_mail_token(email: &str, token: &str) -> Result<(), String> {
    mail_entry(email)?.set_password(token).map_err(|e| format!("Could not save to the keychain: {e}"))
}

pub fn delete_mail_token(email: &str) -> Result<(), String> {
    match mail_entry(email)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    }
}

pub fn set_api_key(key: &str) -> Result<(), String> {
    let key = key.trim();
    if key.is_empty() {
        return match entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(e) => Err(e.to_string()),
        };
    }
    entry()?.set_password(key).map_err(|e| format!("Could not save to the keychain: {e}"))
}
