//! One marked block in a file, the rest left alone (the `instruction-block.py` script).

use crate::text::{splitlines, strip};
use crate::{make_parent, Failure, Io};
use std::fs;
use std::path::Path;

/// The lines with every block named by begin/end replaced by `block`, or dropped when it is None.
/// The first block's place is where the new one goes; later ones are dropped.
pub fn rewrite(lines: &[String], begin: &str, end: &str, block: Option<&[String]>) -> Result<Vec<String>, Failure> {
    let mut out: Vec<String> = Vec::new();
    let mut placed = false;
    let mut index = 0;
    while index < lines.len() {
        if strip(&lines[index]) != begin {
            out.push(lines[index].clone());
            index += 1;
            continue;
        }
        let close = (index + 1..lines.len()).find(|i| strip(&lines[*i]) == end);
        let Some(close) = close else {
            return Err(Failure(format!("the block opened at line {} is never closed", index + 1)));
        };
        if let (Some(block), false) = (block, placed) {
            out.extend(block.iter().cloned());
            placed = true;
        }
        index = close + 1;
    }
    if let (Some(block), false) = (block, placed) {
        if out.last().is_some_and(|last| !strip(last).is_empty()) {
            out.push("\n".to_string());
        }
        out.extend(block.iter().cloned());
    }
    Ok(out)
}

/// `instruction-block.py FILE NAME [--remove]`: the body comes from stdin.
/// Exit 0 with `changed` or `unchanged`; 2 when the block is never closed.
pub fn instruction_block(file: &Path, name: &str, remove: bool, io: &mut Io) -> Result<i32, Failure> {
    let begin = format!("<!-- agent-kit {name} begin -->");
    let end = format!("<!-- agent-kit {name} end -->");
    let mut stdin = String::new();
    if !remove {
        let mut bytes = Vec::new();
        io.stdin.read_to_end(&mut bytes)?;
        stdin = crate::text::universal(&String::from_utf8(bytes).map_err(|e| Failure(e.to_string()))?);
    }
    let body = stdin.trim_end_matches('\n');
    let original = if file.exists() { fs::read_to_string(file)? } else { String::new() };
    let newline = if original.contains("\r\n") { "\r\n" } else { "\n" };
    let block: Option<Vec<String>> = (!remove).then(|| {
        let mut lines = vec![format!("{begin}{newline}")];
        lines.extend(body.split('\n').map(|line| format!("{line}{newline}")));
        lines.push(format!("{end}{newline}"));
        lines
    });
    let lines = splitlines(&original);
    let rewritten = match rewrite(&lines, &begin, &end, block.as_deref()) {
        Ok(lines) => lines,
        Err(error) => {
            writeln!(io.err, "{}: the agent-kit {name} block: {}; left alone", file.display(), error.0)?;
            return Ok(2);
        }
    };
    let text = rewritten.concat();
    if text == original {
        writeln!(io.out, "unchanged")?;
        return Ok(0);
    }
    make_parent(file)?;
    fs::write(file, text)?;
    writeln!(io.out, "changed")?;
    Ok(0)
}
