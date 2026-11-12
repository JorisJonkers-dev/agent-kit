//! The one error the gateway reports: a code the contract names, and the
//! message for the owner.

use crate::ports::FsError;

/// One error, as the contract prints it: `code` on stdout, `message` on both streams.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayError {
    pub code: String,
    pub message: String,
}

impl GatewayError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        GatewayError { code: code.to_string(), message: message.into() }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalid-input", message)
    }

    /// A file system failure is the platform's own words, and carries no code of the gateway's.
    pub fn from_fs(error: FsError) -> Self {
        Self::new("unexpected", error.message)
    }
}
