//! The few JavaScript regular expressions the scans are written in, run with JavaScript's own
//! semantics: a backtracking match that tries alternatives in order, greedy and lazy quantifiers,
//! capture groups that stay unset when their branch did not match, `\s` as JavaScript's white
//! space, `\w` and `\b` over ASCII, and `^` at a line start in multiline mode (after `\n`, `\r`,
//! U+2028 or U+2029). No crate is added for it: the scans' patterns are a small, fixed set, and a
//! port that kept their text can be read against the node runner's source.
//!
//! Supported: literals, `\s \w \b` and escaped characters, classes with ranges and `\s \w`, `^`, `$`,
//! groups `( )` and `(?: )`, the negative lookahead `(?! )`, alternation and `* + ?` with a lazy
//! `?` suffix, and the escape `\n`. Text is matched as characters; an index is a character index.

use model_gateway::text::is_js_whitespace;

#[derive(Debug)]
enum Item {
    Range(char, char),
    Space,
    Word,
}

#[derive(Debug)]
struct Set {
    negated: bool,
    items: Vec<Item>,
}

#[derive(Debug)]
enum Node {
    Char(char),
    Set(Set),
    Space,
    Word,
    Start,
    End,
    Boundary,
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { body: Box<Node>, min: usize, max: Option<usize>, greedy: bool },
    Group(usize, Box<Node>),
    NotAhead(Box<Node>),
}

/// A compiled pattern.
#[derive(Debug)]
pub struct Regex {
    root: Node,
    groups: usize,
    multiline: bool,
}

/// One match: where it starts and ends, and what each capture group took (`None` for a group that did not take part).
#[derive(Debug)]
pub struct Match {
    pub start: usize,
    pub end: usize,
    groups: Vec<Option<(usize, usize)>>,
}

impl Match {
    /// The text of capture group `index` (from 1), if it took part.
    pub fn group(&self, text: &[char], index: usize) -> Option<String> {
        self.groups[index - 1].map(|(from, to)| text[from..to].iter().collect())
    }
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

fn is_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

impl Set {
    fn contains(&self, c: char) -> bool {
        let found = self.items.iter().any(|item| match item {
            Item::Range(low, high) => (*low..=*high).contains(&c),
            Item::Space => is_js_whitespace(c),
            Item::Word => is_word(c),
        });
        found != self.negated
    }
}

struct Parser<'a> {
    chars: &'a [char],
    at: usize,
    groups: usize,
}

impl Parser<'_> {
    fn peek(&self) -> Option<char> {
        self.chars.get(self.at).copied()
    }

    fn eat(&mut self, c: char) -> bool {
        let found = self.peek() == Some(c);
        self.at += usize::from(found);
        found
    }

    fn alternation(&mut self) -> Result<Node, String> {
        let mut branches = vec![self.sequence()?];
        while self.eat('|') {
            branches.push(self.sequence()?);
        }
        Ok(if branches.len() == 1 { branches.remove(0) } else { Node::Alt(branches) })
    }

    fn sequence(&mut self) -> Result<Node, String> {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom()?;
            items.push(self.quantified(atom)?);
        }
        Ok(Node::Seq(items))
    }

    fn quantified(&mut self, atom: Node) -> Result<Node, String> {
        let (min, max) = match self.peek() {
            Some('*') => (0, None),
            Some('+') => (1, None),
            Some('?') => (0, Some(1)),
            _ => return Ok(atom),
        };
        self.at += 1;
        let greedy = !self.eat('?');
        Ok(Node::Repeat { body: Box::new(atom), min, max, greedy })
    }

    fn escape(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or("a pattern ends with a backslash")?;
        self.at += 1;
        // `\n` is the line feed; the other letters are `\s \w \b` or the letter itself.
        Ok(if c == 'n' { '\n' } else { c })
    }

    fn atom(&mut self) -> Result<Node, String> {
        let c = self.escape_or_next()?;
        match c {
            ('(', false) => {
                let node = if self.eat('?') {
                    let kind = self.escape()?;
                    let body = self.alternation()?;
                    match kind {
                        ':' => body,
                        '!' => Node::NotAhead(Box::new(body)),
                        other => return Err(format!("(?{other} is not supported")),
                    }
                } else {
                    self.groups += 1;
                    let number = self.groups;
                    Node::Group(number, Box::new(self.alternation()?))
                };
                if !self.eat(')') {
                    return Err("a group is not closed".to_string());
                }
                Ok(node)
            }
            ('[', false) => self.set(),
            ('^', false) => Ok(Node::Start),
            ('$', false) => Ok(Node::End),
            ('*' | '+' | '?', false) => Err("nothing to repeat".to_string()),
            (letter, true) => Ok(match letter {
                's' => Node::Space,
                'w' => Node::Word,
                'b' => Node::Boundary,
                other => Node::Char(other),
            }),
            (other, false) => Ok(Node::Char(other)),
        }
    }

    /// The next character, and whether a backslash escaped it.
    fn escape_or_next(&mut self) -> Result<(char, bool), String> {
        let c = self.peek().ok_or("a pattern ends too soon")?;
        self.at += 1;
        if c == '\\' {
            Ok((self.escape()?, true))
        } else {
            Ok((c, false))
        }
    }

    fn set(&mut self) -> Result<Node, String> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            let (c, escaped) = self.escape_or_next().map_err(|_| "a class is not closed".to_string())?;
            let item = match (c, escaped) {
                (']', false) => return Ok(Node::Set(Set { negated, items })),
                ('s', true) => Item::Space,
                ('w', true) => Item::Word,
                (low, _) => {
                    let range = self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|next| *next != ']');
                    if range {
                        self.at += 1;
                        let (high, _) = self.escape_or_next()?;
                        Item::Range(low, high)
                    } else {
                        Item::Range(low, low)
                    }
                }
            };
            items.push(item);
        }
    }
}

struct Matching<'a> {
    text: &'a [char],
    multiline: bool,
    captures: Vec<Option<(usize, usize)>>,
}

type Next<'a, 'b> = &'a mut dyn FnMut(&mut Matching<'b>, usize) -> bool;

impl<'b> Matching<'b> {
    /// The width one character of a single-character node takes at `pos`, if it matches there.
    fn single(&self, node: &Node, pos: usize) -> Option<bool> {
        let c = self.text.get(pos).copied();
        match node {
            Node::Char(expected) => Some(c == Some(*expected)),
            Node::Set(set) => Some(c.is_some_and(|c| set.contains(c))),
            Node::Space => Some(c.is_some_and(is_js_whitespace)),
            Node::Word => Some(c.is_some_and(is_word)),
            _ => None,
        }
    }

    fn run(&mut self, node: &Node, pos: usize, next: Next<'_, 'b>) -> bool {
        match node {
            Node::Char(_) | Node::Set(_) | Node::Space | Node::Word => self.single(node, pos) == Some(true) && next(self, pos + 1),
            Node::Start => {
                let at_start = pos == 0 || (self.multiline && is_terminator(self.text[pos - 1]));
                at_start && next(self, pos)
            }
            Node::End => {
                let at_end = pos == self.text.len() || (self.multiline && is_terminator(self.text[pos]));
                at_end && next(self, pos)
            }
            Node::Boundary => {
                let before = pos > 0 && is_word(self.text[pos - 1]);
                let after = self.text.get(pos).is_some_and(|c| is_word(*c));
                before != after && next(self, pos)
            }
            Node::Seq(items) => self.sequence(items, pos, next),
            Node::Alt(branches) => branches.iter().any(|branch| self.run(branch, pos, next)),
            Node::Group(number, body) => self.run(body, pos, &mut |this, end| {
                let previous = this.captures[number - 1];
                this.captures[number - 1] = Some((pos, end));
                next(this, end) || {
                    this.captures[number - 1] = previous;
                    false
                }
            }),
            Node::NotAhead(body) => !self.run(body, pos, &mut |_, _| true) && next(self, pos),
            Node::Repeat { body, min, max, greedy } => self.repeat(body, (*min, *max, *greedy), pos, next),
        }
    }

    fn sequence(&mut self, items: &[Node], pos: usize, next: Next<'_, 'b>) -> bool {
        match items.split_first() {
            None => next(self, pos),
            Some((first, rest)) => self.run(first, pos, &mut |this, at| this.sequence(rest, at, next)),
        }
    }

    fn repeat(&mut self, body: &Node, (min, max, greedy): (usize, Option<usize>, bool), pos: usize, next: Next<'_, 'b>) -> bool {
        if self.single(body, pos).is_some() {
            // One character a time with no recursion, so a long run of one class costs no stack.
            let mut count = 0;
            while max.is_none_or(|limit| count < limit) && self.single(body, pos + count) == Some(true) {
                count += 1;
            }
            let counts: Vec<usize> = if greedy { (min..=count).rev().collect() } else { (min..=count).collect() };
            return counts.into_iter().any(|taken| next(self, pos + taken));
        }
        self.repeat_body(body, (min, max, greedy), 0, pos, next)
    }

    fn repeat_body(&mut self, body: &Node, (min, max, greedy): (usize, Option<usize>, bool), done: usize, pos: usize, next: Next<'_, 'b>) -> bool {
        let more = max.is_none_or(|limit| done < limit);
        let again = |this: &mut Matching<'b>, next: Next<'_, 'b>| {
            more && this.run(body, pos, &mut |this, end| (end > pos || done < min) && this.repeat_body(body, (min, max, greedy), done + 1, end, next))
        };
        if done < min {
            return again(self, next);
        }
        if greedy {
            again(self, next) || next(self, pos)
        } else {
            next(self, pos) || again(self, next)
        }
    }
}

impl Regex {
    /// Compiles a pattern; `multiline` is the `m` flag.
    pub fn new(pattern: &str, multiline: bool) -> Result<Regex, String> {
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = Parser { chars: &chars, at: 0, groups: 0 };
        let root = parser.alternation()?;
        if parser.at < chars.len() {
            return Err("a group is closed that was not opened".to_string());
        }
        Ok(Regex { root, groups: parser.groups, multiline })
    }

    /// The first match that starts at or after `from`.
    pub fn find_at(&self, text: &[char], from: usize) -> Option<Match> {
        (from..=text.len()).find_map(|start| {
            let mut matching = Matching { text, multiline: self.multiline, captures: vec![None; self.groups] };
            let mut end = 0;
            let found = matching.run(&self.root, start, &mut |_, at| {
                end = at;
                true
            });
            found.then(|| Match { start, end, groups: matching.captures })
        })
    }

    /// Every match, as the `g` flag finds them: each search starts where the last match ended.
    pub fn find_all(&self, text: &[char]) -> Vec<Match> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(hit) = self.find_at(text, from) {
            from = hit.end.max(hit.start + 1);
            found.push(hit);
        }
        found
    }
}
