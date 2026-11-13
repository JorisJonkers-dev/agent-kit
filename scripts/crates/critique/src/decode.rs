//! Reading the JSON the runner relies on: one file, or the command's own input. The node runner's
//! `application/decode.ts`.

use crate::error::{CritiqueError, ShapeError};

/// A parser's result as the runner reports it: a syntax error or a wrong shape becomes an error with `code`, worded
/// `<where>: <message>`.
pub fn decoded<T>(code: &str, place: &str, read: Result<T, String>) -> Result<T, CritiqueError> {
    read.map_err(|message| CritiqueError::new(code, format!("{place}: {message}")))
}

/// The command's input, read with `read`; a wrong shape is an `invalid-input` naming the command.
pub fn read_input<T>(command: &str, read: Result<T, ShapeError>) -> Result<T, CritiqueError> {
    read.map_err(|error| error.into_error("invalid-input", &format!("{command} input")))
}
