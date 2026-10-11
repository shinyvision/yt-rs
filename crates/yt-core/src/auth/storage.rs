//! Persistent TV refresh credentials. No access tokens or plaintext fallback.
use std::path::{Path, PathBuf};

use super::Token;
use crate::error::{Error, Result};

pub(super) trait Store: Send + Sync {
    fn read(&self) -> Result<Option<String>>;
    fn write(&self, value: &str) -> Result<()>;
    fn delete(&self) -> Result<()>;
}

pub(super) struct Keyring {
    profile: PathBuf,
}

impl Keyring {
    pub(super) fn new(legacy_path: &Path) -> Self {
        Self {
            profile: legacy_path.parent().unwrap_or(Path::new(".")).to_owned(),
        }
    }

    fn entry(&self) -> Result<keyring::Entry> {
        let profile = self
            .profile
            .canonicalize()
            .or_else(|_| std::path::absolute(&self.profile))?;
        keyring::Entry::new("dev.ytrs.YouTubeTV", &profile.to_string_lossy()).map_err(|_| {
            Error::Keyring("Couldn't open the system keyring. Unlock it and try again.")
        })
    }
}

impl Store for Keyring {
    fn read(&self) -> Result<Option<String>> {
        match self.entry()?.get_password() {
            Ok(value) => Ok(Some(value)),
            Err(keyring::Error::NoEntry) => Ok(None),
            Err(_) => Err(Error::Keyring(
                "Couldn't read the saved sign-in. Unlock the system keyring and try again.",
            )),
        }
    }

    fn write(&self, value: &str) -> Result<()> {
        self.entry()?.set_password(value).map_err(|_| {
            Error::Keyring(
                "Couldn't save the sign-in in the system keyring. Unlock it and try again.",
            )
        })
    }

    fn delete(&self) -> Result<()> {
        match self.entry()?.delete_credential() {
            Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
            Err(_) => Err(Error::Keyring(
                "Couldn't remove the sign-in from the system keyring. Unlock it and try again.",
            )),
        }
    }
}

fn decode(value: &str) -> Result<Token> {
    let token: Token = serde_json::from_str(value)
        .map_err(|_| Error::Keyring("The saved sign-in is invalid. Sign in again."))?;
    if token.refresh_token.is_empty() {
        return Err(Error::Keyring(
            "The saved sign-in is invalid. Sign in again.",
        ));
    }
    Ok(token)
}

/// Verify the keyring before deleting either legacy file. Access tokens are
/// deliberately excluded: they are renewed in memory after each restart.
pub(super) fn save(store: &dyn Store, path: &Path, token: &Token) -> Result<()> {
    if token.refresh_token.is_empty() {
        return Err(Error::NotSignedIn);
    }
    let value = serde_json::json!({ "refresh_token": token.refresh_token }).to_string();
    store.write(&value)?;
    if store.read()?.as_deref() != Some(&value) {
        return Err(Error::Keyring(
            "Couldn't verify the saved sign-in. The previous token file has been kept.",
        ));
    }
    remove_legacy(path)
}

pub(super) fn restore(store: &dyn Store, path: &Path) -> Result<Option<Token>> {
    if let Some(value) = store.read()? {
        let mut token = decode(&value)?;
        // The keyring wins over an old file left behind by an earlier build.
        remove_legacy(path)?;
        token.access_token.clear();
        token.expires_at = 0;
        return Ok(Some(token));
    }
    let value = match std::fs::read_to_string(path) {
        Ok(value) => value,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
            remove_legacy(path)?;
            return Ok(None);
        }
        Err(e) => return Err(e.into()),
    };
    let token = decode(&value)?;
    save(store, path, &token)?;
    Ok(Some(token))
}

pub(super) fn remove_legacy(path: &Path) -> Result<()> {
    for path in [path.to_owned(), path.with_extension("json.tmp")] {
        match std::fs::remove_file(path) {
            Ok(()) => {}
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {}
            Err(e) => return Err(e.into()),
        }
    }
    Ok(())
}
