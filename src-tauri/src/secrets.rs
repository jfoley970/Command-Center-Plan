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

// Microsoft refresh tokens can exceed what one Windows Credential Manager entry
// holds (2,560 bytes of UTF-16), so they are stored in numbered parts, with the
// base entry holding "parts:N".
const PART_CHARS: usize = 1000;
const PARTS_PREFIX: &str = "parts:";

fn mail_entry(email: &str, part: Option<usize>) -> Result<keyring::Entry, String> {
    let user = match part {
        None => format!("ms-refresh:{}", email.to_lowercase()),
        Some(i) => format!("ms-refresh:{}:{i}", email.to_lowercase()),
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

fn split_parts(token: &str) -> Vec<String> {
    let chars: Vec<char> = token.chars().collect();
    chars.chunks(PART_CHARS).map(|c| c.iter().collect()).collect()
}

/// The Microsoft refresh token for a connected inbox.
pub fn get_mail_token(email: &str) -> Result<Option<String>, String> {
    let Some(head) = read(&mail_entry(email, None)?)? else {
        return Ok(None);
    };
    let Some(count) = head.strip_prefix(PARTS_PREFIX).and_then(|n| n.parse::<usize>().ok()) else {
        return Ok(Some(head));
    };
    let mut token = String::new();
    for i in 0..count {
        match read(&mail_entry(email, Some(i))?)? {
            Some(part) => token.push_str(&part),
            None => return Ok(None), // Incomplete; treat as signed out.
        }
    }
    Ok(Some(token))
}

pub fn set_mail_token(email: &str, token: &str) -> Result<(), String> {
    let save = |e: keyring::Error| format!("Could not save to the keychain: {e}");
    delete_mail_token(email)?;
    let parts = split_parts(token);
    for (i, part) in parts.iter().enumerate() {
        mail_entry(email, Some(i))?.set_password(part).map_err(save)?;
    }
    mail_entry(email, None)?.set_password(&format!("{PARTS_PREFIX}{}", parts.len())).map_err(save)
}

pub fn delete_mail_token(email: &str) -> Result<(), String> {
    let remove = |entry: keyring::Entry| match entry.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e.to_string()),
    };
    let head = mail_entry(email, None)?;
    if let Some(count) = read(&head)?.and_then(|h| h.strip_prefix(PARTS_PREFIX).and_then(|n| n.parse::<usize>().ok())) {
        for i in 0..count {
            remove(mail_entry(email, Some(i))?)?;
        }
    }
    remove(head)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn long_tokens_split_into_small_parts() {
        let token = "a".repeat(2500) + "é";
        let parts = split_parts(&token);
        assert_eq!(parts.len(), 3);
        assert!(parts.iter().all(|p| p.chars().count() <= PART_CHARS));
        assert_eq!(parts.concat(), token);
        assert!(split_parts("").is_empty());
    }

    /// Touches the real OS keychain; run with `cargo test -- --ignored`.
    #[test]
    #[ignore]
    fn long_token_round_trips_through_the_os_keychain() {
        let email = "roundtrip-test@example.invalid";
        let token = "t".repeat(4000);
        set_mail_token(email, &token).unwrap();
        assert_eq!(get_mail_token(email).unwrap().as_deref(), Some(token.as_str()));
        set_mail_token(email, "short").unwrap();
        assert_eq!(get_mail_token(email).unwrap().as_deref(), Some("short"));
        delete_mail_token(email).unwrap();
        assert_eq!(get_mail_token(email).unwrap(), None);
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
