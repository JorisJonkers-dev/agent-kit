//! A small backtracking matcher for the Python `re` patterns the two helpers
//! use. The crate has no regex dependency (the lockfile carries none), and
//! the helpers' behaviour on odd input (a heading whose `\s+` runs across
//! lines, a lazy `.+?`) is defined by Python's backtracking, so the patterns
//! are ported as written rather than rewritten as string code.
//!
//! Supported: literals and escapes, `.`, `[...]` sets (with `\w \s \d`),
//! `\s \S \w \W \d \b`, `^` `$`, groups `(...)` and `(?:...)`, `(?<!X)` for a
//! one-character X, alternation, and the greedy and lazy `* + ? {m,n}`.

/// Flags of one pattern: `re.I`, `re.M`, `re.S`.
#[derive(Clone, Copy, Default)]
pub struct Flags {
    pub ignore_case: bool,
    pub multiline: bool,
    pub dot_all: bool,
}

pub const NONE: Flags = Flags {
    ignore_case: false,
    multiline: false,
    dot_all: false,
};
pub const I: Flags = Flags {
    ignore_case: true,
    multiline: false,
    dot_all: false,
};
pub const M: Flags = Flags {
    ignore_case: false,
    multiline: true,
    dot_all: false,
};
pub const S: Flags = Flags {
    ignore_case: false,
    multiline: false,
    dot_all: true,
};

#[derive(Clone, Copy)]
enum Class {
    Word,
    Space,
    Digit,
}

#[derive(Clone)]
struct Set {
    negate: bool,
    chars: Vec<char>,
    ranges: Vec<(char, char)>,
    classes: Vec<(Class, bool)>,
}

#[derive(Clone)]
enum Node {
    Char(char),
    Any,
    Set(Set),
    Start,
    End,
    WordBoundary,
    NotBehind(Box<Node>),
    Group(Box<Node>, Option<usize>),
    Alt(Vec<Node>),
    Seq(Vec<Node>),
    Repeat {
        node: Box<Node>,
        min: usize,
        max: usize,
        greedy: bool,
    },
}

/// One match: the span of the whole match and of each capture group.
pub struct Match {
    pub start: usize,
    pub end: usize,
    spans: Vec<Option<(usize, usize)>>,
}

impl Match {
    pub fn span(&self, group: usize) -> Option<(usize, usize)> {
        self.spans.get(group).copied().flatten()
    }
    /// The text of one group, or `None` when it did not take part.
    pub fn group(&self, text: &[char], group: usize) -> Option<String> {
        self.span(group).map(|(a, b)| text[a..b].iter().collect())
    }
}

pub struct Regex {
    root: Node,
    groups: usize,
    flags: Flags,
}

type Caps = Vec<Option<(usize, usize)>>;

fn is_word(c: char) -> bool {
    c.is_alphanumeric() || c == '_'
}

fn class_has(class: Class, c: char) -> bool {
    match class {
        Class::Word => is_word(c),
        Class::Space => c.is_whitespace(),
        Class::Digit => c.is_ascii_digit(),
    }
}

struct Parser {
    pat: Vec<char>,
    pos: usize,
    groups: usize,
}

impl Parser {
    fn peek(&self) -> Option<char> {
        self.pat.get(self.pos).copied()
    }
    fn eat(&mut self, c: char) -> bool {
        let hit = self.peek() == Some(c);
        self.pos += usize::from(hit);
        hit
    }
    fn number(&mut self) -> usize {
        let mut n = 0;
        while let Some(d) = self.peek().and_then(|c| c.to_digit(10)) {
            n = n * 10 + d as usize;
            self.pos += 1;
        }
        n
    }

    fn alternation(&mut self) -> Node {
        let mut branches = vec![self.sequence()];
        while self.eat('|') {
            branches.push(self.sequence());
        }
        if branches.len() == 1 {
            branches.remove(0)
        } else {
            Node::Alt(branches)
        }
    }

    fn sequence(&mut self) -> Node {
        let mut items = Vec::new();
        while let Some(c) = self.peek() {
            if c == '|' || c == ')' {
                break;
            }
            let atom = self.atom();
            items.push(self.quantified(atom));
        }
        Node::Seq(items)
    }

    fn quantified(&mut self, atom: Node) -> Node {
        let (min, max) = match self.peek() {
            Some('*') => (0, usize::MAX),
            Some('+') => (1, usize::MAX),
            Some('?') => (0, 1),
            Some('{') => {
                self.pos += 1;
                let min = self.number();
                let max = if self.eat(',') {
                    self.peek()
                        .filter(char::is_ascii_digit)
                        .map_or(usize::MAX, |_| self.number())
                } else {
                    min
                };
                (min, max)
            }
            _ => return atom,
        };
        self.pos += 1;
        let greedy = !self.eat('?');
        Node::Repeat {
            node: Box::new(atom),
            min,
            max,
            greedy,
        }
    }

    fn atom(&mut self) -> Node {
        let c = self.peek().unwrap_or(')');
        self.pos += 1;
        match c {
            '.' => Node::Any,
            '^' => Node::Start,
            '$' => Node::End,
            '(' => self.group(),
            '[' => self.set(),
            '\\' => self.escape(),
            other => Node::Char(other),
        }
    }

    fn group(&mut self) -> Node {
        if self.eat('?') {
            let kind = self.peek();
            self.pos += 1;
            if kind == Some('<') {
                self.pos += 1; // the `!` of `(?<!`
                let inner = self.atom();
                self.pos += 1;
                return Node::NotBehind(Box::new(inner));
            }
            let inner = self.alternation();
            self.pos += 1;
            return Node::Group(Box::new(inner), None);
        }
        self.groups += 1;
        let index = self.groups;
        let inner = self.alternation();
        self.pos += 1;
        Node::Group(Box::new(inner), Some(index))
    }

    fn escape(&mut self) -> Node {
        let c = self.peek().unwrap_or('\\');
        self.pos += 1;
        match c {
            'b' => Node::WordBoundary,
            's' | 'S' | 'w' | 'W' | 'd' | 'D' => Node::Set(Set {
                negate: false,
                chars: vec![],
                ranges: vec![],
                classes: vec![Self::class_of(c)],
            }),
            other => Node::Char(other),
        }
    }

    fn class_of(c: char) -> (Class, bool) {
        let class = match c.to_ascii_lowercase() {
            's' => Class::Space,
            'w' => Class::Word,
            _ => Class::Digit,
        };
        (class, c.is_ascii_uppercase())
    }

    fn set(&mut self) -> Node {
        let mut set = Set {
            negate: self.eat('^'),
            chars: vec![],
            ranges: vec![],
            classes: vec![],
        };
        while let Some(c) = self.peek() {
            self.pos += 1;
            if c == ']' {
                break;
            }
            let c = if c == '\\' {
                let e = self.peek().unwrap_or('\\');
                self.pos += 1;
                if "sSwWdD".contains(e) {
                    set.classes.push(Self::class_of(e));
                    continue;
                }
                match e {
                    'n' => '\n',
                    other => other,
                }
            } else {
                c
            };
            let is_range =
                self.peek() == Some('-') && self.pat.get(self.pos + 1).is_some_and(|n| *n != ']');
            if is_range {
                let hi = self.pat[self.pos + 1];
                self.pos += 2;
                set.ranges.push((c, hi));
            } else {
                set.chars.push(c);
            }
        }
        Node::Set(set)
    }
}

impl Regex {
    /// Compiles a pattern written in the supported subset. A pattern outside
    /// the subset is a programming error in this crate, not an input error.
    pub fn new(pattern: &str, flags: Flags) -> Regex {
        let mut parser = Parser {
            pat: pattern.chars().collect(),
            pos: 0,
            groups: 0,
        };
        let root = parser.alternation();
        Regex {
            root,
            groups: parser.groups,
            flags,
        }
    }

    fn eq(&self, a: char, b: char) -> bool {
        a == b || (self.flags.ignore_case && a.to_lowercase().eq(b.to_lowercase()))
    }

    fn set_has(&self, set: &Set, c: char) -> bool {
        let variants = [c, c.to_ascii_lowercase(), c.to_ascii_uppercase()];
        let tried = if self.flags.ignore_case {
            &variants[..]
        } else {
            &variants[..1]
        };
        let hit = tried.iter().any(|v| {
            set.chars.contains(v)
                || set.ranges.iter().any(|(lo, hi)| lo <= v && v <= hi)
                || set
                    .classes
                    .iter()
                    .any(|(class, neg)| class_has(*class, *v) != *neg)
        });
        hit != set.negate
    }

    /// Whether a one-character node accepts `c`.
    fn one(&self, node: &Node, c: char) -> Option<bool> {
        match node {
            Node::Char(x) => Some(self.eq(*x, c)),
            Node::Any => Some(self.flags.dot_all || c != '\n'),
            Node::Set(set) => Some(self.set_has(set, c)),
            _ => None,
        }
    }

    fn step(
        &self,
        node: &Node,
        text: &[char],
        pos: usize,
        caps: &mut Caps,
        k: &mut dyn FnMut(usize, &mut Caps) -> bool,
    ) -> bool {
        if let Some(width_one) = text.get(pos).and_then(|c| self.one(node, *c)) {
            return width_one && k(pos + 1, caps);
        }
        match node {
            Node::Char(_) | Node::Any | Node::Set(_) => false,
            Node::Start => {
                (pos == 0 || (self.flags.multiline && text[pos - 1] == '\n')) && k(pos, caps)
            }
            Node::End => {
                let at_end = pos == text.len() || (pos + 1 == text.len() && text[pos] == '\n');
                (at_end || (self.flags.multiline && text[pos] == '\n')) && k(pos, caps)
            }
            Node::WordBoundary => {
                let before = pos > 0 && is_word(text[pos - 1]);
                let after = pos < text.len() && is_word(text[pos]);
                before != after && k(pos, caps)
            }
            Node::NotBehind(inner) => {
                let behind = pos > 0 && self.one(inner, text[pos - 1]) == Some(true);
                !behind && k(pos, caps)
            }
            Node::Group(inner, index) => {
                let index = *index;
                self.step(inner, text, pos, caps, &mut |end, caps: &mut Caps| {
                    let Some(i) = index else { return k(end, caps) };
                    let old = caps[i];
                    caps[i] = Some((pos, end));
                    if k(end, caps) {
                        return true;
                    }
                    caps[i] = old;
                    false
                })
            }
            Node::Alt(branches) => branches.iter().any(|b| self.step(b, text, pos, caps, k)),
            Node::Seq(items) => self.sequence(items, text, pos, caps, k),
            Node::Repeat {
                node,
                min,
                max,
                greedy,
            } => self.repeat(node, *min, *max, *greedy, text, pos, caps, k),
        }
    }

    fn sequence(
        &self,
        items: &[Node],
        text: &[char],
        pos: usize,
        caps: &mut Caps,
        k: &mut dyn FnMut(usize, &mut Caps) -> bool,
    ) -> bool {
        match items.split_first() {
            None => k(pos, caps),
            Some((first, rest)) => self.step(first, text, pos, caps, &mut |p, c: &mut Caps| {
                self.sequence(rest, text, p, c, k)
            }),
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat(
        &self,
        node: &Node,
        min: usize,
        max: usize,
        greedy: bool,
        text: &[char],
        pos: usize,
        caps: &mut Caps,
        k: &mut dyn FnMut(usize, &mut Caps) -> bool,
    ) -> bool {
        if matches!(node, Node::Char(_) | Node::Any | Node::Set(_)) {
            let mut n = 0;
            while n < max
                && text
                    .get(pos + n)
                    .is_some_and(|c| self.one(node, *c) == Some(true))
            {
                n += 1;
            }
            if n < min {
                return false;
            }
            return if greedy {
                (min..=n).rev().any(|c| k(pos + c, caps))
            } else {
                (min..=n).any(|c| k(pos + c, caps))
            };
        }
        self.repeat_nodes(node, 0, min, max, greedy, text, pos, caps, k)
    }

    #[allow(clippy::too_many_arguments)]
    fn repeat_nodes(
        &self,
        node: &Node,
        count: usize,
        min: usize,
        max: usize,
        greedy: bool,
        text: &[char],
        pos: usize,
        caps: &mut Caps,
        k: &mut dyn FnMut(usize, &mut Caps) -> bool,
    ) -> bool {
        let more = |caps: &mut Caps, k: &mut dyn FnMut(usize, &mut Caps) -> bool| {
            count < max
                && self.step(node, text, pos, caps, &mut |p, c: &mut Caps| {
                    p != pos && self.repeat_nodes(node, count + 1, min, max, greedy, text, p, c, k)
                })
        };
        if count < min {
            return more(caps, k);
        }
        // The two orders differ: both sides run the continuation.
        #[allow(clippy::if_same_then_else)]
        if greedy {
            more(caps, k) || k(pos, caps)
        } else {
            k(pos, caps) || more(caps, k)
        }
    }

    /// The leftmost match at or after `from` (Python's `pattern.search`).
    pub fn search(&self, text: &[char], from: usize) -> Option<Match> {
        (from..=text.len()).find_map(|start| self.match_at(text, start))
    }

    /// A match starting exactly at `start` (Python's `pattern.match`).
    pub fn match_at(&self, text: &[char], start: usize) -> Option<Match> {
        let mut caps: Caps = vec![None; self.groups + 1];
        let mut end = 0;
        let hit = self.step(&self.root, text, start, &mut caps, &mut |e, _| {
            end = e;
            true
        });
        hit.then_some(Match {
            start,
            end,
            spans: caps,
        })
    }

    /// Every non-overlapping match, left to right (Python's `finditer`).
    pub fn find_all(&self, text: &[char]) -> Vec<Match> {
        let mut found = Vec::new();
        let mut from = 0;
        while let Some(m) = self.search(text, from) {
            from = m.end.max(m.start + 1);
            found.push(m);
        }
        found
    }
}
