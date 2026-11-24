//! One error type for the whole backend. Commands return it to the UI as a
//! plain string.

use serde::{Serialize, Serializer};

#[derive(Debug, thiserror::Error)]
pub enum AppError {
    /// No GitHub token could be found.
    #[error("not signed in to GitHub: {0}")]
    NotSignedIn(String),
    /// GitHub refused the token.
    #[error("GitHub rejected the token: {0}")]
    Unauthorized(String),
    #[error("GitHub rate limit reached; resets at {reset_at}")]
    RateLimited { reset_at: String },
    /// The repo or PR does not exist, or the token cannot see it.
    #[error("not found: {0}")]
    NotFound(String),
    /// User input we could not interpret, e.g. a malformed repo name.
    #[error("{0}")]
    InvalidInput(String),
    #[error("network error: {0}")]
    Network(String),
    /// A response we could not parse.
    #[error("unexpected response: {0}")]
    Decode(String),
    #[error("could not read or write the config file: {0}")]
    Config(String),
    #[error("{0}")]
    Other(String),
}

pub type AppResult<T> = Result<T, AppError>;

impl Serialize for AppError {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(&self.to_string())
    }
}

impl From<reqwest::Error> for AppError {
    fn from(e: reqwest::Error) -> Self {
        if e.is_decode() {
            AppError::Decode(e.to_string())
        } else {
            AppError::Network(e.to_string())
        }
    }
}

impl From<serde_json::Error> for AppError {
    fn from(e: serde_json::Error) -> Self {
        AppError::Decode(e.to_string())
    }
}
