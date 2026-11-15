//! `pr-composer mine-prs`: turns a `gh pr list --state merged --json ...` dump
//! into a house-style profile. The point of doing this in code rather than by
//! reading 100 PR bodies: the numbers are reproducible, cheap, and they
//! distinguish "this repo always does X" from "I saw X twice and assumed".

use crate::args::{Opt, Outcome, Spec};
use crate::json::J;
use crate::regex::{Regex, I, M, NONE, S};
use crate::text::{os_error, round};
use crate::Io;
use serde_json::Value;
use std::collections::HashSet;
use std::fs;

const SPEC: Spec = Spec {
    prog: "pr-composer mine-prs",
    synopsis: "[--out OUT] [--md] dump",
    positional: Some("dump"),
    opts: &[
        Opt {
            short: None,
            long: "--out",
            flag: false,
            required: false,
            help: "write profile JSON here",
        },
        Opt {
            short: None,
            long: "--md",
            flag: true,
            required: false,
            help: "also write a markdown summary next to --out",
        },
    ],
};

struct Pats {
    conv: Regex,
    ticket: Regex,
    heading: Regex,
    bold: Regex,
    checkbox: Regex,
    comment: Regex,
    link: Regex,
    bare_issue: Regex,
    image: Regex,
    slack: Regex,
    section: Regex,
    family: Regex,
    prefix: Regex,
}

impl Pats {
    fn new() -> Pats {
        Pats {
            // Allowed to sit after a ticket-key prefix, e.g. "[MOVES-482] feat(api): …"
            conv: Regex::new(
                r"(?:^|\]\s*|\s)(feat|fix|chore|docs|refactor|test|perf|build|ci|style|revert)(\([^)]*\))?!?:\s",
                I,
            ),
            ticket: Regex::new(r"^\[?([A-Z][A-Z0-9]{1,9})[- ](\d+)\]?[:\s]", NONE),
            heading: Regex::new(r"^\s{0,3}(#{1,4})\s+(.+?)\s*#*\s*$", M),
            bold: Regex::new(r"^\s{0,3}\*\*([^*\n]{2,60}?):?\*\*\s*$", M),
            checkbox: Regex::new(r"^\s*[-*]\s*\[( |x|X)\]\s+", M),
            comment: Regex::new(r"<!--.*?-->", S),
            link: Regex::new(
                r"\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?|ref(?:s|erences)?|relate[sd]?\s+to|part\s+of)\b[:\s]*#\d+",
                I,
            ),
            bare_issue: Regex::new(r"(?<![\w/])#\d+", NONE),
            image: Regex::new(
                r"(!\[[^\]]*\]\(|<img\s|user-images\.githubusercontent|githubusercontent\.com/assets)",
                I,
            ),
            slack: Regex::new(r"https?://[a-z0-9-]+\.slack\.com/\S+", I),
            section: Regex::new(r"^\s{0,3}#{1,4}\s+(.+)$", M),
            family: Regex::new(r"^([\w .-]+?)\s*[:/]\s*(.+)$", NONE),
            prefix: Regex::new(r"^(\[[^\]]+\]\s*|\w+(\([^)]*\))?!?:\s*)", NONE),
        }
    }
}

fn chars(s: &str) -> Vec<char> {
    s.chars().collect()
}

fn has(re: &Regex, s: &str) -> bool {
    re.search(&chars(s), 0).is_some()
}

fn pct(n: usize, total: usize) -> f64 {
    round(100.0 * n as f64 / total as f64, 1)
}

/// An insertion-ordered counter; `most_common` is a stable sort by count.
#[derive(Default)]
struct Counter(Vec<(String, i64)>);

impl Counter {
    fn add(&mut self, key: &str) {
        match self.0.iter_mut().find(|(k, _)| k == key) {
            Some(slot) => slot.1 += 1,
            None => self.0.push((key.to_string(), 1)),
        }
    }
    fn get(&self, key: &str) -> i64 {
        self.0.iter().find(|(k, _)| k == key).map_or(0, |(_, c)| *c)
    }
    fn total(&self) -> i64 {
        self.0.iter().map(|(_, c)| c).sum()
    }
    fn most_common(&self, k: usize) -> Vec<(String, i64)> {
        let mut items = self.0.clone();
        items.sort_by_key(|item| std::cmp::Reverse(item.1));
        items.truncate(k);
        items
    }
    fn top(&self, k: usize) -> J {
        J::Arr(
            self.most_common(k)
                .into_iter()
                .map(|(v, c)| J::obj(vec![("value", J::Str(v)), ("count", J::Int(c))]))
                .collect(),
        )
    }
}

fn norm_heading(text: &str) -> String {
    let dropped: String = text
        .chars()
        .filter(|c| !matches!(c, '*' | '_' | '`' | ':'))
        .collect();
    let trimmed = dropped
        .trim()
        .trim_matches(|c: char| !(c.is_alphanumeric() || c == '_'));
    trimmed
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ")
        .to_lowercase()
}

static NULL: Value = Value::Null;

fn field<'a>(v: &'a Value, key: &str) -> &'a Value {
    v.as_object().and_then(|o| o.get(key)).unwrap_or(&NULL)
}

fn truthy(v: &Value) -> bool {
    match v {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

fn text_of(v: &Value) -> &str {
    v.as_str().unwrap_or("")
}

fn int_of(v: &Value) -> i64 {
    v.as_i64()
        .or_else(|| v.as_f64().map(|f| f as i64))
        .unwrap_or(0)
}

fn items(v: &Value) -> &[Value] {
    v.as_array().map_or(&[], Vec::as_slice)
}

fn login(v: &Value) -> Option<String> {
    ["login", "name", "slug"]
        .iter()
        .map(|k| text_of(field(v, k)))
        .find(|s| !s.is_empty())
        .map(str::to_string)
}

fn median(values: &[i64]) -> i64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    let mid = sorted.len() / 2;
    if sorted.len() % 2 == 1 {
        sorted[mid]
    } else {
        ((sorted[mid - 1] + sorted[mid]) as f64 / 2.0) as i64
    }
}

/// Python's `sorted(v)[int(q * (len(v) - 1))]`.
fn quantile(values: &[i64], q: f64) -> i64 {
    let mut sorted = values.to_vec();
    sorted.sort_unstable();
    sorted[(q * (sorted.len() - 1) as f64) as usize]
}

fn push_unique(list: &mut Vec<String>, item: String) {
    if !list.contains(&item) {
        list.push(item);
    }
}

fn title_stats(p: &Pats, titles: &[String], n: usize) -> J {
    let tchars: Vec<Vec<char>> = titles.iter().map(|t| chars(t)).collect();
    let conv_match = |t: &Vec<char>| p.conv.search(t, 0);
    let ticket_match = |t: &Vec<char>| p.ticket.match_at(t, 0);
    let conv = tchars.iter().filter(|t| conv_match(t).is_some()).count();
    let mut keys = Counter::default();
    for t in &tchars {
        if let Some(key) = ticket_match(t).and_then(|m| m.group(t, 1)) {
            keys.add(&key);
        }
    }
    let both = tchars
        .iter()
        .filter(|t| ticket_match(t).is_some() && conv_match(t).is_some())
        .count();
    let lengths: Vec<i64> = tchars.iter().map(|t| t.len() as i64).collect();
    let ends_period = titles
        .iter()
        .filter(|t| t.trim_end().ends_with('.'))
        .count();
    let lower_start = tchars
        .iter()
        .filter(|t| {
            let start = p.prefix.match_at(t, 0).map_or(0, |m| m.end);
            t.get(start).is_some_and(|c| c.is_lowercase())
        })
        .count();
    let mut scopes = Counter::default();
    for t in &tchars {
        if let Some(scope) = conv_match(t).and_then(|m| m.group(t, 2)) {
            scopes.add(scope.trim_matches(['(', ')']));
        }
    }
    J::obj(vec![
        ("conventional_commit_rate", J::Float(pct(conv, n))),
        ("ticket_key_rate", J::Float(pct(keys.total() as usize, n))),
        ("ticket_keys_seen", keys.top(5)),
        ("ticket_plus_conventional_rate", J::Float(pct(both, n))),
        ("median_length", J::Int(median(&lengths))),
        ("p90_length", J::Int(quantile(&lengths, 0.9))),
        ("trailing_period_rate", J::Float(pct(ends_period, n))),
        ("lowercase_after_prefix_rate", J::Float(pct(lower_start, n))),
        ("common_scopes", scopes.top(8)),
        (
            "examples",
            J::Arr(titles.iter().take(6).map(|t| J::s(t)).collect()),
        ),
    ])
}

fn body_stats(p: &Pats, bodies: &[String], n: usize) -> J {
    let empty = bodies.iter().filter(|b| b.trim().is_empty()).count();
    let stripped: Vec<String> = bodies
        .iter()
        .map(|b| {
            let c = chars(b);
            let mut out = String::new();
            let mut at = 0;
            for m in p.comment.find_all(&c) {
                out.extend(&c[at..m.start]);
                at = m.end;
            }
            out.extend(&c[at..]);
            out
        })
        .collect();
    let word_counts: Vec<i64> = stripped
        .iter()
        .map(|s| s.split_whitespace().count() as i64)
        .collect();
    let nonempty: Vec<i64> = Some(
        word_counts
            .iter()
            .copied()
            .filter(|w| *w > 0)
            .collect::<Vec<_>>(),
    )
    .filter(|v| !v.is_empty())
    .unwrap_or(vec![0]);

    let mut headings = Counter::default();
    let mut orders: Vec<Vec<String>> = Vec::new();
    for s in &stripped {
        let c = chars(s);
        let mut found: Vec<String> = p
            .heading
            .find_all(&c)
            .iter()
            .filter_map(|m| m.group(&c, 2))
            .map(|t| norm_heading(&t))
            .collect();
        found.extend(
            p.bold
                .find_all(&c)
                .iter()
                .filter_map(|m| m.group(&c, 1))
                .map(|t| norm_heading(&t)),
        );
        let mut ordered: Vec<String> = Vec::new();
        for h in found.into_iter().filter(|h| !h.is_empty()) {
            push_unique(&mut ordered, h);
        }
        for h in &ordered {
            headings.add(h);
        }
        if !ordered.is_empty() {
            orders.push(ordered);
        }
    }

    // Sections that exist but are routinely left hollow (<8 words of content).
    let mut hollow = Counter::default();
    let mut present = Counter::default();
    for s in &stripped {
        let c = chars(s);
        let found = p.section.find_all(&c);
        for (i, m) in found.iter().enumerate() {
            let h = norm_heading(&m.group(&c, 1).unwrap_or_default());
            let until = found.get(i + 1).map_or(c.len(), |next| next.start);
            if h.is_empty() {
                continue;
            }
            present.add(&h);
            if c[m.end..until]
                .iter()
                .collect::<String>()
                .split_whitespace()
                .count()
                < 8
            {
                hollow.add(&h);
            }
        }
    }
    let seen_floor = 3.max(n / 20) as i64;
    let mut hollow_rows: Vec<(f64, &String, i64)> = present
        .0
        .iter()
        .filter(|(_, seen)| *seen >= seen_floor)
        .map(|(h, seen)| (pct(hollow.get(h) as usize, *seen as usize), h, *seen))
        .collect();
    hollow_rows.sort_by(|a, b| b.0.total_cmp(&a.0));
    let hollow_rates: Vec<J> = hollow_rows
        .into_iter()
        .take(10)
        .map(|(rate, h, seen)| {
            J::obj(vec![
                ("section", J::s(h)),
                ("hollow_rate", J::Float(rate)),
                ("seen", J::Int(seen)),
            ])
        })
        .collect();

    let mut link_kw = Counter::default();
    for s in &stripped {
        let c = chars(s);
        for m in p.link.find_all(&c) {
            link_kw.add(
                &m.group(&c, 1)
                    .unwrap_or_default()
                    .to_lowercase()
                    .replace("  ", " "),
            );
        }
    }
    let preferred = link_kw
        .most_common(1)
        .into_iter()
        .next()
        .map_or(J::Null, |(k, _)| J::Str(k));
    let rate_of = |f: &dyn Fn(&String) -> bool, list: &[String]| {
        J::Float(pct(list.iter().filter(|s| f(s)).count(), n))
    };
    let modal = orders.iter().fold(None::<(&Vec<String>, i64)>, |best, o| {
        let score: i64 = o.iter().map(|h| headings.get(h)).sum();
        best.filter(|(_, top)| *top >= score).or(Some((o, score)))
    });

    J::obj(vec![
        ("empty_body_rate", J::Float(pct(empty, n))),
        ("median_words", J::Int(median(&nonempty))),
        ("p10_words", J::Int(quantile(&nonempty, 0.1))),
        ("p90_words", J::Int(quantile(&nonempty, 0.9))),
        (
            "comments_retained_rate",
            rate_of(&|b| b.contains("<!--"), bodies),
        ),
        (
            "checklist_rate",
            rate_of(&|s| has(&p.checkbox, s), &stripped),
        ),
        (
            "checked_box_rate",
            rate_of(&|s| s.contains("[x]") || s.contains("[X]"), &stripped),
        ),
        ("image_rate", rate_of(&|s| has(&p.image, s), &stripped)),
        ("slack_link_rate", rate_of(&|s| has(&p.slack, s), &stripped)),
        (
            "issue_reference_rate",
            rate_of(&|s| has(&p.bare_issue, s), &stripped),
        ),
        ("link_keywords", link_kw.top(6)),
        ("preferred_link_keyword", preferred),
        (
            "section_frequency",
            J::Arr(
                headings
                    .most_common(14)
                    .into_iter()
                    .map(|(h, c)| {
                        J::obj(vec![
                            ("section", J::Str(h)),
                            ("rate", J::Float(pct(c as usize, n))),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "modal_section_order",
            J::Arr(modal.map_or(vec![], |(o, _)| o.iter().map(|h| J::s(h)).collect())),
        ),
        ("routinely_hollow_sections", J::Arr(hollow_rates)),
    ])
}

fn people_stats(prs: &[Value]) -> J {
    let (mut reviewers, mut approvers, mut requested, mut assignees) = (
        Counter::default(),
        Counter::default(),
        Counter::default(),
        Counter::default(),
    );
    let mut per_pr: Vec<i64> = Vec::new();
    let mut by_path: Vec<(String, Counter)> = Vec::new();
    for p in prs {
        let mut rs: Vec<String> = Vec::new();
        for r in items(field(p, "reviews")) {
            if let Some(u) = login(field(r, "author")) {
                if text_of(field(r, "state")).to_uppercase() == "APPROVED" {
                    approvers.add(&u);
                }
                push_unique(&mut rs, u);
            }
        }
        for rr in items(field(p, "reviewRequests")) {
            if let Some(u) = login(rr).or_else(|| login(field(rr, "requestedReviewer"))) {
                requested.add(&u);
                push_unique(&mut rs, u);
            }
        }
        for u in &rs {
            reviewers.add(u);
        }
        per_pr.push(rs.len() as i64);
        for a in items(field(p, "assignees")) {
            if let Some(u) = login(a) {
                assignees.add(&u);
            }
        }
        for f in items(field(p, "files")).iter().take(200) {
            let path = text_of(field(f, "path"));
            if path.is_empty() || rs.is_empty() {
                continue;
            }
            let bucket: Vec<&str> = path.split('/').take(2).collect();
            let bucket = bucket.join("/");
            let bucket = if bucket.is_empty() {
                path.to_string()
            } else {
                bucket
            };
            let slot = match by_path.iter().position(|(k, _)| *k == bucket) {
                Some(i) => i,
                None => {
                    by_path.push((bucket, Counter::default()));
                    by_path.len() - 1
                }
            };
            for u in &rs {
                by_path[slot].1.add(u);
            }
        }
    }
    by_path.sort_by_key(|(_, c)| -c.total());
    let mean = round(per_pr.iter().sum::<i64>() as f64 / per_pr.len() as f64, 2);
    J::obj(vec![
        ("top_reviewers", reviewers.top(10)),
        ("top_approvers", approvers.top(10)),
        ("top_requested", requested.top(10)),
        ("top_assignees", assignees.top(8)),
        ("median_reviewers_per_pr", J::Int(median(&per_pr))),
        ("mean_reviewers_per_pr", J::Float(mean)),
        (
            "reviewer_by_path",
            J::Obj(
                by_path
                    .into_iter()
                    .take(20)
                    .map(|(k, c)| (k, c.top(4)))
                    .collect(),
            ),
        ),
    ])
}

fn meta_stats(p: &Pats, prs: &[Value], n: usize) -> J {
    let mut labels = Counter::default();
    // A label family carried by nearly every merged PR is effectively
    // mandatory (version/*, release:*, type/*). Missing one is a finding.
    let mut families: Vec<(String, Counter, HashSet<String>)> = Vec::new();
    for pr in prs {
        for l in items(field(pr, "labels")) {
            let name = text_of(field(l, "name"));
            if name.is_empty() {
                continue;
            }
            labels.add(name);
            let c = chars(name);
            let Some(fam) = p
                .family
                .match_at(&c, 0)
                .and_then(|m| m.group(&c, 1))
                .map(|f| f.trim().to_lowercase())
                .filter(|f| !f.is_empty())
            else {
                continue;
            };
            let slot = match families.iter().position(|(k, _, _)| *k == fam) {
                Some(i) => i,
                None => {
                    families.push((fam, Counter::default(), HashSet::new()));
                    families.len() - 1
                }
            };
            families[slot].1.add(name);
            families[slot].2.insert(field(pr, "number").to_string());
        }
    }
    let coverage = |f: &(String, Counter, HashSet<String>)| pct(f.2.len(), n);
    let mut required: Vec<&(String, Counter, HashSet<String>)> =
        families.iter().filter(|f| coverage(f) >= 80.0).collect();
    required.sort_by(|a, b| coverage(b).total_cmp(&coverage(a)));
    let mut by_size: Vec<&(String, Counter, HashSet<String>)> = families.iter().collect();
    by_size.sort_by_key(|f| std::cmp::Reverse(f.2.len()));
    let sizes: Vec<i64> = prs
        .iter()
        .map(|p| int_of(field(p, "additions")) + int_of(field(p, "deletions")))
        .collect();
    let changed: Vec<i64> = prs
        .iter()
        .map(|p| int_of(field(p, "changedFiles")))
        .collect();
    let mut bases = Counter::default();
    for pr in prs {
        let base = text_of(field(pr, "baseRefName"));
        bases.add(if base.is_empty() { "?" } else { base });
    }
    J::obj(vec![
        ("label_frequency", labels.top(12)),
        (
            "labelled_pr_rate",
            J::Float(pct(
                prs.iter().filter(|p| truthy(field(p, "labels"))).count(),
                n,
            )),
        ),
        (
            "required_label_families",
            J::Arr(
                required
                    .iter()
                    .map(|f| {
                        J::obj(vec![
                            ("family", J::s(&f.0)),
                            ("coverage", J::Float(coverage(f))),
                            ("values", f.1.top(6)),
                        ])
                    })
                    .collect(),
            ),
        ),
        (
            "label_families",
            J::Arr(
                by_size
                    .iter()
                    .take(8)
                    .map(|f| {
                        J::obj(vec![
                            ("family", J::s(&f.0)),
                            ("coverage", J::Float(coverage(f))),
                        ])
                    })
                    .collect(),
            ),
        ),
        ("median_lines_changed", J::Int(median(&sizes))),
        ("median_files_changed", J::Int(median(&changed))),
        ("base_branches", bases.top(4)),
    ])
}

pub fn analyse(prs: &[Value]) -> J {
    let n = prs.len();
    let mut prof = vec![("sample_size", J::Int(n as i64))];
    if n == 0 {
        prof.push((
            "warning",
            J::s("no merged PRs available; no style profile can be derived"),
        ));
        return J::obj(prof);
    }
    let p = Pats::new();
    let titles: Vec<String> = prs
        .iter()
        .map(|x| text_of(field(x, "title")).to_string())
        .collect();
    let bodies: Vec<String> = prs
        .iter()
        .map(|x| text_of(field(x, "body")).to_string())
        .collect();
    prof.push(("title", title_stats(&p, &titles, n)));
    prof.push(("body", body_stats(&p, &bodies, n)));
    prof.push(("people", people_stats(prs)));
    prof.push(("meta", meta_stats(&p, prs, n)));
    if n < 10 {
        prof.push((
            "warning",
            J::Str(format!("only {n} merged PRs — treat every rate here as anecdote, lean on the template and conventional defaults instead")),
        ));
    }
    J::obj(prof)
}

fn joined(list: &J, f: impl Fn(&J) -> String) -> String {
    list.list().iter().map(f).collect::<Vec<_>>().join(", ")
}

fn value_count(prefix: &str, suffix: &str) -> impl Fn(&J) -> String {
    let (prefix, suffix) = (prefix.to_string(), suffix.to_string());
    move |r| {
        format!(
            "{prefix}{} ({}){suffix}",
            r.get("value").show(),
            r.get("count").show()
        )
    }
}

pub fn render_md(p: &J) -> String {
    let n = p.get("sample_size").number() as i64;
    let mut l: Vec<String> = vec![format!("# House style profile ({n} merged PRs)")];
    if p.get("warning") != &J::Null {
        l.push(String::new());
        l.push(format!("> **{}**", p.get("warning").show()));
    }
    if n == 0 {
        return l.join("\n") + "\n";
    }
    let (t, b, ppl, m) = (
        p.get("title"),
        p.get("body"),
        p.get("people"),
        p.get("meta"),
    );
    let s = |j: &J, k: &str| j.get(k).show();

    l.extend(["".into(), "## Title".into(), "".into()]);
    l.push(format!(
        "- conventional-commit prefix: **{}%**",
        s(t, "conventional_commit_rate")
    ));
    let keys = if t.get("ticket_keys_seen").is_empty_list() {
        String::new()
    } else {
        format!(
            " (keys: {})",
            joined(t.get("ticket_keys_seen"), |k| k.get("value").show())
        )
    };
    l.push(format!(
        "- ticket key prefix: **{}%**{keys}",
        s(t, "ticket_key_rate")
    ));
    l.push(format!(
        "- both together: {}%",
        s(t, "ticket_plus_conventional_rate")
    ));
    l.push(format!(
        "- length: median {}, p90 {} chars",
        s(t, "median_length"),
        s(t, "p90_length")
    ));
    l.push(format!(
        "- lowercase after prefix: {}% · trailing period: {}%",
        s(t, "lowercase_after_prefix_rate"),
        s(t, "trailing_period_rate")
    ));
    if !t.get("common_scopes").is_empty_list() {
        l.push(format!(
            "- scopes: {}",
            joined(t.get("common_scopes"), value_count("", ""))
        ));
    }
    l.extend(["".into(), "Recent titles:".into()]);
    l.extend(
        t.get("examples")
            .list()
            .iter()
            .map(|x| format!("  - {}", x.show())),
    );

    l.extend(["".into(), "## Body".into(), "".into()]);
    l.push(format!("- empty bodies: **{}%**", s(b, "empty_body_rate")));
    l.push(format!(
        "- length: p10 {} / median **{}** / p90 {} words",
        s(b, "p10_words"),
        s(b, "median_words"),
        s(b, "p90_words")
    ));
    l.push(format!(
        "- template HTML comments left in: {}%",
        s(b, "comments_retained_rate")
    ));
    l.push(format!(
        "- checklists: {}% (boxes actually ticked: {}%)",
        s(b, "checklist_rate"),
        s(b, "checked_box_rate")
    ));
    l.push(format!(
        "- images/screenshots: {}% · Slack links: {}%",
        s(b, "image_rate"),
        s(b, "slack_link_rate")
    ));
    let keyword = if b.get("preferred_link_keyword") == &J::Null {
        "none".to_string()
    } else {
        s(b, "preferred_link_keyword")
    };
    l.push(format!(
        "- issue references: {}% · preferred keyword: **{keyword}**",
        s(b, "issue_reference_rate")
    ));
    if !b.get("section_frequency").is_empty_list() {
        l.extend(["".into(), "Sections by frequency:".into()]);
        l.extend(
            b.get("section_frequency")
                .list()
                .iter()
                .map(|x| format!("  - {} — {}%", s(x, "section"), s(x, "rate"))),
        );
    }
    if !b.get("modal_section_order").is_empty_list() {
        l.extend([
            "".into(),
            format!(
                "Typical order: {}",
                b.get("modal_section_order")
                    .list()
                    .iter()
                    .map(J::show)
                    .collect::<Vec<_>>()
                    .join(" → ")
            ),
        ]);
    }
    if !b.get("routinely_hollow_sections").is_empty_list() {
        l.extend([
            "".into(),
            "Sections usually left hollow (fill these only if you have real content):".into(),
        ]);
        l.extend(
            b.get("routinely_hollow_sections")
                .list()
                .iter()
                .filter(|x| x.get("hollow_rate").number() >= 40.0)
                .map(|x| {
                    format!(
                        "  - {} — {}% hollow of {} seen",
                        s(x, "section"),
                        s(x, "hollow_rate"),
                        s(x, "seen")
                    )
                }),
        );
    }

    l.extend(["".into(), "## People".into(), "".into()]);
    l.push(format!(
        "- reviewers per PR: median **{}** (mean {})",
        s(ppl, "median_reviewers_per_pr"),
        s(ppl, "mean_reviewers_per_pr")
    ));
    for (label, key) in [
        ("frequent reviewers", "top_reviewers"),
        ("frequent approvers", "top_approvers"),
    ] {
        let list = ppl.get(key);
        if !list.is_empty_list() {
            let top: Vec<String> = list
                .list()
                .iter()
                .take(6)
                .map(value_count("@", ""))
                .collect();
            l.push(format!("- {label}: {}", top.join(", ")));
        }
    }
    let areas = ppl.get("reviewer_by_path").entries();
    if !areas.is_empty() {
        l.extend(["".into(), "Reviewers by area:".into()]);
        l.extend(
            areas
                .iter()
                .take(10)
                .map(|(path, rs)| format!("  - `{path}` → {}", joined(rs, value_count("@", "")))),
        );
    }

    l.extend(["".into(), "## Meta".into(), "".into()]);
    for f in m.get("required_label_families").list() {
        l.push(format!(
            "- **`{}` label appears on {}% of merged PRs — treat as mandatory.** Values: {}",
            s(f, "family"),
            s(f, "coverage"),
            joined(f.get("values"), |v| format!(
                "`{}` ({})",
                s(v, "value"),
                s(v, "count")
            ))
        ));
    }
    let freq = if m.get("label_frequency").is_empty_list() {
        String::new()
    } else {
        format!(
            " · {}",
            m.get("label_frequency")
                .list()
                .iter()
                .take(8)
                .map(value_count("", ""))
                .collect::<Vec<_>>()
                .join(", ")
        )
    };
    l.push(format!(
        "- labelled PRs: {}%{freq}",
        s(m, "labelled_pr_rate")
    ));
    l.push(format!(
        "- median size: {} lines across {} files",
        s(m, "median_lines_changed"),
        s(m, "median_files_changed")
    ));
    l.push(format!(
        "- base branches: {}",
        joined(m.get("base_branches"), value_count("", ""))
    ));
    l.join("\n") + "\n"
}

/// `os.path.splitext`: the root and the last extension of the final path
/// component, ignoring leading dots.
fn splitext(path: &str) -> (&str, &str) {
    let name_start = path.rfind('/').map_or(0, |i| i + 1);
    let name = &path[name_start..];
    let lead = name.len() - name.trim_start_matches('.').len();
    match name[lead..].rfind('.') {
        Some(i) => path.split_at(name_start + lead + i),
        None => (path, ""),
    }
}

fn load(path: &str) -> Result<Vec<Value>, String> {
    let text = fs::read_to_string(path)
        .map_err(|e| format!("could not read {path}: {}", os_error(&e, path)))?;
    let data: Value =
        serde_json::from_str(&text).map_err(|e| format!("could not read {path}: {e}"))?;
    let data = match data {
        Value::Object(map) => ["data", "prs"]
            .iter()
            .map(|k| map.get(*k).cloned().unwrap_or(Value::Null))
            .find(truthy)
            .unwrap_or(Value::Array(vec![])),
        other => other,
    };
    match data {
        Value::Array(list) => Ok(list),
        _ => Err("expected a JSON array of PRs".to_string()),
    }
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
    let dump = parsed.positional.unwrap_or_default();
    let prs = match load(&dump) {
        Ok(prs) => prs,
        Err(message) => {
            io.err.push_str(&format!("{message}\n"));
            return 1;
        }
    };
    let prof = analyse(&prs);
    let out = parsed
        .values
        .get("--out")
        .cloned()
        .unwrap_or_else(|| format!("{}.style.json", splitext(&dump).0));
    let md_path = format!("{}.md", splitext(&out).0);
    let write = |path: &str, content: String| {
        fs::write(path, content)
            .map_err(|e| format!("could not write {path}: {}", os_error(&e, path)))
    };
    let md = parsed.values.contains_key("--md");
    let written = write(&out, prof.dump()).and_then(|()| {
        if md {
            write(&md_path, render_md(&prof))
        } else {
            Ok(())
        }
    });
    if let Err(message) = written {
        io.err.push_str(&format!("{message}\n"));
        return 1;
    }
    io.out.push_str(&format!("profile -> {out}\n"));
    if md {
        io.out.push_str(&format!("summary -> {md_path}\n"));
    }
    io.out
        .push_str(&format!("sample_size={}\n", prof.get("sample_size").show()));
    if prof.get("warning") != &J::Null {
        io.out
            .push_str(&format!("WARNING: {}\n", prof.get("warning").show()));
    }
    0
}
