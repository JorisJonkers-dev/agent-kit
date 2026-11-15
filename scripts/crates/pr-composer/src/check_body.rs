//! `pr-composer check-body`: checks a drafted pull request body against the
//! repository's template before anyone sees it. The failures it prevents are
//! an invented section, dropped checklist items, a reordered template and
//! flattened heading levels: all four look fine in a paraphrased preview.
//!
//! Exit 0 clean, 1 violations (printed), 2 bad invocation.

use crate::args::{Opt, Outcome, Spec};
use crate::regex::{Regex, NONE, S};
use crate::text::{os_error, splitlines};
use crate::Io;
use std::collections::HashSet;
use std::fs;

const SPEC: Spec = Spec {
    prog: "pr-composer check-body",
    synopsis: "-t TEMPLATE -b BODY [--skipped SKIPPED]",
    positional: None,
    opts: &[
        Opt {
            short: Some("-t"),
            long: "--template",
            flag: false,
            required: true,
            help: "the repository's PR template",
        },
        Opt {
            short: Some("-b"),
            long: "--body",
            flag: false,
            required: true,
            help: "the drafted body",
        },
        Opt {
            short: None,
            long: "--skipped",
            flag: false,
            required: false,
            help: "comma-separated headings the user chose to omit (not reported as missing)",
        },
    ],
};

#[derive(Clone)]
struct Heading {
    level: usize,
    text: String,
    line: usize,
}

/// An insertion-ordered map where a repeated key keeps its first position and
/// takes the last value, as a Python dict does.
struct ByNorm(Vec<(String, Heading)>);

impl ByNorm {
    fn new(headings: Vec<Heading>) -> ByNorm {
        let mut map: Vec<(String, Heading)> = Vec::new();
        for h in headings {
            let key = norm(&h.text);
            match map.iter_mut().find(|(k, _)| *k == key) {
                Some(slot) => slot.1 = h,
                None => map.push((key, h)),
            }
        }
        ByNorm(map)
    }
    fn get(&self, key: &str) -> Option<&Heading> {
        self.0.iter().find(|(k, _)| k == key).map(|(_, h)| h)
    }
}

struct Patterns {
    heading: Regex,
    checkbox: Regex,
    fence: Regex,
    placeholder: Regex,
    comment: Regex,
}

impl Patterns {
    fn new() -> Patterns {
        Patterns {
            heading: Regex::new(r"^(\s{0,3})(#{1,6})\s+(.+?)\s*#*\s*$", NONE),
            checkbox: Regex::new(r"^\s*[-*]\s*\[( |x|X)\]\s*(.+?)\s*$", NONE),
            fence: Regex::new(r"^\s*(```|~~~)", NONE),
            placeholder: Regex::new("«([^»]*)»", NONE),
            comment: Regex::new("<!--(.*?)-->", S),
        }
    }
}

fn parse(p: &Patterns, text: &str) -> (Vec<Heading>, Vec<String>) {
    let mut headings = Vec::new();
    let mut boxes = Vec::new();
    let mut in_fence = false;
    for (i, line) in splitlines(text).iter().enumerate() {
        let chars: Vec<char> = line.chars().collect();
        if p.fence.match_at(&chars, 0).is_some() {
            in_fence = !in_fence;
            continue;
        }
        if in_fence {
            continue;
        }
        if let Some(m) = p.heading.match_at(&chars, 0) {
            let level = m.span(2).map_or(0, |(a, b)| b - a);
            let text = m.group(&chars, 3).unwrap_or_default().trim().to_string();
            headings.push(Heading {
                level,
                text,
                line: i + 1,
            });
            continue;
        }
        if let Some(m) = p.checkbox.match_at(&chars, 0) {
            boxes.push(m.group(&chars, 2).unwrap_or_default().trim().to_string());
        }
    }
    (headings, boxes)
}

/// Compares heading text loosely: case, punctuation, emoji padding, bold markers.
pub fn norm(s: &str) -> String {
    let stripped: String = s
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`'))
        .collect();
    let spaced: String = stripped
        .chars()
        .map(|c| {
            if c.is_alphanumeric() || c == '_' || c.is_whitespace() {
                c
            } else {
                ' '
            }
        })
        .collect();
    spaced
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

fn order(headings: &[Heading], present_in: &ByNorm) -> Vec<String> {
    headings
        .iter()
        .map(|h| norm(&h.text))
        .filter(|n| present_in.get(n).is_some())
        .collect()
}

fn read(path: &str) -> Result<String, String> {
    // Python reads text with universal newlines.
    fs::read_to_string(path)
        .map(|t| t.replace("\r\n", "\n").replace('\r', "\n"))
        .map_err(|e| os_error(&e, path))
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    let parsed = match SPEC.parse(args) {
        Outcome::Run(p) => p,
        Outcome::Help(text) => {
            io.out.push_str(&text);
            return 0;
        }
        Outcome::Error(text) => {
            io.err.push_str(&text);
            return 2;
        }
    };
    let skipped_arg = parsed
        .values
        .get("--skipped")
        .map(String::as_str)
        .unwrap_or("");
    let (template, body) = match (
        read(&parsed.values["--template"]),
        read(&parsed.values["--body"]),
    ) {
        (Ok(t), Ok(b)) => (t, b),
        (Err(e), _) | (_, Err(e)) => {
            io.err.push_str(&format!("cannot read input: {e}\n"));
            return 2;
        }
    };
    let p = Patterns::new();
    let (t_head, t_boxes) = parse(&p, &template);
    let (b_head, b_boxes) = parse(&p, &body);
    let mut skipped: Vec<String> = skipped_arg
        .split(',')
        .filter(|s| !s.trim().is_empty())
        .map(norm)
        .collect::<HashSet<_>>()
        .into_iter()
        .collect();
    skipped.sort();

    let t_by = ByNorm::new(t_head.clone());
    let b_by = ByNorm::new(b_head.clone());
    let mut problems: Vec<String> = Vec::new();
    let mut warnings: Vec<String> = Vec::new();

    // 1. Invented sections: the worst failure, because they look authoritative.
    for h in &b_head {
        if t_by.get(&norm(&h.text)).is_none() {
            problems.push(format!(
                "INVENTED SECTION: '{}' (body line {}) is not in the template. Remove it, or ask the user before adding a section the repo doesn't use.",
                h.text, h.line
            ));
        }
    }
    // 2. Dropped sections.
    for h in &t_head {
        let n = norm(&h.text);
        if b_by.get(&n).is_none() && !skipped.contains(&n) {
            problems.push(format!(
                "MISSING SECTION: template has '{}' (template line {}) and the body doesn't. Include it, or pass it in --skipped if the user skipped it.",
                h.text, h.line
            ));
        }
    }
    // 3. Heading level changes: '##' silently becoming '#' reflows the whole PR.
    for (n, bh) in &b_by.0 {
        if let Some(th) = t_by.get(n).filter(|th| th.level != bh.level) {
            problems.push(format!(
                "HEADING LEVEL CHANGED: '{}' is {} in the body but {} in the template. Match the template exactly.",
                bh.text,
                "#".repeat(bh.level),
                "#".repeat(th.level)
            ));
        }
    }
    // 4. Order. Compare only sections present in both, so a skip doesn't read as a reorder.
    let t_order = order(&t_head, &b_by);
    let b_order = order(&b_head, &t_by);
    if t_order != b_order {
        let names = |keys: &[String], by: &ByNorm| {
            keys.iter()
                .filter_map(|n| by.get(n))
                .map(|h| h.text.clone())
                .collect::<Vec<_>>()
                .join(" → ")
        };
        problems.push(format!(
            "SECTION ORDER CHANGED. Template order: {}  |  body order: {}. Restore the template's order — link blocks in particular belong where the template puts them, not wherever they read best.",
            names(&t_order, &t_by),
            names(&b_order, &b_by)
        ));
    }
    // 5. Checklist completeness. Templates use checklists as review gates;
    //    dropping an item removes a gate, and a summarized preview hides that.
    let b_norm_boxes: HashSet<String> = b_boxes.iter().map(|x| norm(x)).collect();
    for item in &t_boxes {
        if !b_norm_boxes.contains(&norm(item)) {
            problems.push(format!(
                "DROPPED CHECKLIST ITEM: '{item}'. Reproduce every item verbatim."
            ));
        }
    }
    if !t_boxes.is_empty() && b_boxes.len() < t_boxes.len() {
        problems.push(format!(
            "CHECKLIST TRUNCATED: template has {} items, body has {}.",
            t_boxes.len(),
            b_boxes.len()
        ));
    }
    // 6. Placeholders: reported, never auto-removed.
    let body_chars: Vec<char> = body.chars().collect();
    let found: Vec<String> = p
        .placeholder
        .find_all(&body_chars)
        .iter()
        .map(|m| m.group(&body_chars, 1).unwrap_or_default())
        .collect();
    if !found.is_empty() {
        let shown: Vec<String> = found.iter().take(6).map(|x| format!("«{x}»")).collect();
        warnings.push(format!(
            "{} unfilled placeholder(s): {}{}",
            found.len(),
            shown.join("; "),
            if found.len() > 6 { " …" } else { "" }
        ));
    }
    // 7. Leftover template instructions.
    let leftover = p.comment.find_all(&body_chars).len();
    if leftover > 0 {
        warnings.push(format!(
            "{leftover} HTML comment(s) from the template still present — keep them only if the repo's merged PRs also keep them (check comments_retained_rate in the style profile)."
        ));
    }

    io.out.push_str(&format!(
        "template sections: {} · body sections: {} · template checkboxes: {} · body checkboxes: {}\n",
        t_head.len(),
        b_head.len(),
        t_boxes.len(),
        b_boxes.len()
    ));
    if !skipped.is_empty() {
        io.out
            .push_str(&format!("user-skipped: {}\n", skipped.join(", ")));
    }
    for w in &warnings {
        io.out.push_str(&format!("  warn  {w}\n"));
    }
    for pr in &problems {
        io.out.push_str(&format!("  FAIL  {pr}\n"));
    }
    if problems.is_empty() {
        io.out.push_str(&format!(
            "\nbody matches template{}\n",
            if warnings.is_empty() {
                ""
            } else {
                " (with warnings)"
            }
        ));
        return 0;
    }
    io.out.push_str(&format!(
        "\n{} violation(s). Fix the body and re-run before rendering the preview.\n",
        problems.len()
    ));
    1
}
