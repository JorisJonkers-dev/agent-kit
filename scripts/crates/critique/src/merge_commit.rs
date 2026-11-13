//! What a merge commit may change beyond the clean auto-merge and still add nothing of its own: a manifest's sha256
//! pins, which a rebuild rewrites. The node runner's `domain/merge-commit.ts`.

use crate::js_regex::Regex;

/// Whether a path names a manifest whose sha256 pins a merge may rewrite.
pub fn is_manifest(path: &str) -> bool {
    path == "manifest.yaml" || path.ends_with("/manifest.yaml")
}

/// Whether a merge's version of a manifest differs from the clean auto-merge's only in sha256 pins. The auto-merge may
/// hold conflicts, in git's default style: a conflict whose two sides differ only in their pins counts as resolved by
/// either. Every other line must be the same, so an entry added, dropped or edited by hand is not a pin.
pub fn differs_only_in_pins(auto_merged: &str, merged: &str) -> bool {
    resolve_pin_conflicts(&masked(auto_merged)).is_some_and(|resolved| resolved.join("\n") == masked(merged).join("\n"))
}

/// The lines of a text, each pin's digest replaced by the same placeholder.
fn masked(text: &str) -> Vec<String> {
    let pin = Regex::new(r"^(\s*(?:-\s+)?sha256:\s*)[0-9a-f]{64}(\s*)$", false).expect("the pin pattern");
    text.split('\n')
        .map(|line| {
            let chars: Vec<char> = line.chars().collect();
            match pin.find_at(&chars, 0) {
                Some(hit) => format!("{}<pin>{}", hit.group(&chars, 1).unwrap_or_default(), hit.group(&chars, 2).unwrap_or_default()),
                None => line.to_string(),
            }
        })
        .collect()
}

/// The lines with each conflict replaced by its one side, when both sides are the same; `None` when any conflict's
/// sides differ or a conflict is not closed.
fn resolve_pin_conflicts(lines: &[String]) -> Option<Vec<String>> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < lines.len() {
        if !lines[i].starts_with("<<<<<<<") {
            out.push(lines[i].clone());
            i += 1;
            continue;
        }
        let middle = lines.iter().skip(i + 1).position(|line| line == "=======")? + i + 1;
        let end = lines.iter().skip(middle + 1).position(|line| line.starts_with(">>>>>>>"))? + middle + 1;
        let (ours, theirs) = (&lines[i + 1..middle], &lines[middle + 1..end]);
        if ours.join("\n") != theirs.join("\n") {
            return None;
        }
        out.extend(ours.iter().cloned());
        i = end + 1;
    }
    Some(out)
}

/// The sections of a unified diff (`diff_commits`' form) for the files `paths` names, a rename's section by either
/// of its paths.
pub fn diff_of_paths(diff: &str, paths: &[String]) -> String {
    sections(diff)
        .into_iter()
        .filter(|section| {
            let header = section.split('\n').next().unwrap_or_default();
            paths.iter().any(|path| header.starts_with(&format!("diff --git a/{path} ")) || header.ends_with(&format!(" b/{path}")))
        })
        .collect()
}

/// `diff.split(/^(?=diff --git )/m)`: the text cut before each line that starts a file's section, a line starting
/// after `\n`, `\r`, U+2028 or U+2029; a section at the very start makes no empty first piece.
fn sections(diff: &str) -> Vec<&str> {
    let mut cuts = Vec::new();
    let mut previous: Option<char> = None;
    for (at, c) in diff.char_indices() {
        let line_start = previous.is_none_or(|before| matches!(before, '\n' | '\r' | '\u{2028}' | '\u{2029}'));
        if at > 0 && line_start && diff[at..].starts_with("diff --git ") {
            cuts.push(at);
        }
        previous = Some(c);
    }
    let mut pieces = Vec::new();
    let mut from = 0;
    for cut in cuts {
        pieces.push(&diff[from..cut]);
        from = cut;
    }
    pieces.push(&diff[from..]);
    pieces
}
