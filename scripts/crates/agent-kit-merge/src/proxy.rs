//! One marked `#` block in a config file (the `package-proxy-merge.py` script):
//! npmrc, java .properties, TOML and pip.conf all take `#` comments.

use crate::text::{repr, splitlines, strip, universal};
use crate::{make_parent, with_suffix, Failure, Io};
use regex::Regex;
use std::fs;
use std::io::Write;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::Path;

/// What `--validate` parses the merged text with.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Validate {
    Toml,
    Ini,
}

pub struct Options {
    pub remove: bool,
    pub validate: Option<Validate>,
    pub refuse: Option<String>,
    pub mode: Option<String>,
}

/// The begin and end lines of setup's block in one file.
pub fn markers(name: &str) -> (String, String) {
    (format!("# agent-kit {name} begin"), format!("# agent-kit {name} end"))
}

/// Where the first block of this name stands; a block that never closes is an error.
pub fn block_bounds(lines: &[String], begin: &str, end: &str) -> Result<Option<(usize, usize)>, Failure> {
    let Some(start) = lines.iter().position(|line| strip(line) == begin) else {
        return Ok(None);
    };
    match (start + 1..lines.len()).find(|i| strip(&lines[*i]) == end) {
        Some(stop) => Ok(Some((start, stop))),
        None => Err(Failure(format!("the block opened at line {} is never closed", start + 1))),
    }
}

/// The lines with every block of this name taken out.
pub fn without_block(lines: &[String], begin: &str, end: &str) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    let mut skipping = false;
    for line in lines {
        let stripped = strip(line);
        if !skipping && stripped == begin {
            skipping = true;
        } else if skipping && stripped == end {
            skipping = false;
            if let Some(last) = out.last_mut().filter(|last| !last.ends_with('\n')) {
                last.push('\n');
            }
        } else if !skipping {
            out.push(line.clone());
        }
    }
    out
}

/// The lines with one block appended, kept apart by a blank line.
pub fn append_block(lines: &[String], block: &[String]) -> Vec<String> {
    let mut out = lines.to_vec();
    if let Some(last) = out.last_mut().filter(|last| !last.ends_with('\n')) {
        last.push('\n');
    }
    if out.last().is_some_and(|last| !strip(last).is_empty()) {
        out.push("\n".to_string());
    }
    out.extend(block.iter().cloned());
    out
}

/// Why the text does not parse as `kind`, or None when it does.
pub fn parse_problem(kind: Validate, text: &str) -> Option<String> {
    match kind {
        Validate::Toml => text.parse::<toml_edit::DocumentMut>().err().map(|e| e.to_string()),
        Validate::Ini => ini_problem(text),
    }
}

/// Python's `configparser` in strict mode: what it would refuse.
fn ini_problem(text: &str) -> Option<String> {
    let mut sections: Vec<(String, Vec<String>)> = Vec::new();
    let mut option: Option<String> = None;
    let mut indent_level = 0;
    for (number, raw) in text.split('\n').enumerate() {
        let number = number + 1;
        let value = strip(raw);
        if value.is_empty() || value.starts_with('#') || value.starts_with(';') {
            continue;
        }
        let indent = raw.chars().take_while(|c| c.is_whitespace()).count();
        if !sections.is_empty() && option.is_some() && indent > indent_level {
            continue;
        }
        indent_level = indent;
        if let Some(header) = value.strip_prefix('[').and_then(|rest| rest.rfind(']').filter(|at| *at > 0).map(|at| &rest[..at])) {
            if sections.iter().any(|(name, _)| name == header) {
                return Some(format!("While reading from '<string>' [line {number:2}]: section {} already exists", repr(header)));
            }
            sections.push((header.to_string(), Vec::new()));
            option = None;
            continue;
        }
        let Some((_, options)) = sections.last_mut() else {
            return Some(format!("File contains no section headers.\nfile: '<string>', line: {number}\n{}", repr(&format!("{raw}\n"))));
        };
        let name = value.find(['=', ':']).map(|at| strip(&value[..at]).to_lowercase()).filter(|name| !name.is_empty());
        let Some(name) = name else {
            return Some(format!("Source contains parsing errors: '<string>'\n\t[line {number:2}]: {}", repr(&format!("{raw}\n"))));
        };
        if options.contains(&name) {
            return Some(format!("While reading from '<string>' [line {number:2}]: option {} in section already exists", repr(&name)));
        }
        options.push(name.clone());
        option = Some(name);
    }
    None
}

/// `package-proxy-merge.py FILE NAME [--remove] [--validate KIND] [--refuse REGEX] [--mode MODE]`.
/// Exit 0 with `changed` or `unchanged`; 2 for a half-written block, an empty body or a
/// bad option; 3 for a refused merge.
pub fn package_proxy(file: &Path, name: &str, options: &Options, io: &mut Io) -> Result<i32, Failure> {
    let (begin, end) = markers(name);
    let refuse = match options.refuse.as_deref().map(Regex::new) {
        Some(Err(error)) => {
            let pattern = repr(options.refuse.as_deref().unwrap_or_default());
            writeln!(io.err, "package-proxy-merge: {pattern} is not a regex: {error}")?;
            return Ok(2);
        }
        Some(Ok(regex)) => Some(regex),
        None => None,
    };
    let mode = match options.mode.as_deref().map(|mode| u32::from_str_radix(mode, 8)) {
        Some(Err(_)) => {
            writeln!(io.err, "package-proxy-merge: {} is not an octal mode", repr(options.mode.as_deref().unwrap_or_default()))?;
            return Ok(2);
        }
        Some(Ok(mode)) => Some(mode),
        None => None,
    };

    let existing = if file.is_file() { universal(&fs::read_to_string(file)?) } else { String::new() };
    let lines = splitlines(&existing);
    let bounds = match block_bounds(&lines, &begin, &end) {
        Ok(bounds) => bounds,
        Err(error) => {
            writeln!(io.err, "package-proxy-merge: {}: {}", file.display(), error.0)?;
            return Ok(2);
        }
    };

    // The conflict check reads the file WITHOUT this setup's block, so a re-run
    // does not trip over the block it wrote the time before.
    if let Some(refuse) = refuse {
        for (number, line) in without_block(&lines, &begin, &end).iter().enumerate() {
            if refuse.is_match(line.strip_suffix('\n').unwrap_or(line)) {
                // Name the property only, never its value: a user's own password
                // or token must not reach the output.
                let named = line.split_once('=').map(|(key, _)| format!(" ({})", repr(strip(key)))).unwrap_or_default();
                let message = format!("package-proxy-merge: refusing to merge into {}: line {}{named} is outside the block and must not be overridden", file.display(), number + 1);
                writeln!(io.err, "{message}")?;
                return Ok(3);
            }
        }
    }

    let merged: Vec<String> = if options.remove {
        if bounds.is_none() {
            writeln!(io.out, "unchanged")?;
            return Ok(0);
        }
        without_block(&lines, &begin, &end)
    } else {
        let mut stdin = Vec::new();
        io.stdin.read_to_end(&mut stdin)?;
        let stdin = universal(&String::from_utf8(stdin).map_err(|e| Failure(e.to_string()))?);
        let body: Vec<String> = splitlines(&stdin).iter().map(|line| format!("{}\n", line.trim_end_matches(['\n', '\r', '\u{b}', '\u{c}', '\u{1c}', '\u{1d}', '\u{1e}', '\u{85}', '\u{2028}', '\u{2029}']))).collect();
        if body.is_empty() {
            writeln!(io.err, "package-proxy-merge: the block body is empty")?;
            return Ok(2);
        }
        let mut block = vec![format!("{begin}\n")];
        block.extend(body);
        block.push(format!("{end}\n"));
        match bounds {
            Some((start, stop)) => [&lines[..start], &block[..], &lines[stop + 1..]].concat(),
            None => append_block(&lines, &block),
        }
    };

    let merged_text = merged.concat();
    if merged_text == existing {
        writeln!(io.out, "unchanged")?;
        return Ok(0);
    }

    // Validate the merged result BEFORE anything is written, so a bad merge
    // never lands on disk in the first place.
    if let Some(problem) = options.validate.and_then(|kind| parse_problem(kind, &merged_text)) {
        writeln!(io.err, "package-proxy-merge: refusing to write {}: the merged result does not parse: {problem}", file.display())?;
        return Ok(3);
    }

    make_parent(file)?;
    let existed = file.is_file();
    if existed {
        let backup = with_suffix(file, ".bak-agent-kit");
        if !backup.is_file() {
            fs::write(backup, &existing)?;
        }
    }
    // An existing file keeps its own bits; a new one takes 0o666 so the umask applies.
    let create_mode = match (mode, existed) {
        (Some(mode), _) => mode,
        (None, true) => fs::metadata(file)?.permissions().mode() & 0o777,
        (None, false) => 0o666,
    };
    let mut handle = fs::OpenOptions::new().write(true).create(true).truncate(true).mode(create_mode).open(file)?;
    handle.write_all(merged_text.as_bytes())?;
    if mode.is_some() {
        // The open mode applies to new files only; this pins an existing one too.
        fs::set_permissions(file, fs::Permissions::from_mode(create_mode))?;
    }
    writeln!(io.out, "changed")?;
    Ok(0)
}
