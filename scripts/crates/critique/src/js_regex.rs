//! The few JavaScript regular expressions the scans are written in, run with JavaScript's own
//! semantics: a backtracking match that tries alternatives in order, greedy and lazy quantifiers,
//! capture groups that stay unset when their branch did not match, `\s` as JavaScript's white
//! space, `\w` and `\b` over ASCII, and `^` at a line start in multiline mode (after `\n`, `\r`,
//! U+2028 or U+2029). No crate is added for it: the scans' patterns are a small, fixed set, and a
//! port that kept their text can be read against the node runner's source.
//!
//! Supported: literals, `.`, `\s \w \d \b` and escaped punctuation, classes with ranges and
//! `\s \w \d`, `^`, `$`, groups `( )` and `(?: )`, the lookaheads `(?= )` and `(?! )`, alternation,
//! `* + ? {n} {n,} {n,m}` with a lazy `?` suffix, the escapes `\n \r \t`, and the `i` flag over ASCII
//! letters (JavaScript folds no other letter onto an ASCII one). Any other escaped letter is refused,
//! so a pattern edited to use one fails when it is compiled. Text is matched as characters; an index
//! is a character index.

use model_gateway::text::is_js_whitespace;

#[derive(Debug)]
enum Item {
    Range(char, char),
    Space,
    Word,
    Digit,
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
    Any,
    Space,
    Word,
    Digit,
    Start,
    End,
    Boundary,
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { body: Box<Node>, min: usize, max: Option<usize>, greedy: bool },
    Group(usize, Box<Node>),
    NotAhead(Box<Node>),
    Ahead(Box<Node>),
}

/// A compiled pattern.
#[derive(Debug)]
pub struct Regex {
    root: Node,
    groups: usize,
    multiline: bool,
    ignore_case: bool,
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

/// The other case of an ASCII letter, which the `i` flag also accepts.
fn other_case(c: char) -> char {
    if c.is_ascii_lowercase() {
        c.to_ascii_uppercase()
    } else {
        c.to_ascii_lowercase()
    }
}

impl Set {
    fn contains(&self, c: char, ignore_case: bool) -> bool {
        let holds = |c: char| {
            self.items.iter().any(|item| match item {
                Item::Range(low, high) => (*low..=*high).contains(&c),
                Item::Space => is_js_whitespace(c),
                Item::Word => is_word(c),
                Item::Digit => c.is_ascii_digit(),
            })
        };
        let found = holds(c) || (ignore_case && c.is_ascii_alphabetic() && holds(other_case(c)));
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
            Some('{') => match self.braces() {
                Some(bounds) => bounds,
                None => return Ok(atom),
            },
            _ => return Ok(atom),
        };
        // JavaScript lets a lookahead be repeated (Annex B), but not `^`, `$` or `\b`.
        if matches!(atom, Node::Start | Node::End | Node::Boundary) {
            return Err("nothing to repeat".to_string());
        }
        self.at += 1;
        let greedy = !self.eat('?');
        Ok(Node::Repeat { body: Box::new(atom), min, max, greedy })
    }

    /// A `{n}`, `{n,}` or `{n,m}` quantifier starting at `{`, leaving `at` on its `}`; `None` when the brace
    /// starts no quantifier, which JavaScript then reads as a literal `{`.
    fn braces(&mut self) -> Option<(usize, Option<usize>)> {
        let digits = |from: usize| self.chars[from..].iter().take_while(|c| c.is_ascii_digit()).count();
        let number = |from: usize, count: usize| self.chars[from..from + count].iter().collect::<String>().parse::<usize>().ok();
        let start = self.at + 1;
        let low = digits(start);
        let min = number(start, low)?;
        let mut at = start + low;
        let max = if self.chars.get(at) == Some(&',') {
            let high = digits(at + 1);
            let max = if high == 0 { None } else { Some(number(at + 1, high)?) };
            at += 1 + high;
            max
        } else {
            Some(min)
        };
        (self.chars.get(at) == Some(&'}')).then(|| {
            self.at = at;
            (min, max)
        })
    }

    fn escape(&mut self) -> Result<char, String> {
        let c = self.peek().ok_or("a pattern ends with a backslash")?;
        self.at += 1;
        // `\n \r \t` are control characters and `\s \w \d \b` classes; any other letter would mean
        // something this engine does not do, so it is refused rather than read as the letter.
        match c {
            'n' => Ok('\n'),
            'r' => Ok('\r'),
            't' => Ok('\t'),
            's' | 'w' | 'd' | 'b' => Ok(c),
            letter if letter.is_ascii_alphanumeric() => Err(format!("\\{letter} is not supported")),
            other => Ok(other),
        }
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
                        '=' => Node::Ahead(Box::new(body)),
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
            ('.', false) => Ok(Node::Any),
            ('$', false) => Ok(Node::End),
            ('*' | '+' | '?', false) => Err("nothing to repeat".to_string()),
            (letter, true) => Ok(match letter {
                's' => Node::Space,
                'w' => Node::Word,
                'd' => Node::Digit,
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
                ('d', true) => Item::Digit,
                ('b', true) => return Err("\\b in a class is not supported".to_string()),
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
    ignore_case: bool,
    captures: Vec<Option<(usize, usize)>>,
}

type Next<'a, 'b> = &'a mut dyn FnMut(&mut Matching<'b>, usize) -> bool;

impl<'b> Matching<'b> {
    /// The width one character of a single-character node takes at `pos`, if it matches there.
    fn single(&self, node: &Node, pos: usize) -> Option<bool> {
        let c = self.text.get(pos).copied();
        match node {
            Node::Char(expected) => Some(c.is_some_and(|c| c == *expected || (self.ignore_case && c.is_ascii_alphabetic() && other_case(c) == *expected))),
            Node::Set(set) => Some(c.is_some_and(|c| set.contains(c, self.ignore_case))),
            Node::Any => Some(c.is_some_and(|c| !is_terminator(c))),
            Node::Space => Some(c.is_some_and(is_js_whitespace)),
            Node::Word => Some(c.is_some_and(is_word)),
            Node::Digit => Some(c.is_some_and(|c| c.is_ascii_digit())),
            _ => None,
        }
    }

    fn run(&mut self, node: &Node, pos: usize, next: Next<'_, 'b>) -> bool {
        match node {
            Node::Char(_) | Node::Set(_) | Node::Any | Node::Space | Node::Word | Node::Digit => self.single(node, pos) == Some(true) && next(self, pos + 1),
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
            Node::NotAhead(body) => {
                // A lookahead that fails keeps no capture it took on the way.
                let captures = self.captures.clone();
                let found = self.run(body, pos, &mut |_, _| true);
                self.captures = captures;
                !found && next(self, pos)
            }
            // JavaScript backtracks no further into a lookahead once it matched; its captures stay.
            Node::Ahead(body) => self.run(body, pos, &mut |_, _| true) && next(self, pos),
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
        Regex::with_flags(pattern, if multiline { "m" } else { "" })
    }

    /// Compiles a pattern with JavaScript flags: `m` multiline and `i` ignore case; `g` changes nothing here.
    pub fn with_flags(pattern: &str, flags: &str) -> Result<Regex, String> {
        if let Some(flag) = flags.chars().find(|flag| !matches!(flag, 'g' | 'i' | 'm')) {
            return Err(format!("the flag {flag} is not supported"));
        }
        let chars: Vec<char> = pattern.chars().collect();
        let mut parser = Parser { chars: &chars, at: 0, groups: 0 };
        let root = parser.alternation()?;
        if parser.at < chars.len() {
            return Err("a group is closed that was not opened".to_string());
        }
        Ok(Regex { root, groups: parser.groups, multiline: flags.contains('m'), ignore_case: flags.contains('i') })
    }

    /// How many capture groups the pattern has.
    pub fn group_count(&self) -> usize {
        self.groups
    }

    /// The first match that starts at or after `from`.
    pub fn find_at(&self, text: &[char], from: usize) -> Option<Match> {
        (from..=text.len()).find_map(|start| {
            let mut matching = Matching { text, multiline: self.multiline, ignore_case: self.ignore_case, captures: vec![None; self.groups] };
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
