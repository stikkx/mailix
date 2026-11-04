//! Credentials in the OS keyring (Secret Service, macOS Keychain, Windows
//! Credential Manager).

use anyhow::{Context, Result};

const SERVICE: &str = "mailix";
/// Name used before the project was renamed; entries are copied over on first use.
const OLD_SERVICE: &str = "mailixx";

fn entry(key: &str) -> Result<keyring::Entry> {
    keyring::Entry::new(SERVICE, key).context("OS keyring unavailable")
}

pub fn get(key: &str) -> Result<Option<String>> {
    match entry(key)?.get_password() {
        Ok(p) => Ok(Some(p)),
        Err(keyring::Error::NoEntry) => {
            // Copy from the old name (the old entry stays as a backup).
            let old = keyring::Entry::new(OLD_SERVICE, key).context("OS keyring unavailable")?;
            match old.get_password() {
                Ok(p) => {
                    if let Err(e) = entry(key)?.set_password(&p) {
                        tracing::warn!("copying keyring entry {key}: {e}");
                    }
                    Ok(Some(p))
                }
                Err(keyring::Error::NoEntry) => Ok(None),
                Err(e) => Err(e).context("reading keyring"),
            }
        }
        Err(e) => Err(e).context("reading keyring"),
    }
}

pub fn set(key: &str, value: &str) -> Result<()> {
    if value.is_empty() {
        return delete(key);
    }
    entry(key)?.set_password(value).context("writing keyring")
}

pub fn delete(key: &str) -> Result<()> {
    match entry(key)?.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(e) => Err(e).context("deleting from keyring"),
    }
}

pub fn imap_key(account_id: &str) -> String {
    format!("account:{account_id}:imap")
}
pub fn smtp_key(account_id: &str) -> String {
    format!("account:{account_id}:smtp")
}
pub fn ldap_key(name: &str) -> String {
    format!("ldap:{name}")
}
pub fn google_key(name: &str) -> String {
    format!("google:{name}:refresh_token")
}

/// Password for a server: dedicated SMTP password if set, otherwise the IMAP one.
pub fn smtp_password(account_id: &str) -> Result<String> {
    if let Some(p) = get(&smtp_key(account_id))? {
        return Ok(p);
    }
    imap_password(account_id)
}

pub fn imap_password(account_id: &str) -> Result<String> {
    get(&imap_key(account_id))?.context("no password stored for this account — set it in Settings")
}

#[cfg(test)]
mod tests {
    /// Touches the real OS keyring: `cargo test -- --ignored keyring`
    #[test]
    #[ignore]
    fn keyring_roundtrip() {
        let key = "test:roundtrip";
        super::set(key, "s3cret").unwrap();
        assert_eq!(super::get(key).unwrap().as_deref(), Some("s3cret"));
        super::delete(key).unwrap();
        assert_eq!(super::get(key).unwrap(), None);
    }
}
