//! The one error the runner reports: a code the contract names and the message
//! for the owner. The command line prints it on both streams and exits 1.

use model_gateway::ports::FsError;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CritiqueError {
    pub code: String,
    pub message: String,
}

impl CritiqueError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        CritiqueError { code: code.to_string(), message: message.into() }
    }

    /// Any failure the contract has no code for: what the node runner calls `unexpected`.
    pub fn unexpected(message: impl Into<String>) -> Self {
        Self::new("unexpected", message)
    }

    /// A file system failure is the platform's own words, with no code of the runner's.
    pub fn from_fs(error: FsError) -> Self {
        Self::unexpected(error.message)
    }
}

/// A JSON value without the shape its reader expects; the message names the path of the wrong field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ShapeError(pub String);

impl ShapeError {
    /// The error a command's input or a file earns, worded `<where>: <message>`, under the given code.
    pub fn into_error(self, code: &str, place: &str) -> CritiqueError {
        CritiqueError::new(code, format!("{place}: {}", self.0))
    }
}
