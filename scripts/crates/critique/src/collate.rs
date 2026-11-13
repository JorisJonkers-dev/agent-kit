//! `String.prototype.localeCompare` in the `en` locale, which the node runner sorts by: the ICU root collation, for
//! the ASCII text the ledger and routing hold (client names, timestamps, run ids, file names).
//!
//! Strings compare in two passes, as ICU does. The first, the primary one, goes over the characters in the order the
//! collation ranks them: white space, punctuation and symbols, digits, then letters with no regard to case. Control
//! characters other than tab, line feed, vertical tab, form feed and carriage return are ignored. Only when the
//! primary pass finds the strings equal does the second, the tertiary one, decide, and a lower-case letter sorts
//! before its upper-case twin. With `numeric` a run of digits is one element worth its number.
//!
//! Beyond ASCII the port differs from ICU: it sorts every such character after all of ASCII, by code point, where
//! ICU sorts an accented letter with its base letter (`'z'.localeCompare('é')` is 1 in node). The ledger's run ids,
//! client names, timestamps and file names are ASCII; docs/RUST.md records the difference.

use std::cmp::Ordering;

/// What sorts before the digits, in the collation's order.
const BEFORE_DIGITS: &str = "\t\n\u{b}\u{c}\r _-,;:!?.'\"()[]{}@*/\\&#%`^+<=>|~$";

/// The primary weight of the digit `0`; the other digits follow it, then the letters.
const DIGITS: u32 = 100;

/// The primary weight of the letter `a`.
const LETTERS: u32 = 200;

/// The primary weight of a character beyond ASCII is this plus its code point.
const BEYOND_ASCII: u32 = 1000;

/// One thing the strings are compared by.
#[derive(Clone, Debug, PartialEq)]
enum Element<'a> {
    /// A character: its primary weight, and whether it is an upper-case letter.
    Char(u32, bool),
    /// A run of digits without its leading zeros, which `numeric` compares as one number.
    Number(&'a str),
}

impl Element<'_> {
    fn primary(&self) -> u32 {
        match self {
            Element::Char(weight, _) => *weight,
            Element::Number(_) => DIGITS,
        }
    }

    fn upper(&self) -> bool {
        matches!(self, Element::Char(_, true))
    }

    fn primary_cmp(&self, other: &Element<'_>) -> Ordering {
        match (self, other) {
            (Element::Number(a), Element::Number(b)) => a.len().cmp(&b.len()).then_with(|| a.cmp(b)),
            _ => self.primary().cmp(&other.primary()),
        }
    }
}

fn weight(c: char) -> Option<Element<'static>> {
    match c {
        'a'..='z' => Some(Element::Char(LETTERS + (c as u32 - 'a' as u32), false)),
        'A'..='Z' => Some(Element::Char(LETTERS + (c as u32 - 'A' as u32), true)),
        '0'..='9' => Some(Element::Char(DIGITS + (c as u32 - '0' as u32), false)),
        c if c.is_ascii() => BEFORE_DIGITS.find(c).map(|at| Element::Char(at as u32, false)),
        c => Some(Element::Char(BEYOND_ASCII + c as u32, false)),
    }
}

fn elements(text: &str, numeric: bool) -> Vec<Element<'_>> {
    let mut found = Vec::new();
    let mut rest = text.char_indices().peekable();
    while let Some((at, c)) = rest.next() {
        if numeric && c.is_ascii_digit() {
            let mut end = at + 1;
            while let Some((next, _)) = rest.next_if(|(_, d)| d.is_ascii_digit()) {
                end = next + 1;
            }
            found.push(Element::Number(text[at..end].trim_start_matches('0')));
        } else if let Some(element) = weight(c) {
            found.push(element);
        }
    }
    found
}

/// `a.localeCompare(b, 'en', { numeric })`.
pub fn locale_compare(a: &str, b: &str, numeric: bool) -> Ordering {
    let (a, b) = (elements(a, numeric), elements(b, numeric));
    let primary = a.iter().zip(&b).map(|(x, y)| x.primary_cmp(y)).find(|order| order.is_ne());
    primary.unwrap_or_else(|| a.len().cmp(&b.len())).then_with(|| a.iter().map(Element::upper).cmp(b.iter().map(Element::upper)))
}
