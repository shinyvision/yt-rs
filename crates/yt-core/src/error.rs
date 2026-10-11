use std::fmt;

/// Errors from talking to YouTube. Messages are written to be shown to the
/// user as they are.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("not signed in")]
    NotSignedIn,
    #[error("the sign-in has expired or was revoked; sign in again")]
    SignInExpired,
    #[error("the sign-in was declined")]
    SignInDeclined,
    #[error("the sign-in code expired before it was approved")]
    SignInCodeExpired,
    #[error("{0}")]
    Keyring(&'static str),
    #[error("YouTube rejected the access token")]
    Unauthorized,
    #[error("{0}")]
    OAuth(OAuthError),
    #[error("{endpoint}: HTTP {status}{}", message.as_deref().map(|m| format!(": {m}")).unwrap_or_default())]
    Api {
        endpoint: String,
        status: u16,
        message: Option<String>,
    },
    #[error("network error: {0}")]
    Http(#[from] reqwest::Error),
    #[error("unexpected response: {0}")]
    Json(#[from] serde_json::Error),
    #[error(transparent)]
    Io(#[from] std::io::Error),
    #[error("{0}")]
    Streams(String),
    /// From a plugin hooking a call.
    #[error("{0}")]
    Plugin(String),
    /// YouTube won't play the video (now): `reason` is its own wording.
    #[error("{reason}")]
    Unplayable {
        reason: String,
        why: Unplayable,
        /// The video's channel, for members-only videos.
        channel_id: Option<String>,
    },
}

/// Why YouTube won't play a video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Unplayable {
    /// It needs a signed-in account (to confirm the viewer's age, or that
    /// they aren't a bot), or one whose age YouTube has confirmed.
    SignIn,
    /// It is for the channel's members.
    MembersOnly,
    /// A live stream or premiere that hasn't started yet.
    Upcoming,
    /// Private, removed, not available in the country, ...
    Other,
}

pub type Result<T, E = Error> = std::result::Result<T, E>;

impl yt_export::HookError for Error {
    fn hook_error(message: String) -> Self {
        Error::Plugin(message)
    }
}

/// An error reported by Google's OAuth endpoints, such as
/// `authorization_pending` or `invalid_grant`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Deserialize)]
pub struct OAuthError {
    #[serde(rename = "error")]
    pub code: String,
    #[serde(rename = "error_description", default)]
    pub description: Option<String>,
}

impl fmt::Display for OAuthError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "sign-in failed: {}", self.code)?;
        if let Some(d) = &self.description {
            write!(f, ": {d}")?;
        }
        Ok(())
    }
}
