//! Signing in the way the YouTube TV app does: the OAuth 2.0 device flow
//! with the TV app's OAuth client.
//!
//! The user enters a short code at google.com/device; the app then holds a
//! refresh token that lasts until it is revoked. YouTube only accepts these
//! tokens from TV clients, which is why [`crate::innertube`] calls the API
//! as the TV app.

use std::path::{Path, PathBuf};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde::{Deserialize, Deserializer, Serialize};
use serde_json::{Value, json};
use tokio::sync::Mutex;

use crate::error::{Error, OAuthError, Result};

/// The YouTube TV app's OAuth client. These ship in the TV app itself.
pub const CLIENT_ID: &str =
    "861556708454-d6dlm3lh05idd8npek18k6be8ba3oc68.apps.googleusercontent.com";
pub const CLIENT_SECRET: &str = "SboVhoG9s0rNafixCSGGKXAT";
pub const SCOPE: &str = "http://gdata.youtube.com https://www.googleapis.com/auth/youtube";

const OAUTH_URL: &str = "https://www.youtube.com/o/oauth2";
const REVOKE_URL: &str = "https://oauth2.googleapis.com/revoke";
pub(crate) const TV_USER_AGENT: &str = "Mozilla/5.0 (ChromiumStylePlatform) Cobalt/25.lts.30.1034943-gold (unlike Gecko), Unknown_TV_Unknown_0/Unknown (Unknown, Unknown)";

/// A saved sign-in. The refresh token lasts until it is revoked; the access
/// token is replaced whenever it expires.
#[derive(Clone, Debug, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    /// Unix time in seconds. Files written by other tools may store a date
    /// string here instead; that reads as 0, so the token is refreshed.
    #[serde(default, alias = "expiry", deserialize_with = "lenient_unix_time")]
    pub expires_at: u64,
}

impl Token {
    /// Whether the access token can still be used for at least a minute.
    pub fn is_fresh(&self) -> bool {
        !self.access_token.is_empty() && self.expires_at > unix_now() + 60
    }
}

fn lenient_unix_time<'de, D: Deserializer<'de>>(d: D) -> Result<u64, D::Error> {
    Ok(Value::deserialize(d)?.as_u64().unwrap_or(0))
}

fn unix_now() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |d| d.as_secs())
}

/// A pending sign-in: the user enters `user_code` at `verification_url`.
#[derive(Clone, Debug, Deserialize, Serialize)]
pub struct DeviceCode {
    pub device_code: String,
    pub user_code: String,
    pub verification_url: String,
    pub expires_in: u64,
    pub interval: u64,
}

/// Google's OAuth endpoints, as used by the YouTube TV app.
#[derive(Clone)]
pub struct OAuth {
    http: reqwest::Client,
    base: String,
    revoke_url: String,
}

#[yt_export::export("oauth")]
impl OAuth {
    #[export(skip = "makes the service")]
    pub fn new(http: reqwest::Client) -> Self {
        Self::with_endpoints(http, OAUTH_URL, REVOKE_URL)
    }

    /// Uses other endpoints, for tests.
    #[export(skip = "makes the service")]
    pub fn with_endpoints(http: reqwest::Client, base: &str, revoke_url: &str) -> Self {
        Self {
            http,
            base: base.to_owned(),
            revoke_url: revoke_url.to_owned(),
        }
    }

    /// Starts a sign-in.
    pub async fn request_device_code(&self) -> Result<DeviceCode> {
        let device_id: String = (0..32)
            .map(|_| format!("{:x}", rand::random::<u8>() & 15))
            .collect();
        let resp = self
            .post(
                "/device/code",
                json!({
                    "client_id": CLIENT_ID,
                    "scope": SCOPE,
                    "device_id": device_id,
                    "device_model": "ytlr::",
                }),
            )
            .await?;
        Ok(serde_json::from_value(resp)?)
    }

    /// Polls until the user has approved the device code.
    pub async fn wait_for_approval(&self, code: &DeviceCode) -> Result<Token> {
        let mut interval = Duration::from_secs(code.interval);
        let deadline = tokio::time::Instant::now() + Duration::from_secs(code.expires_in);
        while tokio::time::Instant::now() < deadline {
            tokio::time::sleep(interval).await;
            let result = self
                .request_token(json!({
                    "code": code.device_code,
                    "grant_type": "http://oauth.net/grant_type/device/1.0",
                }))
                .await;
            match result {
                Ok(token) => return Ok(token),
                Err(Error::OAuth(e)) if e.code == "authorization_pending" => {}
                Err(Error::OAuth(e)) if e.code == "slow_down" => interval += Duration::from_secs(5),
                Err(Error::OAuth(e)) if e.code == "access_denied" => {
                    return Err(Error::SignInDeclined);
                }
                Err(Error::OAuth(e)) if e.code == "expired_token" => {
                    return Err(Error::SignInCodeExpired);
                }
                Err(e) => return Err(e),
            }
        }
        Err(Error::SignInCodeExpired)
    }

    /// Gets a new access token for a refresh token.
    pub async fn refresh(&self, refresh_token: &str) -> Result<Token> {
        let result = self
            .request_token(json!({ "refresh_token": refresh_token, "grant_type": "refresh_token" }))
            .await;
        match result {
            Ok(token) => Ok(Token {
                refresh_token: refresh_token.to_owned(),
                ..token
            }),
            Err(Error::OAuth(e)) if e.code == "invalid_grant" => Err(Error::SignInExpired),
            Err(e) => Err(e),
        }
    }

    /// Revokes a refresh token and every access token issued for it.
    pub async fn revoke(&self, refresh_token: &str) -> Result<()> {
        let resp = self
            .http
            .post(&self.revoke_url)
            .form(&[("token", refresh_token)])
            .send()
            .await?;
        if !resp.status().is_success() {
            return Err(Error::Api {
                endpoint: "revoke".into(),
                status: resp.status().as_u16(),
                message: None,
            });
        }
        Ok(())
    }

    async fn request_token(&self, mut params: Value) -> Result<Token> {
        params["client_id"] = CLIENT_ID.into();
        params["client_secret"] = CLIENT_SECRET.into();
        #[derive(Deserialize)]
        struct Resp {
            access_token: String,
            #[serde(default)]
            refresh_token: String,
            #[serde(default)]
            expires_in: u64,
        }
        let resp: Resp = serde_json::from_value(self.post("/token", params).await?)?;
        Ok(Token {
            refresh_token: resp.refresh_token,
            access_token: resp.access_token,
            expires_at: unix_now() + resp.expires_in,
        })
    }

    async fn post(&self, path: &str, body: Value) -> Result<Value> {
        let resp = self
            .http
            .post(format!("{}{path}", self.base))
            .header(reqwest::header::USER_AGENT, TV_USER_AGENT)
            .json(&body)
            .send()
            .await?;
        let status = resp.status();
        let body: Value = resp.json().await.unwrap_or(Value::Null);
        // YouTube's endpoint reports errors such as authorization_pending
        // with status 200, so look for an error in every response.
        if body.get("error").is_some_and(Value::is_string) {
            return Err(Error::OAuth(serde_json::from_value::<OAuthError>(body)?));
        }
        if !status.is_success() {
            return Err(Error::Api {
                endpoint: path.into(),
                status: status.as_u16(),
                message: None,
            });
        }
        Ok(body)
    }
}

/// The signed-in account. Hands out valid access tokens, refreshing and
/// saving the token when it expires.
pub struct Session {
    oauth: OAuth,
    path: PathBuf,
    token: Mutex<Option<Token>>,
}

#[yt_export::export("session")]
impl Session {
    /// Loads the sign-in saved at `path`, if there is one.
    #[export(skip = "makes the service")]
    pub fn load(oauth: OAuth, path: PathBuf) -> Self {
        let token = read_token(&path).ok();
        Self {
            oauth,
            path,
            token: Mutex::new(token),
        }
    }

    #[export(skip = "a reference into the service (OAuth is exported itself)")]
    pub fn oauth(&self) -> &OAuth {
        &self.oauth
    }

    pub async fn is_signed_in(&self) -> bool {
        self.token.lock().await.is_some()
    }

    /// Returns an access token, refreshing it first if it has expired.
    pub async fn access_token(&self) -> Result<String> {
        let mut guard = self.token.lock().await;
        let token = guard.as_mut().ok_or(Error::NotSignedIn)?;
        if !token.is_fresh() {
            *token = self.oauth.refresh(&token.refresh_token).await?;
            write_token(&self.path, token)?;
        }
        Ok(token.access_token.clone())
    }

    /// Replaces the access token even if it hasn't expired, for when YouTube
    /// rejected it early. `rejected` is the token that was rejected, so that
    /// concurrent callers refresh only once.
    pub async fn refresh_rejected(&self, rejected: &str) -> Result<String> {
        let mut guard = self.token.lock().await;
        let token = guard.as_mut().ok_or(Error::NotSignedIn)?;
        if token.access_token == rejected {
            *token = self.oauth.refresh(&token.refresh_token).await?;
            write_token(&self.path, token)?;
        }
        Ok(token.access_token.clone())
    }

    /// Stores a token from a completed sign-in.
    pub async fn sign_in(&self, token: Token) -> Result<()> {
        write_token(&self.path, &token)?;
        *self.token.lock().await = Some(token);
        Ok(())
    }

    /// Revokes the sign-in and deletes the saved token.
    pub async fn sign_out(&self) -> Result<()> {
        let token = self.token.lock().await.take();
        if let Some(token) = token {
            self.oauth.revoke(&token.refresh_token).await?;
        }
        match std::fs::remove_file(&self.path) {
            Err(e) if e.kind() != std::io::ErrorKind::NotFound => Err(e.into()),
            _ => Ok(()),
        }
    }
}

pub(crate) fn read_token(path: &Path) -> Result<Token> {
    let token: Token = serde_json::from_slice(&std::fs::read(path)?)?;
    if token.refresh_token.is_empty() {
        return Err(Error::NotSignedIn);
    }
    Ok(token)
}

/// Writes the token atomically, readable only by the current user.
pub(crate) fn write_token(path: &Path, token: &Token) -> Result<()> {
    use std::io::Write;
    use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};

    if let Some(dir) = path.parent() {
        std::fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(dir)?;
    }
    let tmp = path.with_extension("json.tmp");
    let mut file = std::fs::OpenOptions::new()
        .write(true)
        .create(true)
        .truncate(true)
        .mode(0o600)
        .open(&tmp)?;
    file.write_all(&serde_json::to_vec_pretty(token)?)?;
    file.sync_all()?;
    std::fs::rename(&tmp, path)?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reads_tokens_written_by_the_go_cli() {
        let token: Token = serde_json::from_str(
            r#"{"refresh_token":"r","access_token":"a","expiry":"2026-10-11T16:42:46.37+02:00"}"#,
        )
        .unwrap();
        assert_eq!(token.refresh_token, "r");
        assert!(!token.is_fresh(), "a date string must read as expired");
    }

    #[test]
    fn freshness() {
        let mut token = Token {
            refresh_token: "r".into(),
            access_token: "a".into(),
            expires_at: unix_now() + 3600,
        };
        assert!(token.is_fresh());
        token.expires_at = unix_now() + 30;
        assert!(!token.is_fresh());
    }

    #[test]
    fn token_file_round_trip_is_private() {
        use std::os::unix::fs::PermissionsExt;
        let dir = std::env::temp_dir().join(format!("yt-core-token-{}", std::process::id()));
        let path = dir.join("sub").join("token.json");
        let token = Token {
            refresh_token: "r".into(),
            access_token: "a".into(),
            expires_at: 42,
        };
        write_token(&path, &token).unwrap();
        assert_eq!(
            std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
            0o600
        );
        assert_eq!(read_token(&path).unwrap(), token);
        std::fs::remove_dir_all(dir).unwrap();
    }
}
