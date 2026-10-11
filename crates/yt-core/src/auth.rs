//! Signing in the way the YouTube TV app does: the OAuth 2.0 device flow
//! with the TV app's OAuth client.
//!
//! The user enters a short code at google.com/device; the app then holds a
//! refresh token that lasts until it is revoked. YouTube only accepts these
//! tokens from TV clients, which is why [`crate::innertube`] calls the API
//! as the TV app.

use std::path::PathBuf;
use std::sync::Arc;

mod storage;
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
#[derive(Clone, PartialEq, Serialize, Deserialize)]
pub struct Token {
    pub refresh_token: String,
    #[serde(default, skip_serializing_if = "String::is_empty")]
    pub access_token: String,
    /// Unix time in seconds. Files written by other tools may store a date
    /// string here instead; that reads as 0, so the token is refreshed.
    #[serde(default, alias = "expiry", deserialize_with = "lenient_unix_time")]
    pub expires_at: u64,
}

impl std::fmt::Debug for Token {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Token")
            .field("expires_at", &self.expires_at)
            .finish_non_exhaustive()
    }
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

/// The signed-in account. Persistent refresh credentials live in the OS
/// keyring; access tokens stay in memory. All keyring I/O uses blocking workers.
pub struct Session {
    oauth: OAuth,
    legacy_path: PathBuf,
    store: Arc<dyn storage::Store>,
    state: Mutex<SessionState>,
}

#[derive(Default)]
struct SessionState {
    loaded: bool,
    token: Option<Token>,
}

impl Session {
    async fn storage<T: Send + 'static>(
        &self,
        work: impl FnOnce(&dyn storage::Store, &std::path::Path) -> Result<T> + Send + 'static,
    ) -> Result<T> {
        let store = self.store.clone();
        let path = self.legacy_path.clone();
        tokio::task::spawn_blocking(move || work(store.as_ref(), &path))
            .await
            .map_err(|_| Error::Keyring("The system keyring operation failed. Try again."))?
    }

    async fn restore(&self, state: &mut SessionState) -> Result<()> {
        if !state.loaded {
            state.token = self.storage(storage::restore).await?;
            state.loaded = true;
        }
        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn signed_out(oauth: OAuth) -> Self {
        let mut session = Self::load(oauth, PathBuf::new());
        session.state = Mutex::new(SessionState {
            loaded: true,
            token: None,
        });
        session
    }
}

#[yt_export::export("session")]
impl Session {
    /// Opens the profile's account. The first async call restores the keyring
    /// and migrates an old token file, without blocking the GTK thread.
    #[export(skip = "makes the service")]
    pub fn load(oauth: OAuth, path: PathBuf) -> Self {
        Self {
            oauth,
            store: Arc::new(storage::Keyring::new(&path)),
            legacy_path: path,
            state: Mutex::new(SessionState::default()),
        }
    }

    #[export(skip = "a reference into the service (OAuth is exported itself)")]
    pub fn oauth(&self) -> &OAuth {
        &self.oauth
    }

    pub async fn is_signed_in(&self) -> Result<bool> {
        let mut state = self.state.lock().await;
        self.restore(&mut state).await?;
        Ok(state.token.is_some())
    }

    /// Returns an access token, refreshing it first if it has expired.
    pub async fn access_token(&self) -> Result<String> {
        let mut state = self.state.lock().await;
        self.restore(&mut state).await?;
        let token = state.token.as_mut().ok_or(Error::NotSignedIn)?;
        if !token.is_fresh() {
            *token = self.oauth.refresh(&token.refresh_token).await?;
        }
        Ok(token.access_token.clone())
    }

    /// Refresh once when concurrent requests reject the same access token.
    pub async fn refresh_rejected(&self, rejected: &str) -> Result<String> {
        let mut state = self.state.lock().await;
        self.restore(&mut state).await?;
        let token = state.token.as_mut().ok_or(Error::NotSignedIn)?;
        if token.access_token == rejected {
            *token = self.oauth.refresh(&token.refresh_token).await?;
        }
        Ok(token.access_token.clone())
    }

    /// Sign-in succeeds only after the refresh credential is saved and verified.
    pub async fn sign_in(&self, token: Token) -> Result<()> {
        let mut state = self.state.lock().await;
        let saved = token.clone();
        self.storage(move |store, path| storage::save(store, path, &saved))
            .await?;
        state.token = Some(token);
        state.loaded = true;
        Ok(())
    }

    /// Delete local credentials even when Google is unreachable. Remote
    /// revocation is best effort; a keyring failure remains a retryable error.
    pub async fn sign_out(&self) -> Result<()> {
        let mut state = self.state.lock().await;
        // Removing credentials must remain possible even if their contents
        // are corrupt. Read only to obtain a token for remote revocation.
        let _ = self.restore(&mut state).await;
        self.storage(|store, path| {
            storage::remove_legacy(path)?;
            store.delete()
        })
        .await?;
        let token = state.token.take();
        state.loaded = true;
        drop(state);
        if let Some(token) = token
            && self.oauth.revoke(&token.refresh_token).await.is_err()
        {
            tracing::warn!("Signed out locally; Google session revocation could not be completed");
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use storage::Store;

    #[derive(Default)]
    struct MemoryStore {
        value: std::sync::Mutex<Option<String>>,
        fail_read: AtomicBool,
        fail_write: AtomicBool,
        fail_delete: AtomicBool,
        lose_write: AtomicBool,
        writes: AtomicUsize,
    }

    impl Store for MemoryStore {
        fn read(&self) -> Result<Option<String>> {
            if self.fail_read.load(Ordering::SeqCst) {
                return Err(Error::Keyring("locked"));
            }
            Ok(self.value.lock().unwrap().clone())
        }
        fn write(&self, value: &str) -> Result<()> {
            if self.fail_write.load(Ordering::SeqCst) {
                return Err(Error::Keyring("locked"));
            }
            self.writes.fetch_add(1, Ordering::SeqCst);
            if !self.lose_write.load(Ordering::SeqCst) {
                *self.value.lock().unwrap() = Some(value.to_owned());
            }
            Ok(())
        }
        fn delete(&self) -> Result<()> {
            if self.fail_delete.load(Ordering::SeqCst) {
                return Err(Error::Keyring("locked"));
            }
            self.value.lock().unwrap().take();
            Ok(())
        }
    }

    struct Profile(PathBuf);
    impl Profile {
        fn new() -> Self {
            let path = std::env::temp_dir().join(format!(
                "yt-rs-auth-{}-{}",
                std::process::id(),
                rand::random::<u64>()
            ));
            std::fs::create_dir(&path).unwrap();
            Self(path)
        }
        fn path(&self) -> PathBuf {
            self.0.join("token.json")
        }
        fn legacy(&self) {
            std::fs::write(self.path(), serde_json::to_vec(&token()).unwrap()).unwrap();
            std::fs::write(
                self.path().with_extension("json.tmp"),
                "old temporary token",
            )
            .unwrap();
        }
    }
    impl Drop for Profile {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }
    fn token() -> Token {
        Token {
            refresh_token: "fixture-refresh".into(),
            access_token: "fixture-access".into(),
            expires_at: unix_now() + 3600,
        }
    }
    fn session(profile: &Profile, store: Arc<MemoryStore>) -> Session {
        let mut session = Session::load(
            OAuth::with_endpoints(
                reqwest::Client::new(),
                "http://127.0.0.1:0",
                "http://127.0.0.1:0/revoke",
            ),
            profile.path(),
        );
        session.store = store;
        session
    }

    #[tokio::test]
    async fn migration_verifies_keyring_and_removes_both_plaintext_files() {
        let profile = Profile::new();
        profile.legacy();
        let store = Arc::new(MemoryStore::default());
        let session = session(&profile, store.clone());
        let (first, second) = tokio::join!(session.is_signed_in(), session.is_signed_in());
        assert!(first.unwrap() && second.unwrap());
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);
        let saved: Value = serde_json::from_str(&store.read().unwrap().unwrap()).unwrap();
        assert_eq!(saved, json!({"refresh_token": "fixture-refresh"}));
        assert!(!profile.path().exists());
        assert!(!profile.path().with_extension("json.tmp").exists());
    }

    #[tokio::test]
    async fn migration_never_removes_the_only_copy_after_a_failed_save() {
        for lose_write in [false, true] {
            let profile = Profile::new();
            profile.legacy();
            let store = Arc::new(MemoryStore::default());
            store.fail_write.store(!lose_write, Ordering::SeqCst);
            store.lose_write.store(lose_write, Ordering::SeqCst);
            let session = session(&profile, store);
            assert!(session.is_signed_in().await.is_err());
            assert!(profile.path().exists());
            assert!(profile.path().with_extension("json.tmp").exists());
            assert!(session.state.lock().await.token.is_none());
        }
    }

    #[tokio::test]
    async fn locked_keyring_does_not_fall_back_to_plaintext_and_can_retry() {
        let profile = Profile::new();
        profile.legacy();
        let store = Arc::new(MemoryStore::default());
        store.fail_read.store(true, Ordering::SeqCst);
        let session = session(&profile, store.clone());
        assert!(matches!(
            session.access_token().await,
            Err(Error::Keyring(_))
        ));
        assert_eq!(store.writes.load(Ordering::SeqCst), 0);
        store.fail_read.store(false, Ordering::SeqCst);
        assert!(session.is_signed_in().await.unwrap());
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    async fn keyring_account_takes_precedence_over_stale_plaintext() {
        let profile = Profile::new();
        profile.legacy();
        let store = Arc::new(MemoryStore::default());
        store.write(r#"{"refresh_token":"new-account"}"#).unwrap();
        let session = session(&profile, store);
        assert!(session.is_signed_in().await.unwrap());
        let state = session.state.lock().await;
        let token = state.token.as_ref().unwrap();
        assert_eq!(token.refresh_token, "new-account");
        assert!(token.access_token.is_empty());
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    async fn failed_sign_in_save_never_creates_a_plaintext_fallback() {
        let profile = Profile::new();
        let store = Arc::new(MemoryStore::default());
        store.fail_write.store(true, Ordering::SeqCst);
        let session = session(&profile, store);
        assert!(session.sign_in(token()).await.is_err());
        assert!(!session.is_signed_in().await.unwrap());
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    async fn signing_out_offline_clears_keyring_memory_and_legacy_files() {
        let profile = Profile::new();
        let store = Arc::new(MemoryStore::default());
        let session = session(&profile, store.clone());
        session.sign_in(token()).await.unwrap();
        profile.legacy();
        session.sign_out().await.unwrap();
        assert!(!session.is_signed_in().await.unwrap());
        assert!(store.read().unwrap().is_none());
        assert!(!profile.path().exists());
        assert!(!profile.path().with_extension("json.tmp").exists());
    }

    #[tokio::test]
    async fn failed_keyring_removal_keeps_sign_out_retryable() {
        let profile = Profile::new();
        let store = Arc::new(MemoryStore::default());
        let session = session(&profile, store.clone());
        session.sign_in(token()).await.unwrap();
        store.fail_delete.store(true, Ordering::SeqCst);
        assert!(session.sign_out().await.is_err());
        assert!(session.is_signed_in().await.unwrap());
        store.fail_delete.store(false, Ordering::SeqCst);
        session.sign_out().await.unwrap();
        assert!(!session.is_signed_in().await.unwrap());
    }

    #[tokio::test]
    async fn corrupt_keyring_is_an_error_and_can_be_cleared() {
        let profile = Profile::new();
        let store = Arc::new(MemoryStore::default());
        store.write("invalid-fixture-secret").unwrap();
        let session = session(&profile, store.clone());
        let error = session.is_signed_in().await.unwrap_err().to_string();
        assert!(!error.contains("invalid-fixture-secret"));
        session.sign_out().await.unwrap();
        assert!(store.read().unwrap().is_none());
    }

    #[tokio::test]
    async fn restart_renews_access_in_memory_without_writing_an_access_token() {
        use std::io::{Read, Write};
        let server = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let address = server.local_addr().unwrap();
        let reply = std::thread::spawn(move || {
            let (mut stream, _) = server.accept().unwrap();
            stream
                .set_read_timeout(Some(Duration::from_secs(5)))
                .unwrap();
            let mut request = Vec::new();
            let mut buf = [0; 1024];
            loop {
                let n = stream.read(&mut buf).unwrap();
                assert!(n > 0);
                request.extend_from_slice(&buf[..n]);
                let text = String::from_utf8_lossy(&request);
                if let Some((headers, body)) = text.split_once("\r\n\r\n") {
                    let length: usize = headers
                        .lines()
                        .find_map(|l| {
                            l.to_lowercase()
                                .strip_prefix("content-length: ")
                                .map(str::to_owned)
                        })
                        .unwrap()
                        .parse()
                        .unwrap();
                    if body.len() >= length {
                        break;
                    }
                }
            }
            assert!(String::from_utf8_lossy(&request).starts_with("POST /token "));
            let body = r#"{"access_token":"renewed-fixture-access","expires_in":3600}"#;
            write!(stream, "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}", body.len(), body).unwrap();
        });
        let profile = Profile::new();
        let store = Arc::new(MemoryStore::default());
        session(&profile, store.clone())
            .sign_in(token())
            .await
            .unwrap();
        let mut restarted = session(&profile, store.clone());
        restarted.oauth = OAuth::with_endpoints(
            reqwest::Client::new(),
            &format!("http://{address}"),
            "http://127.0.0.1:0/revoke",
        );
        assert_eq!(
            restarted.access_token().await.unwrap(),
            "renewed-fixture-access"
        );
        assert_eq!(
            restarted.access_token().await.unwrap(),
            "renewed-fixture-access"
        );
        reply.join().unwrap();
        assert_eq!(store.writes.load(Ordering::SeqCst), 1);
        assert!(!store.read().unwrap().unwrap().contains("access"));
        assert!(!profile.path().exists());
    }

    #[tokio::test]
    #[ignore = "requires an unlocked system keyring; uses only disposable dummy credentials"]
    async fn system_keyring_round_trip_isolated_by_profile() {
        let first = Profile::new();
        let second = Profile::new();
        let make = |p: &Profile| {
            Session::load(
                OAuth::with_endpoints(
                    reqwest::Client::new(),
                    "http://127.0.0.1:0",
                    "http://127.0.0.1:0/revoke",
                ),
                p.path(),
            )
        };
        let signed_in = make(&first);
        signed_in.sign_in(token()).await.unwrap();
        assert!(make(&first).is_signed_in().await.unwrap());
        assert!(!make(&second).is_signed_in().await.unwrap());
        signed_in.sign_out().await.unwrap();
        assert!(!make(&first).is_signed_in().await.unwrap());
        assert!(!first.path().exists());
    }

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
}
