//! Reading the JSON the runner relies on: one file, or the command's own input. The node runner's
//! `application/decode.ts`.

use crate::error::{CritiqueError, ShapeError};
use crate::runtime::Runtime;

/// A parser's result as the runner reports it: a syntax error or a wrong shape becomes an error with `code`, worded
/// `<where>: <message>`.
pub fn decoded<T>(code: &str, place: &str, read: Result<T, String>) -> Result<T, CritiqueError> {
    read.map_err(|message| CritiqueError::new(code, format!("{place}: {message}")))
}

/// The command's input, read with `read`; a wrong shape is an `invalid-input` naming the command.
pub fn read_input<T>(command: &str, read: Result<T, ShapeError>) -> Result<T, CritiqueError> {
    read.map_err(|error| error.into_error("invalid-input", &format!("{command} input")))
}

/// Reads one JSON file the runner relies on; a missing or malformed file is an error with `code`.
pub fn read_json_file<T>(runtime: &Runtime, path: &str, code: &str, parse: fn(&str) -> Result<T, String>) -> Result<T, CritiqueError> {
    if !runtime.files.exists(path).unwrap_or(false) {
        return Err(CritiqueError::new(code, format!("{path} does not exist")));
    }
    let text = runtime.files.read_text(path).map_err(CritiqueError::from_fs)?;
    decoded(code, path, parse(&text))
}
