//! A port of Python's `shlex.shlex(command, posix=True, punctuation_chars=";&|()")`
//! with `whitespace_split`, quirks included (a `#` starts a comment, a newline is
//! plain whitespace, a quoted `;` still separates), and of `shlex.quote`.

const PUNCT: &str = ";&|()";

#[derive(Clone, Copy)]
enum State {
    Space,
    Word,
    Punct,
    Quote(char),
    EscapeWord,
    EscapeQuote,
}

/// The tokens of `command`, or `None` where shlex raises (an unclosed quote or a
/// trailing backslash).
pub fn tokens(command: &str) -> Option<Vec<String>> {
    let chars: Vec<char> = command.chars().collect();
    let mut out = Vec::new();
    let mut i = 0;
    let mut state = State::Space;
    let mut token = String::new();
    loop {
        let c = chars.get(i).copied();
        let mut emit = false;
        match (state, c) {
            (State::Space, None) => break,
            (State::Space, Some(c)) => {
                i += 1;
                if is_space(c) {
                } else if c == '#' {
                    i = skip_line(&chars, i);
                } else if PUNCT.contains(c) {
                    token.push(c);
                    state = State::Punct;
                } else if c == '\'' || c == '"' {
                    state = State::Quote(c);
                } else if c == '\\' {
                    state = State::EscapeWord;
                } else {
                    token.push(c);
                    state = State::Word;
                }
            }
            (State::Word, None) | (State::Punct, None) => emit = true,
            (State::Word, Some(c)) => {
                if is_space(c) {
                    i += 1;
                    emit = true;
                } else if c == '#' {
                    i = skip_line(&chars, i + 1);
                    emit = true;
                } else if c == '\'' || c == '"' {
                    i += 1;
                    state = State::Quote(c);
                } else if c == '\\' {
                    i += 1;
                    state = State::EscapeWord;
                } else if PUNCT.contains(c) {
                    emit = true;
                } else {
                    i += 1;
                    token.push(c);
                }
            }
            (State::Punct, Some(c)) => {
                if is_space(c) {
                    i += 1;
                    emit = true;
                } else if c == '#' {
                    i = skip_line(&chars, i + 1);
                    emit = true;
                } else if PUNCT.contains(c) {
                    i += 1;
                    token.push(c);
                } else {
                    emit = true;
                }
            }
            (State::Quote(_), None) | (State::EscapeWord, None) | (State::EscapeQuote, None) => {
                return None
            }
            (State::Quote(q), Some(c)) => {
                i += 1;
                if c == q {
                    state = State::Word;
                } else if q == '"' && c == '\\' {
                    state = State::EscapeQuote;
                } else {
                    token.push(c);
                }
            }
            (State::EscapeWord, Some(c)) => {
                i += 1;
                token.push(c);
                state = State::Word;
            }
            (State::EscapeQuote, Some(c)) => {
                i += 1;
                if c != '"' && c != '\\' {
                    token.push('\\');
                }
                token.push(c);
                state = State::Quote('"');
            }
        }
        if emit {
            out.push(std::mem::take(&mut token));
            state = State::Space;
        }
    }
    Some(out)
}

/// shlex's whitespace: space, tab, carriage return, newline.
fn is_space(c: char) -> bool {
    matches!(c, ' ' | '\t' | '\r' | '\n')
}

/// shlex's `readline()` after a comment character: everything up to and
/// including the next newline.
fn skip_line(chars: &[char], from: usize) -> usize {
    chars[from..]
        .iter()
        .position(|&c| c == '\n')
        .map_or(chars.len(), |n| from + n + 1)
}

/// The simple commands of `command`, split on `;`, `&`, `|`, `(` and `)`.
pub fn segments(command: &str) -> Vec<Vec<String>> {
    let mut out = Vec::new();
    let mut cur: Vec<String> = Vec::new();
    for t in tokens(command).unwrap_or_default() {
        if !t.is_empty() && t.chars().all(|c| PUNCT.contains(c)) {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(t);
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// `shlex.quote`.
pub fn quote(s: &str) -> String {
    if s.is_empty() {
        return "''".to_string();
    }
    if s.chars()
        .all(|c| c.is_ascii_alphanumeric() || "_@%+=:,./-".contains(c))
    {
        return s.to_string();
    }
    format!("'{}'", s.replace('\'', "'\"'\"'"))
}
