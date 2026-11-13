//! When a ticket is likely too big for one fresh `/implement` context, and the limits routing gives the check.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::js_regex::Regex;
use model_gateway::json::{self, Value};
use model_gateway::text::js_trim;

/// When a ticket is likely too big for one fresh `/implement` context: it touches more than about `max_modules`
/// modules or has more than about `max_criteria` acceptance criteria. Routing configures both.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketSize {
    pub max_criteria: f64,
    pub max_modules: f64,
}

/// #181's starting values, for a stage whose routing states none.
pub fn default_ticket_size() -> TicketSize {
    TicketSize { max_criteria: 8.0, max_modules: 5.0 }
}

/// Reads a stage's `ticket_size`.
pub fn read_ticket_size(fields: &Fields<'_>) -> Result<TicketSize, ShapeError> {
    Ok(TicketSize { max_criteria: fields.number("max_criteria")?, max_modules: fields.number("max_modules")? })
}

/// The checker's verdict on a ticket set: failed exactly while the tickets hold a dependency cycle.
pub fn ticket_check_status(check: &TicketCheck) -> &'static str {
    if check.cycles.is_empty() {
        "passed"
    } else {
        "failed"
    }
}

/// One ticket of a spec, as its tracker holds it.
#[derive(Clone, Debug, PartialEq)]
pub struct Ticket {
    /// `#184` for a GitHub issue of the spec's repository, the `NN` of its file name for a local ticket.
    pub id: String,
    /// Where it lives: `host/owner/repo#<number>` for an issue, its repository path for a file.
    pub reference: String,
    pub title: String,
    pub text: String,
    /// The ids of the tickets that must be done first. A blocker that names no ticket of the spec keeps the form it
    /// was written in, so it shows as dangling.
    pub blocked_by: Vec<String>,
}

/// Every elementary dependency cycle, each as ticket ids where each ticket is blocked by the next and the last by the
/// first. A cycle starts at its earliest ticket in tracker order, and cycles come in the order of their first
/// tickets. A ticket that blocks itself is a cycle of one. Johnson's circuit search, so a dense knot of tickets is
/// still finite work per cycle.
pub fn dependency_cycles(tickets: &[Ticket]) -> Vec<Vec<String>> {
    // A `Map` built from the tickets keeps the last index of an id that repeats.
    let mut index = std::collections::HashMap::new();
    for (at, ticket) in tickets.iter().enumerate() {
        index.insert(ticket.id.as_str(), at);
    }
    let edges: Vec<Vec<usize>> = tickets
        .iter()
        .map(|ticket| {
            let mut next = Vec::new();
            for blocker in &ticket.blocked_by {
                if let Some(at) = index.get(blocker.as_str()) {
                    if !next.contains(at) {
                        next.push(*at);
                    }
                }
            }
            next
        })
        .collect();
    let mut search = Circuits { tickets, edges: &edges, start: 0, blocked: vec![false; tickets.len()], waiting: vec![Vec::new(); tickets.len()], path: Vec::new(), cycles: Vec::new() };
    for start in 0..tickets.len() {
        search.start = start;
        search.blocked = vec![false; tickets.len()];
        search.waiting = vec![Vec::new(); tickets.len()];
        search.circuit(start);
    }
    search.cycles
}

/// Johnson's search from one start ticket: the tickets on the path are blocked, and a ticket that led to no cycle
/// waits on its successors to be unblocked.
struct Circuits<'a> {
    tickets: &'a [Ticket],
    edges: &'a [Vec<usize>],
    start: usize,
    blocked: Vec<bool>,
    waiting: Vec<Vec<usize>>,
    path: Vec<usize>,
    cycles: Vec<Vec<String>>,
}

impl Circuits<'_> {
    fn unblock(&mut self, node: usize) {
        self.blocked[node] = false;
        for other in std::mem::take(&mut self.waiting[node]) {
            if self.blocked[other] {
                self.unblock(other);
            }
        }
    }

    fn circuit(&mut self, node: usize) -> bool {
        let mut found = false;
        self.path.push(node);
        self.blocked[node] = true;
        let next: Vec<usize> = self.edges[node].iter().copied().filter(|other| *other >= self.start).collect();
        for &other in &next {
            if other == self.start {
                self.cycles.push(self.path.iter().map(|at| self.tickets[*at].id.clone()).collect());
                found = true;
            } else if !self.blocked[other] && self.circuit(other) {
                found = true;
            }
        }
        if found {
            self.unblock(node);
        } else {
            for &other in &next {
                if !self.waiting[other].contains(&node) {
                    self.waiting[other].push(node);
                }
            }
        }
        self.path.pop();
        found
    }
}

/// One ticket file as `/to-tickets` writes it locally: `.scratch/<feature>/issues/<NN>-<slug>.md`.
pub struct LocalTicketFile {
    /// Its repository path.
    pub reference: String,
    pub text: String,
}

/// A pattern of this module; they are fixed, so one that does not compile is a bug.
fn pattern(source: &str, flags: &str) -> Regex {
    Regex::with_flags(source, flags).expect("a ticket pattern compiles")
}

/// `pattern.exec(text)`: the first match's capture `group`, as JavaScript's `?.[group]` reads it; 0 is the match.
fn capture(regex: &Regex, text: &str, group: usize) -> Option<String> {
    let chars: Vec<char> = text.chars().collect();
    let hit = regex.find_at(&chars, 0)?;
    if group == 0 {
        Some(chars[hit.start..hit.end].iter().collect())
    } else {
        hit.group(&chars, group)
    }
}

/// `pattern.test(text)`.
fn tests(regex: &Regex, text: &str) -> bool {
    let chars: Vec<char> = text.chars().collect();
    regex.find_at(&chars, 0).is_some()
}

/// `text.replace(pattern, '')` for a pattern without the `g` flag: the first match removed.
fn remove_first(regex: &Regex, text: &str) -> String {
    let chars: Vec<char> = text.chars().collect();
    match regex.find_at(&chars, 0) {
        Some(hit) => chars[..hit.start].iter().chain(&chars[hit.end..]).collect(),
        None => text.to_string(),
    }
}

/// `text.split(/\r?\n/)`.
fn lines_of(text: &str) -> Vec<&str> {
    text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).collect()
}

/// `text.match(/\d+/g)`: the runs of ASCII digits, as numbers.
fn digit_runs(text: &str) -> Vec<f64> {
    text.split(|c: char| !c.is_ascii_digit()).filter(|run| !run.is_empty()).map(|run| run.parse::<f64>().unwrap_or(f64::NAN)).collect()
}

/// `Number(text)` for a run of ASCII digits.
fn digits_number(text: &str) -> f64 {
    text.parse::<f64>().unwrap_or(f64::NAN)
}

/// Reads local ticket files. A ticket's id is the number its file name starts with, as written (`01`), or the file
/// name without `.md` when there is none. Its "Blocked by" names other tickets by number (`01`, `#1`, `01-schema.md`)
/// or by title; a name that matches no ticket is kept as written.
pub fn local_tickets(files: &[LocalTicketFile]) -> Vec<Ticket> {
    let md = pattern(r"\.md$", "i");
    let leading = pattern(r"^\d+", "");
    let heading = pattern(r"^#[ \t]+(.+?)[ \t]*$", "m");
    let numbered = pattern(r"^#?\d+\s*[:.\-–]\s*", "");
    let parsed: Vec<(Ticket, Vec<String>)> = files
        .iter()
        .map(|file| {
            let stem = remove_first(&md, file.reference.rsplit('/').next().unwrap_or_default());
            let id = capture(&leading, &stem, 0).unwrap_or_else(|| stem.clone());
            let title = capture(&heading, &file.text, 1).map_or_else(|| stem.clone(), |heading| remove_first(&numbered, &heading));
            let ticket = Ticket { id, reference: file.reference.clone(), title, text: file.text.clone(), blocked_by: Vec::new() };
            (ticket, blocker_names(&file.text))
        })
        .collect();
    let number = pattern(r"^#?(\d+)(?=$|[\s\-:.(])", "");
    let all_digits = pattern(r"^\d+$", "");
    let resolve = |name: &String| -> String {
        let found = match capture(&number, name, 1) {
            None => parsed.iter().find(|(ticket, _)| ticket.title.to_lowercase() == name.to_lowercase()),
            Some(number) => parsed.iter().find(|(ticket, _)| tests(&all_digits, &ticket.id) && digits_number(&ticket.id) == digits_number(&number)),
        };
        found.map_or_else(|| name.clone(), |(ticket, _)| ticket.id.clone())
    };
    parsed.iter().map(|(ticket, blockers)| Ticket { blocked_by: blockers.iter().map(resolve).collect(), ..ticket.clone() }).collect()
}

/// The names a ticket's "Blocked by" lists: on the line itself (`**Blocked by:** 01, 02`), or as list items under a
/// `Blocked by` line or heading. "None (can start immediately)" lists nothing.
fn blocker_names(text: &str) -> Vec<String> {
    let lines = lines_of(text);
    let line_start = pattern(r"^[\s>*_#-]*blocked by\b", "i");
    let Some(at) = lines.iter().position(|line| tests(&line_start, line)) else {
        return Vec::new();
    };
    let rest = remove_first(&pattern(r"^[\s>*_#-]*blocked by\b[\s*_:]*", "i"), lines[at]);
    let mut items = Vec::new();
    if !js_trim(&rest).is_empty() {
        items.push(rest);
    } else {
        let item = pattern(r"^\s*(?:[-*+]|\d+[.)])\s+(.*)$", "");
        for line in &lines[at + 1..] {
            match capture(&item, line, 1) {
                Some(text) => items.push(text),
                None if !js_trim(line).is_empty() || !items.is_empty() => break,
                None => {}
            }
        }
    }
    let emphasis = pattern(r#"[*_`"']+"#, "g");
    let no_blocker = pattern(r"^(none|n\/a|nothing|-+)\b", "i");
    let numbers_only = pattern(r"^(?:#?\d+\s*(?:and|&|\s)\s*)*#?\d+$", "i");
    items
        .iter()
        .flat_map(|item| item.split([',', ';']).map(str::to_string).collect::<Vec<_>>())
        .flat_map(|item| {
            let chars: Vec<char> = item.chars().collect();
            let mut kept = String::new();
            let mut from = 0;
            for hit in emphasis.find_all(&chars) {
                kept.extend(&chars[from..hit.start]);
                from = hit.end;
            }
            kept.extend(&chars[from..]);
            let name = js_trim(&kept).to_string();
            if name.is_empty() || tests(&no_blocker, &name) {
                Vec::new()
            } else if tests(&numbers_only, &name) {
                name.split(|c: char| !c.is_ascii_digit()).filter(|run| !run.is_empty()).map(str::to_string).collect()
            } else {
                vec![name]
            }
        })
        .collect()
}

/// What the checker reports about one ticket.
#[derive(Clone, Debug, PartialEq)]
pub struct TicketReport {
    pub id: String,
    pub reference: String,
    pub title: String,
    /// Its acceptance criteria: the task-list items under its "Acceptance criteria" heading, or anywhere before its
    /// comments when it has none.
    pub criteria: usize,
    pub blocked_by: Vec<String>,
    /// The user story numbers it cites.
    pub stories: Vec<f64>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct DanglingBlocker {
    pub ticket: String,
    /// The blocker as the ticket names it, which is no ticket of the spec.
    pub blocker: String,
}

/// Which spec user stories no ticket cites. Coverage is only checked when some ticket cites a story; otherwise
/// nothing shows whether the tickets cover the spec, and it is not checkable rather than covered.
#[derive(Clone, Debug, PartialEq)]
pub enum StoryCoverage {
    Checked { stories: Vec<f64>, uncited: Vec<f64> },
    NotCheckable { reason: &'static str },
}

#[derive(Clone, Debug, PartialEq)]
pub struct TicketCheck {
    pub tickets: Vec<TicketReport>,
    pub cycles: Vec<Vec<String>>,
    pub dangling_blockers: Vec<DanglingBlocker>,
    pub coverage: StoryCoverage,
}

/// Checks a spec's tickets against each other and against the spec.
pub fn check_tickets(spec: &str, tickets: &[Ticket]) -> TicketCheck {
    let reports: Vec<TicketReport> = tickets
        .iter()
        .map(|ticket| TicketReport {
            blocked_by: ticket.blocked_by.clone(),
            criteria: criteria_count(&ticket.text),
            id: ticket.id.clone(),
            reference: ticket.reference.clone(),
            stories: cited_stories(&ticket.text),
            title: ticket.title.clone(),
        })
        .collect();
    let dangling_blockers = tickets
        .iter()
        .flat_map(|ticket| {
            ticket
                .blocked_by
                .iter()
                .filter(|blocker| !tickets.iter().any(|other| &other.id == *blocker))
                .map(|blocker| DanglingBlocker { blocker: blocker.clone(), ticket: ticket.id.clone() })
        })
        .collect();
    TicketCheck { coverage: story_coverage(user_stories(spec), &reports), cycles: dependency_cycles(tickets), dangling_blockers, tickets: reports }
}

fn story_coverage(stories: Vec<f64>, reports: &[TicketReport]) -> StoryCoverage {
    if stories.is_empty() {
        return StoryCoverage::NotCheckable { reason: "the spec has no numbered user stories" };
    }
    let cited: Vec<f64> = reports.iter().flat_map(|report| report.stories.iter().copied()).collect();
    if cited.is_empty() {
        return StoryCoverage::NotCheckable { reason: "no ticket cites a user story" };
    }
    let uncited = stories.iter().copied().filter(|story| !cited.contains(story)).collect();
    StoryCoverage::Checked { stories, uncited }
}

/// The numbers of the top-level numbered items under the spec's "User Stories" heading, subheadings included.
pub fn user_stories(spec: &str) -> Vec<f64> {
    let lines = prose(spec);
    let item = pattern(r"^(\d+)[.)]\s", "");
    let mut numbers = Vec::new();
    for line in section(&lines, &pattern(r"^user stories$", "i")).unwrap_or_default() {
        if let Some(number) = capture(&item, line, 1).map(|text| digits_number(&text)) {
            if !numbers.contains(&number) {
                numbers.push(number);
            }
        }
    }
    numbers
}

fn criteria_count(text: &str) -> usize {
    let lines = prose(text);
    let scope = section(&lines, &pattern(r"^acceptance criteria$", "i")).unwrap_or_else(|| before_comments(&lines));
    let task_item = pattern(r"^\s*[-*+]\s+\[[ xX]\]\s", "");
    scope.iter().filter(|line| tests(&task_item, line)).count()
}

/// The longest range a citation such as "user stories 3–9" is expanded over.
const MAX_RANGE: f64 = 1000.0;

/// The user story numbers a ticket cites: "user story 3", "user stories 3, 5 and 7", "User stories: 3–5" or "user
/// story #3", outside code blocks. A range is expanded a story at a time from its low end; node's loop never ends
/// where numbers above 2^53 make adding one change nothing, and this one ends.
fn cited_stories(text: &str) -> Vec<f64> {
    let story = r"#?\d+(?:\s*[-–]\s*#?\d+)?";
    let citation = pattern(&format!(r"\buser\s+stor(?:y|ies)\b[\s:*_]*({story}(?:\s*(?:,\s*(?:and\s+|or\s+)?|and\s+|or\s+|&\s*){story})*)"), "gi");
    let separator = pattern(r",|\band\b|\bor\b|&", "g");
    let joined: Vec<char> = prose(text).join("\n").chars().collect();
    let mut cited: Vec<f64> = Vec::new();
    let mut add = |story: f64| {
        if !cited.contains(&story) {
            cited.push(story);
        }
    };
    for hit in citation.find_all(&joined) {
        let list: Vec<char> = hit.group(&joined, 1).unwrap_or_default().chars().collect();
        let mut parts = Vec::new();
        let mut from = 0;
        for cut in separator.find_all(&list) {
            parts.push(list[from..cut.start].iter().collect::<String>());
            from = cut.end;
        }
        parts.push(list[from..].iter().collect());
        for part in parts {
            let numbers = digit_runs(&part);
            let Some(&low) = numbers.first() else { continue };
            let high = numbers.get(1).copied().unwrap_or(low);
            let span = if high >= low && high - low <= MAX_RANGE { (high - low) as usize } else { 0 };
            for step in 0..=span {
                add(low + step as f64);
            }
            add(high);
        }
    }
    cited.sort_by(f64::total_cmp);
    cited
}

/// The lines of a markdown text outside fenced code blocks.
fn prose(text: &str) -> Vec<&str> {
    let fence = pattern(r"^\s*(?:```|~~~)", "");
    let mut fenced = false;
    lines_of(text)
        .into_iter()
        .filter(|line| {
            if tests(&fence, line) {
                fenced = !fenced;
                return false;
            }
            !fenced
        })
        .collect()
}

/// A markdown heading's level and title.
fn heading_of(line: &str) -> Option<(usize, String)> {
    let heading = pattern(r"^(#{1,6})[ \t]+(.*?)[ \t#]*$", "");
    let chars: Vec<char> = line.chars().collect();
    let hit = heading.find_at(&chars, 0)?;
    Some((hit.group(&chars, 1).unwrap_or_default().chars().count(), hit.group(&chars, 2).unwrap_or_default()))
}

/// The lines under the first heading whose title matches, up to the next heading of its level or above.
fn section<'a>(lines: &[&'a str], title: &Regex) -> Option<Vec<&'a str>> {
    let start = lines.iter().position(|line| tests(title, &heading_of(line).map(|(_, title)| title).unwrap_or_default()))?;
    let level = heading_of(lines[start]).map_or(1, |(level, _)| level);
    let rest = &lines[start + 1..];
    let end = rest.iter().position(|line| heading_of(line).is_some_and(|(other, _)| other <= level)).unwrap_or(rest.len());
    Some(rest[..end].to_vec())
}

/// The lines before a ticket's `## Comments`, where conversation is appended.
fn before_comments<'a>(lines: &[&'a str]) -> Vec<&'a str> {
    let comments = pattern(r"^comments$", "i");
    let end = lines.iter().position(|line| heading_of(line).is_some_and(|(_, title)| tests(&comments, &title))).unwrap_or(lines.len());
    lines[..end].to_vec()
}

/// The check as the command prints it, in the order the node runner's object literals put the keys.
pub fn check_json(check: &TicketCheck) -> Vec<(&'static str, Value)> {
    let strings = |items: &[String]| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
    let numbers = |items: &[f64]| Value::Arr(items.iter().map(|item| Value::Num(*item)).collect());
    let coverage = match &check.coverage {
        StoryCoverage::Checked { stories, uncited } => {
            json::object(vec![("status", Value::str("checked")), ("stories", numbers(stories)), ("uncited", numbers(uncited))])
        }
        StoryCoverage::NotCheckable { reason } => json::object(vec![("reason", Value::str(reason)), ("status", Value::str("not-checkable"))]),
    };
    let tickets = check
        .tickets
        .iter()
        .map(|ticket| {
            json::object(vec![
                ("blocked_by", strings(&ticket.blocked_by)),
                ("criteria", Value::count(ticket.criteria)),
                ("id", Value::str(&ticket.id)),
                ("reference", Value::str(&ticket.reference)),
                ("stories", numbers(&ticket.stories)),
                ("title", Value::str(&ticket.title)),
            ])
        })
        .collect();
    vec![
        ("coverage", coverage),
        ("cycles", Value::Arr(check.cycles.iter().map(|cycle| strings(cycle)).collect())),
        ("dangling_blockers", dangling_json(&check.dangling_blockers)),
        ("tickets", Value::Arr(tickets)),
    ]
}

fn dangling_json(dangling: &[DanglingBlocker]) -> Value {
    Value::Arr(dangling.iter().map(|item| json::object(vec![("blocker", Value::str(&item.blocker)), ("ticket", Value::str(&item.ticket))])).collect())
}

/// What a tickets-stage run keeps of the checker's report, so record can say whether the stage failed and fan-out
/// knows which tickets a finding may name: ticket ids, numbers and blocker names only.
pub fn summarise_check(check: &TicketCheck) -> Value {
    let mut pairs = vec![
        ("cycles", Value::Arr(check.cycles.iter().map(|cycle| Value::Arr(cycle.iter().map(|id| Value::str(id)).collect())).collect())),
        ("dangling_blockers", dangling_json(&check.dangling_blockers)),
        ("status", Value::str(ticket_check_status(check))),
        ("ticket_ids", Value::Arr(check.tickets.iter().map(|ticket| Value::str(&ticket.id)).collect())),
    ];
    match &check.coverage {
        StoryCoverage::Checked { uncited, .. } => pairs.push(("uncited_stories", Value::Arr(uncited.iter().map(|story| Value::Num(*story)).collect()))),
        StoryCoverage::NotCheckable { reason } => {
            pairs.push(("coverage_reason", Value::str(reason)));
            pairs.push(("uncited_stories", Value::Null));
        }
    }
    json::object(pairs)
}

/// The size limits as the ticket linter is given them, with the tickets whose criterion count is already above the
/// limit; the module count is the linter's judgement.
pub fn size_report(check: &TicketCheck, size: &TicketSize) -> Value {
    let over: Vec<Value> = check.tickets.iter().filter(|ticket| ticket.criteria as f64 > size.max_criteria).map(|ticket| Value::str(&ticket.id)).collect();
    json::object(vec![("max_criteria", Value::Num(size.max_criteria)), ("max_modules", Value::Num(size.max_modules)), ("over_criteria", Value::Arr(over))])
}

/// The statuses that close a local ticket.
const CLOSED_STATUSES: [&str; 4] = ["closed", "done", "resolved", "wontfix"];

/// Whether a local markdown ticket is still open: its first `Status:` line names no closed status. A ticket with no
/// such line is open. The node runner's `localTicketOpen` in `domain/feature-review.ts`.
pub fn local_ticket_open(text: &str) -> bool {
    let status = capture(&pattern(r"^[ \t>]*\**Status\**[ \t]*:\**[ \t]*(.*)$", "im"), text, 1);
    status.is_none_or(|status| {
        let value: String = status.chars().filter(|c| !matches!(c, '*' | '_' | '`')).collect();
        !CLOSED_STATUSES.contains(&js_trim(&value).to_lowercase().as_str())
    })
}

/// The rules a ticket finding's location may name.
pub const TICKET_RULES: [&str; 4] = ["untestable-criterion", "no-seam", "uncovered-requirement", "too-big"];

/// The rule and the ticket of the spec that a finding's location names, where it names them.
pub fn ticket_target(location: &str, ticket_ids: &[String]) -> (Option<&'static str>, Option<String>) {
    let text: Vec<char> = location.chars().collect();
    let rule = TICKET_RULES.iter().find(|name| names(&format!(r"(?<![\w-]){name}(?![\w-])"), "i", &text)).copied();
    let ticket = ticket_ids.iter().find(|id| names(&format!(r"(?<![\w#/]){}(?!\w)", escape_pattern(id)), "", &text));
    (rule, ticket.cloned())
}

/// Whether a pattern built from a finding's wording matches the location; a pattern the engine refuses matches nothing.
pub(crate) fn names(pattern: &str, flags: &str, text: &[char]) -> bool {
    Regex::with_flags(pattern, flags).is_ok_and(|regex| regex.find_at(text, 0).is_some())
}

/// `text.replace(/[.*+?^${}()|[\]\\]/g, '\\$&')`.
pub(crate) fn escape_pattern(text: &str) -> String {
    let mut escaped = String::new();
    for c in text.chars() {
        if ".*+?^${}()|[]\\".contains(c) {
            escaped.push('\\');
        }
        escaped.push(c);
    }
    escaped
}
