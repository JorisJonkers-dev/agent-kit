//! The one error the gateway reports: a code the contract names, the message
//! for the owner, and, for a plan the gateway refused, every problem it found.

use crate::batch::PlanProblem;
use crate::ports::FsError;

/// One error, as the contract prints it: `code` on stdout, `message` on both streams, and `problems` beside them
/// for a refused plan.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct GatewayError {
    pub code: String,
    pub message: String,
    /// The problems a refused plan found, which the command line prints beside the code; empty for any other error.
    pub problems: Vec<PlanProblem>,
}

impl GatewayError {
    pub fn new(code: &str, message: impl Into<String>) -> Self {
        GatewayError { code: code.to_string(), message: message.into(), problems: Vec::new() }
    }

    /// An error that carries the problems it found, as a refused plan does.
    pub fn with_problems(code: &str, message: impl Into<String>, problems: Vec<PlanProblem>) -> Self {
        GatewayError { code: code.to_string(), message: message.into(), problems }
    }

    pub fn invalid_input(message: impl Into<String>) -> Self {
        Self::new("invalid-input", message)
    }

    /// A file system failure is the platform's own words, and carries no code of the gateway's.
    pub fn from_fs(error: FsError) -> Self {
        Self::new("unexpected", error.message)
    }
}
