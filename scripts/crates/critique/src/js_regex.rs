//! The few JavaScript regular expressions the scans are written in, run with JavaScript's own
//! semantics: a backtracking match that tries alternatives in order, greedy and lazy quantifiers,
//! capture groups that stay unset when their branch did not match, `\s` as JavaScript's white
//! space, `\w` and `\b` over ASCII, and `^` at a line start in multiline mode (after `\n`, `\r`,
//! U+2028 or U+2029). No crate is added for it: the scans' patterns are a small, fixed set, and a
//! port that kept their text can be read against the node runner's source.
//!
//! Supported: literals, `.`, `\s \w \d \b` and escaped punctuation, classes with ranges and
//! `\s \w \d`, `^`, `$`, groups `( )` and `(?: )`, the lookaheads `(?= )` and `(?! )`, the negative lookbehind `(?<! )`
//! over one character, alternation,
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
    Not(Box<Item>),
    /// The folded forms of a range's members under `i`, sorted.
    Folded(Vec<char>),
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
    NotBoundary,
    Seq(Vec<Node>),
    Alt(Vec<Node>),
    Repeat { body: Box<Node>, min: usize, max: Option<usize>, greedy: bool },
    Group(usize, Box<Node>),
    NotAhead(Box<Node>),
    /// `(?<! )` over one character: the character before is not one the body matches.
    NotBehind(Box<Node>),
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

/// JavaScript's `Canonicalize` for the `i` flag without `u`: the character's upper case when that is one character,
/// except that nothing beyond ASCII folds onto ASCII. Text is matched by characters here, where JavaScript matches
/// UTF-16 code units, so a character beyond U+FFFF (two code units there) folds to nothing else.
fn canonicalize(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(folded), None) if c <= '\u{ffff}' && (c.is_ascii() || !folded.is_ascii()) => folded,
        _ => c,
    }
}

impl Item {
    fn holds(&self, c: char) -> bool {
        match self {
            Item::Range(low, high) => (*low..=*high).contains(&c),
            Item::Space => is_js_whitespace(c),
            Item::Word => is_word(c),
            Item::Digit => c.is_ascii_digit(),
            Item::Not(item) => !item.holds(c),
            Item::Folded(folded) => folded.binary_search(&canonicalize(c)).is_ok(),
        }
    }
}

/// The folded forms of every character from `low` to `high`, sorted and without repeats.
fn folded_range(low: char, high: char) -> Vec<char> {
    let mut folded: Vec<char> = (low..=high).map(canonicalize).collect();
    folded.sort_unstable();
    folded.dedup();
    folded
}

impl Set {
    fn contains(&self, c: char, ignore_case: bool) -> bool {
        let holds = |c: char| self.items.iter().any(|item| item.holds(c));
        // Under `i` a one-character item matches what folds to the same character; a range was folded when it was read.
        let single = |c: char| self.items.iter().any(|item| matches!(item, Item::Range(low, high) if low == high && canonicalize(*low) == canonicalize(c)));
        let found = holds(c) || (ignore_case && single(c));
        found != self.negated
    }
}

struct Parser<'a> {
    chars: &'a [char],
    at: usize,
    groups: usize,
    ignore_case: bool,
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
        if matches!(atom, Node::Start | Node::End | Node::Boundary | Node::NotBoundary) {
            return Err("nothing to repeat".to_string());
        }
        if max.is_some_and(|max| max < min) {
            return Err("numbers out of order in {} quantifier".to_string());
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

    /// What follows a backslash: a character it stands for, or one of the classes and assertions `\\s \\S \\w \\W
    /// \\d \\D \\b \\B`. Any other letter or digit would mean something this engine does not do, or something
    /// JavaScript reads differently, so it is refused rather than read as itself.
    fn escape(&mut self) -> Result<Piece, String> {
        let c = self.peek().ok_or("a pattern ends with a backslash")?;
        self.at += 1;
        Ok(match c {
            'n' => Piece::Literal('\n'),
            'r' => Piece::Literal('\r'),
            't' => Piece::Literal('\t'),
            'f' => Piece::Literal('\u{c}'),
            'v' => Piece::Literal('\u{b}'),
            '0' if !self.peek().is_some_and(|next| next.is_ascii_digit()) => Piece::Literal('\0'),
            'x' => self.hex(2).map_or(Piece::Literal('x'), Piece::Literal),
            'u' => self.hex(4).map_or(Piece::Literal('u'), Piece::Literal),
            's' | 'S' | 'w' | 'W' | 'd' | 'D' | 'b' | 'B' => Piece::Class(c),
            letter if letter.is_ascii_alphanumeric() => return Err(format!("\\{letter} is not supported")),
            other => Piece::Literal(other),
        })
    }

    /// The character `count` hex digits name, taken when all of them follow; otherwise JavaScript reads the letter
    /// before them as itself. A `\\u` escape that names half a surrogate pair is refused, since text is matched as
    /// characters here.
    fn hex(&mut self, count: usize) -> Option<char> {
        let digits: String = self.chars.get(self.at..self.at + count)?.iter().collect();
        let value = u32::from_str_radix(&digits, 16).ok().filter(|_| digits.chars().all(|c| c.is_ascii_hexdigit()))?;
        let c = char::from_u32(value)?;
        self.at += count;
        Some(c)
    }

    fn atom(&mut self) -> Result<Node, String> {
        match self.escape_or_next()? {
            Piece::Plain('(') => {
                let node = if self.eat('?') {
                    let kind = self.peek().ok_or("a group is not closed")?;
                    self.at += 1;
                    if kind == '<' && !self.eat('!') {
                        return Err("(?< is not supported".to_string());
                    }
                    let body = self.alternation()?;
                    match kind {
                        '<' => Node::NotBehind(Box::new(single_character(body)?)),
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
            Piece::Plain('[') => self.set(),
            Piece::Plain('^') => Ok(Node::Start),
            Piece::Plain('.') => Ok(Node::Any),
            Piece::Plain('$') => Ok(Node::End),
            Piece::Plain('*' | '+' | '?') => Err("nothing to repeat".to_string()),
            Piece::Class(letter) => Ok(match letter {
                's' => Node::Space,
                'w' => Node::Word,
                'd' => Node::Digit,
                'b' => Node::Boundary,
                'B' => Node::NotBoundary,
                other => Node::Set(Set { negated: true, items: vec![class_item(other.to_ascii_lowercase())] }),
            }),
            Piece::Plain(other) | Piece::Literal(other) => Ok(Node::Char(other)),
        }
    }

    /// The next character, read as a backslash escape when it is one.
    fn escape_or_next(&mut self) -> Result<Piece, String> {
        let c = self.peek().ok_or("a pattern ends too soon")?;
        self.at += 1;
        if c == '\\' {
            self.escape()
        } else {
            Ok(Piece::Plain(c))
        }
    }

    fn set(&mut self) -> Result<Node, String> {
        let negated = self.eat('^');
        let mut items = Vec::new();
        loop {
            let piece = self.escape_or_next().map_err(|error| if error == "a pattern ends too soon" { "a class is not closed".to_string() } else { error })?;
            let low = match piece {
                Piece::Plain(']') => return Ok(Node::Set(Set { negated, items })),
                // In a class `\\b` is the backspace.
                Piece::Class('b') => '\u{8}',
                Piece::Class('B') => return Err("\\B in a class is not supported".to_string()),
                Piece::Class(letter) if letter.is_ascii_lowercase() => {
                    items.push(class_item(letter));
                    continue;
                }
                Piece::Class(letter) => {
                    items.push(Item::Not(Box::new(class_item(letter.to_ascii_lowercase()))));
                    continue;
                }
                Piece::Plain(c) | Piece::Literal(c) => c,
            };
            let range = self.peek() == Some('-') && self.chars.get(self.at + 1).is_some_and(|next| *next != ']');
            if !range {
                items.push(Item::Range(low, low));
                continue;
            }
            self.at += 1;
            let high = match self.escape_or_next()? {
                Piece::Plain(c) | Piece::Literal(c) => c,
                Piece::Class('b') => '\u{8}',
                Piece::Class(_) => return Err("a class escape cannot end a range".to_string()),
            };
            if high < low {
                return Err("range out of order in character class".to_string());
            }
            // Under `i` JavaScript matches a character whose folded form is the folded form of some member.
            items.push(if self.ignore_case { Item::Folded(folded_range(low, high)) } else { Item::Range(low, high) });
        }
    }
}

/// One piece of a pattern: a character as written, one a backslash escape stands for, or a class escape's letter.
enum Piece {
    Plain(char),
    Literal(char),
    Class(char),
}

/// The class item of `\\s`, `\\w` or `\\d`.
fn class_item(letter: char) -> Item {
    match letter {
        's' => Item::Space,
        'w' => Item::Word,
        _ => Item::Digit,
    }
}

/// The one character node a lookbehind's body is, so the engine looks one character back.
fn single_character(body: Node) -> Result<Node, String> {
    match body {
        Node::Seq(mut items) if items.len() == 1 => single_character(items.remove(0)),
        node @ (Node::Char(_) | Node::Set(_) | Node::Any | Node::Word | Node::Digit | Node::Space) => Ok(node),
        _ => Err("a lookbehind over more than one character is not supported".to_string()),
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
            Node::Char(expected) => Some(c.is_some_and(|c| c == *expected || (self.ignore_case && canonicalize(c) == canonicalize(*expected)))),
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
            Node::Boundary | Node::NotBoundary => {
                let before = pos > 0 && is_word(self.text[pos - 1]);
                let after = self.text.get(pos).is_some_and(|c| is_word(*c));
                (before != after) == matches!(node, Node::Boundary) && next(self, pos)
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
            Node::NotBehind(body) => {
                let before = pos > 0 && self.single(body, pos - 1) == Some(true);
                !before && next(self, pos)
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
        let ignore_case = flags.contains('i');
        let mut parser = Parser { chars: &chars, at: 0, groups: 0, ignore_case };
        let root = parser.alternation()?;
        if parser.at < chars.len() {
            return Err("a group is closed that was not opened".to_string());
        }
        Ok(Regex { root, groups: parser.groups, multiline: flags.contains('m'), ignore_case })
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
