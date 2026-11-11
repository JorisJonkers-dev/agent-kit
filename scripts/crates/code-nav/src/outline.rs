//! Outline and expand: the symbols a file declares, and one symbol's source.
//!
//! Line-based and language-agnostic: no parser and no index. A line declares a
//! symbol when, after leading whitespace and any modifiers, the next word is a
//! declaration keyword. The range runs to the next line at the same or a
//! shallower indentation (including the line that closes the block), and
//! upward over the attributes or decorators directly above the declaration.

/// One declaration in a file.
#[derive(Debug, Clone, PartialEq)]
pub struct Symbol {
    /// The declared name; for `impl`, the type or trait it is for.
    pub name: String,
    /// The declaring keyword: `fn`, `class`, `impl` and so on.
    pub kind: String,
    /// The declaring line, 1-based.
    pub line: usize,
    /// The first line of the range, 1-based: the declaration plus the
    /// attribute or decorator lines directly above it.
    pub start: usize,
    /// The last line of the range, 1-based and inclusive.
    pub end: usize,
}

/// Words that may stand between the indentation and the declaration keyword.
const MODIFIERS: [&str; 13] = [
    "pub", "export", "default", "async", "static", "public", "private", "protected", "final", "abstract", "unsafe", "extern",
    "const",
];

/// Words that declare a symbol.
const KEYWORDS: [&str; 12] = ["fn", "def", "function", "func", "class", "struct", "enum", "trait", "impl", "interface", "type", "mod"];

/// Every symbol `text` declares, in file order.
pub fn symbols(text: &str) -> Vec<Symbol> {
    let lines: Vec<&str> = text.lines().collect();
    let mut found = Vec::new();
    for (i, line) in lines.iter().enumerate() {
        let Some((kind, name)) = declaration(line) else { continue };
        let indent = indent_of(line);
        found.push(Symbol { name, kind, line: i + 1, start: start_of(&lines, i, indent), end: end_of(&lines, i, indent) });
    }
    found
}

/// Every symbol named `name`, each with its source lines joined by newlines.
pub fn expand(text: &str, name: &str) -> Vec<(Symbol, String)> {
    let lines: Vec<&str> = text.lines().collect();
    symbols(text)
        .into_iter()
        .filter(|s| s.name == name)
        .map(|s| {
            let source = lines[s.start - 1..s.end].join("\n");
            (s, source)
        })
        .collect()
}

/// The kind and name `line` declares, if any.
fn declaration(line: &str) -> Option<(String, String)> {
    let mut rest = line.trim_start();
    loop {
        let word = word_of(rest);
        if word == "pub" && rest[word.len()..].starts_with('(') {
            rest = skip_parens(&rest[word.len()..]).trim_start();
            continue;
        }
        if !MODIFIERS.contains(&word) {
            break;
        }
        let after = rest[word.len()..].trim_start();
        // `const` is a modifier only in `const fn`.
        if word == "const" && word_of(after) != "fn" {
            break;
        }
        rest = after;
    }
    let kind = word_of(rest);
    if !KEYWORDS.contains(&kind) {
        return None;
    }
    let after = rest[kind.len()..].trim_start();
    let name = if kind == "impl" { impl_name(after) } else { word_of(after).to_string() };
    if name.is_empty() {
        return None;
    }
    Some((kind.to_string(), name))
}

/// The name of an `impl`: its generics skipped, the rest up to the body.
fn impl_name(after: &str) -> String {
    let rest = match after.strip_prefix('<') {
        Some(tail) => skip_generics(tail),
        None => after,
    };
    rest.find('{').map_or(rest, |at| &rest[..at]).trim().to_string()
}

/// `tail` (the text after an opening `<`) once the matching `>` is passed,
/// or empty when there is none.
fn skip_generics(tail: &str) -> &str {
    let mut depth = 1;
    for (at, c) in tail.char_indices() {
        match c {
            '<' => depth += 1,
            '>' => depth -= 1,
            _ => {}
        }
        if depth == 0 {
            return &tail[at + 1..];
        }
    }
    ""
}

/// `text` after the first `)`, or empty when there is none.
fn skip_parens(text: &str) -> &str {
    text.split_once(')').map_or("", |(_, rest)| rest)
}

/// The identifier at the start of `text`, empty when it starts with anything else.
fn word_of(text: &str) -> &str {
    let end = text.find(|c: char| !is_ident(c)).unwrap_or(text.len());
    &text[..end]
}

fn is_ident(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

/// The count of leading whitespace characters.
fn indent_of(line: &str) -> usize {
    line.chars().take_while(|c| c.is_whitespace()).count()
}

/// The first line of the range: the declaration and the attributes or
/// decorators directly above it, at the same indentation.
fn start_of(lines: &[&str], i: usize, indent: usize) -> usize {
    let mut start = i;
    while start > 0 {
        let above = lines[start - 1];
        let trimmed = above.trim_start();
        if indent_of(above) != indent || !(trimmed.starts_with("#[") || trimmed.starts_with('@')) {
            break;
        }
        start -= 1;
    }
    start + 1
}

/// The last line of the range: the last non-blank line before the next line at
/// the same or a shallower indentation, that line included when it closes the
/// block.
fn end_of(lines: &[&str], i: usize, indent: usize) -> usize {
    let mut end = signature_end(lines, i, indent);
    for (j, line) in lines.iter().enumerate().skip(end) {
        if line.trim().is_empty() {
            continue;
        }
        if indent_of(line) > indent {
            end = j + 1;
            continue;
        }
        // Only a closer at the declaration's own indentation belongs to it;
        // a shallower one closes the enclosing block.
        end = if indent_of(line) == indent && closes(line) { j + 1 } else { j };
        break;
    }
    while end > i + 1 && lines[end - 1].trim().is_empty() {
        end -= 1;
    }
    end
}

/// The last line (1-based) of the declaration's signature: it runs on while
/// brackets are open, then over `where` and `->` lines and a lone `{`, with the
/// indented lines of a `where` clause, so a signature that rustfmt or a
/// formatter wraps with its closing line at the declaration's indentation is
/// not mistaken for the end of the block.
fn signature_end(lines: &[&str], i: usize, indent: usize) -> usize {
    let mut depth: i32 = 0;
    let mut end = i;
    for (j, line) in lines.iter().enumerate().skip(i) {
        depth += line.chars().map(|c| match c { '(' | '[' => 1, ')' | ']' => -1, _ => 0 }).sum::<i32>();
        end = j;
        if depth <= 0 {
            break;
        }
    }
    let mut in_where = false;
    for (j, line) in lines.iter().enumerate().skip(end + 1) {
        let trimmed = line.trim_start();
        if indent_of(line) <= indent && (word_of(trimmed) == "where" || trimmed.starts_with('{') || trimmed.starts_with("->")) {
            in_where = true;
            end = j;
        } else if in_where && indent_of(line) > indent {
            end = j;
        } else {
            break;
        }
    }
    end + 1
}

/// True when `line` closes the block above it.
fn closes(line: &str) -> bool {
    let trimmed = line.trim_start();
    matches!(trimmed.chars().next(), Some('}' | ')' | ']')) || word_of(trimmed) == "end"
}
