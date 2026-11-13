//! The refactoring plan's format, checked by a script: its sections, the fields
//! and kind of each step, the step graph, and the rules of the format a script
//! can judge. The node runner reads the plan with regular expressions; this file
//! is the same expressions written out by hand, so the crate needs no regular
//! expression engine.

use crate::decision_docs::without_fences;
use crate::tickets::TicketSize;
use model_gateway::json::number_text;
use model_gateway::text::{is_js_whitespace, js_trim};
use std::collections::{HashMap, HashSet};

/// The sections a refactoring plan holds, in the order the format lists them.
pub const PLAN_SECTIONS: [&str; 6] = ["Goal", "Target", "Safety net", "Steps", "Not doing", "Open decisions"];

/// The fields every step states, as `- **Field**: value` lines.
pub const PLAN_FIELDS: [&str; 10] = [
    "Kind",
    "Technique",
    "Anchor",
    "Touches",
    "Safety net",
    "Main stays green because",
    "Rollback",
    "Transitional element",
    "Blocked by",
    "Done when",
];

/// The one kind a step has.
pub const PLAN_KINDS: [&str; 8] = ["characterize", "tidy", "structure", "behaviour", "expand", "migrate", "contract", "remove-transitional"];

/// The words `\b(?:untested|not tested|no (?:existing )?tests?|nothing covers)\b` names, under `/i`.
const UNTESTED_WORDS: [&str; 7] = ["untested", "not tested", "no test", "no tests", "no existing test", "no existing tests", "nothing covers"];

/// The words `\b(?:tables?|columns?|migrations?|schemas?|databases?|stored data)\b` names, under `/i`.
const STORED_WORDS: [&str; 11] = ["table", "tables", "column", "columns", "migration", "migrations", "schema", "schemas", "database", "databases", "stored data"];

/// The extensions `\.(?:ts|tsx|...|html)\b` names, in the order the alternation tries them.
const EXTENSIONS: [&str; 21] = ["ts", "tsx", "js", "mjs", "cjs", "jsx", "py", "kt", "java", "go", "rs", "rb", "sh", "md", "json", "yaml", "yml", "toml", "sql", "css", "html"];

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanError {
    pub step: Option<String>,
    /// A short kebab-case id, one per rule.
    pub rule: String,
    pub message: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PlanStep {
    pub id: String,
    pub title: String,
    pub kind: String,
    pub blocked_by: Vec<String>,
}

/// What the decision log says about the ADRs a plan may anchor to; absent when no repository was given.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanAdrs {
    /// The ADR numbers the log holds.
    pub present: HashSet<u32>,
    /// The numbers of the records the log marks superseded.
    pub superseded: HashSet<u32>,
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct PlanCheck {
    pub errors: Vec<PlanError>,
    /// The migrate and contract steps that touch stored data, for the depth signal.
    pub stored_data_steps: Vec<String>,
    pub steps: Vec<PlanStep>,
    /// What was not checked, and why.
    pub notes: Vec<String>,
}

/// The checker's verdict: failed exactly while the plan holds an error.
pub fn plan_check_status(check: &PlanCheck) -> &'static str {
    if check.errors.is_empty() {
        "passed"
    } else {
        "failed"
    }
}

/// One step as the plan writes it, before the rules read it.
struct RawStep {
    id: String,
    title: String,
    fields: Vec<(String, Field)>,
    /// The non-field lines under the heading.
    prose: Vec<String>,
    /// Fields given more than once.
    duplicates: Vec<String>,
}

/// A field's line and its continuation lines, then its sub-bullets, joined: what a rule reads.
struct Field {
    value: String,
    text: String,
    /// The sub-bullets under the field line; empty when the value is a single line.
    items: Vec<String>,
}

/// A field while its continuation lines are still arriving.
struct FieldDraft {
    text: Vec<String>,
    items: Vec<String>,
}

/// The path-like tokens of a text, outside URLs.
pub fn paths_in(text: &str) -> Vec<String> {
    let stripped = replace_urls(text);
    let mut found = Vec::new();
    let mut at = 0;
    while at < stripped.len() {
        match path_like_at(&stripped, at) {
            Some(end) => {
                found.push(stripped[at..end].to_string());
                at = end;
            }
            None => at = advance(&stripped, at),
        }
    }
    found
}

/// Whether the Goal says behaviour changes: a sentence that says so and does not say it does not.
pub fn goal_changes_behaviour(goal: &str) -> bool {
    sentences(goal).into_iter().any(|sentence| says_change(sentence) && !denies_change(sentence))
}

/// Checks a refactoring plan: its sections, the fields and kind of each step, the step graph, and the
/// rules of the format that a script can judge. `size` is routing's ticket size; `adrs` is absent when
/// no repository was given.
pub fn check_plan(text: &str, size: &TicketSize, adrs: Option<&PlanAdrs>) -> PlanCheck {
    let mut errors: Vec<PlanError> = Vec::new();
    let mut notes: Vec<String> = Vec::new();
    let prose = without_fences(text);
    let lines = lines_of(&prose);
    let sections = sections_of(&lines, &mut errors);
    for name in PLAN_SECTIONS {
        if !sections.iter().any(|(section, _)| *section == name) {
            fail_plan(&mut errors, "missing-section", format!("the plan has no \"## {name}\" section"));
        }
    }
    let step_lines: &[&str] = match sections.iter().find(|(name, _)| *name == "Steps") {
        Some((_, lines)) => lines.as_slice(),
        None => &[],
    };
    let steps = parse_steps(step_lines, &mut errors);
    if sections.iter().any(|(name, _)| *name == "Steps") && steps.is_empty() {
        fail_plan(&mut errors, "no-steps", "the plan has no steps".to_string());
    }
    let known: Vec<&str> = steps.iter().map(|step| step.id.as_str()).collect();
    let kinds_of = last_values(&steps, kind_of);
    let edges = last_values(&steps, |step| ids_in(value_of(step, "Blocked by")));
    let goal = sections.iter().find(|(name, _)| *name == "Goal").map_or(String::new(), |(_, lines)| lines.join("\n"));
    let behaviour_goal = goal_changes_behaviour(&goal);
    let step_kinds: Vec<String> = steps.iter().flat_map(|step| kinds_in(&kind_of(step))).collect();

    for step in &steps {
        for name in PLAN_FIELDS {
            if value_of(step, name).is_empty() {
                fail(&mut errors, &step.id, "missing-field", format!("step {} has no \"{name}\" field", step.id));
            }
        }
        for name in unique(&step.duplicates) {
            fail(&mut errors, &step.id, "duplicate-field", format!("step {} gives \"{name}\" more than once", step.id));
        }
        for path in paths_in(&step.prose.join(" ")) {
            fail(&mut errors, &step.id, "path-outside-anchor", format!("step {} names the path {path} outside its fields; paths belong only in fact anchors", step.id));
        }
        let kinds = kinds_in(&kind_of(step));
        if kinds.len() > 1 {
            fail(&mut errors, &step.id, "multiple-kinds", format!("step {} has {} kinds ({}); a step has exactly one", step.id, kinds.len(), kinds.join(", ")));
        }
        for kind in &kinds {
            if !PLAN_KINDS.contains(&kind.as_str()) {
                fail(&mut errors, &step.id, "unknown-kind", format!("step {} has the unknown kind \"{kind}\"", step.id));
            }
        }

        let blocked = value_of(step, "Blocked by");
        if !blocked.is_empty() && !says_none(blocked) && ids_in(blocked).is_empty() {
            fail(&mut errors, &step.id, "blocked-by-unknown", format!("step {} is blocked by \"{blocked}\", which names no step", step.id));
        }
        for id in lookup(&edges, &step.id).into_iter().flatten() {
            if !known.contains(&id.as_str()) {
                fail(&mut errors, &step.id, "blocked-by-unknown", format!("step {} is blocked by {id}, which is no step of the plan", step.id));
            }
        }

        let transitional = value_of(step, "Transitional element");
        if !transitional.is_empty() && !says_none(transitional) {
            let removers = ids_in(transitional).into_iter().filter(|id| lookup(&kinds_of, id).is_some_and(|kind| kind == "remove-transitional")).count();
            if removers == 0 {
                fail(&mut errors, &step.id, "transitional-no-removal", format!("step {} adds a transitional element that no remove-transitional step removes", step.id));
            }
        }

        let anchor = value_of(step, "Anchor");
        if !anchor.is_empty() {
            check_anchor(&step.id, anchor, &known, adrs, &mut errors);
        }

        if kinds.iter().any(|kind| kind == "behaviour") && !behaviour_goal {
            fail(&mut errors, &step.id, "behaviour-not-in-goal", format!("step {} changes behaviour, but the Goal does not say behaviour changes", step.id));
        }

        if kinds.iter().any(|kind| kind == "structure")
            && says_untested(value_of(step, "Safety net"))
            && !ancestors_of(&step.id, &edges).iter().any(|id| lookup(&kinds_of, id).is_some_and(|kind| kind == "characterize"))
        {
            fail(&mut errors, &step.id, "untested-structure", format!("step {} restructures untested code with no characterize step before it", step.id));
        }

        for (name, field) in &step.fields {
            if name == "Anchor" || name == "Touches" {
                continue;
            }
            let mut parts = vec![field.text.as_str()];
            parts.extend(field.items.iter().map(String::as_str));
            for path in paths_in(&parts.join(" ")) {
                fail(&mut errors, &step.id, "path-outside-anchor", format!("step {} names the path {path} in \"{name}\"; paths belong only in fact anchors", step.id));
            }
        }

        let touches = count_of(field_of(step, "Touches"), &[',', ';']);
        if touches as f64 > size.max_modules {
            fail(&mut errors, &step.id, "too-many-touches", format!("step {} touches {touches} modules; the limit is {}", step.id, number_text(size.max_modules)));
        }
        let criteria = count_of(field_of(step, "Done when"), &[';']);
        if criteria as f64 > size.max_criteria {
            fail(&mut errors, &step.id, "too-many-criteria", format!("step {} has {criteria} done-when criteria; the limit is {}", step.id, number_text(size.max_criteria)));
        }

        if kinds.iter().any(|kind| kind == "expand") && !(step_kinds.iter().any(|kind| kind == "migrate") && step_kinds.iter().any(|kind| kind == "contract")) {
            let missing: Vec<&str> = ["migrate", "contract"].into_iter().filter(|kind| !step_kinds.iter().any(|known| known == kind)).collect();
            fail(&mut errors, &step.id, "expand-incomplete", format!("step {} expands, but the plan has no {} step", step.id, missing.join(" or ")));
        }
    }

    for cycle in cycles_of(&edges) {
        let message = if cycle.len() == 1 {
            format!("step {} blocks itself", cycle.first().map_or("", String::as_str))
        } else {
            format!("steps {} block each other in a cycle", cycle.join(", "))
        };
        fail_plan(&mut errors, "step-cycle", message);
    }
    for (section, lines) in &sections {
        if *section == "Goal" || *section == "Steps" {
            continue;
        }
        for path in paths_in(&lines.join("\n")) {
            fail_plan(&mut errors, "path-outside-anchor", format!("the \"{section}\" section names the path {path}; paths belong only in fact anchors"));
        }
    }
    if adrs.is_none() {
        notes.push("ADR ids were not checked against docs/adr: no repo was given".to_string());
    }

    let stored: Vec<String> = steps
        .iter()
        .filter(|step| {
            let kind = kind_of(step);
            let items = field_of(step, "Touches").map_or(String::new(), |field| field.items.join(" "));
            (kind == "migrate" || kind == "contract") && says_stored_data(&format!("{} {} {items}", value_of(step, "Touches"), value_of(step, "Technique")))
        })
        .map(|step| step.id.clone())
        .collect();

    PlanCheck {
        errors,
        notes,
        steps: steps
            .iter()
            .map(|step| PlanStep {
                blocked_by: lookup(&edges, &step.id).cloned().unwrap_or_default(),
                id: step.id.clone(),
                kind: kind_of(step),
                title: step.title.clone(),
            })
            .collect(),
        stored_data_steps: stored,
    }
}

fn fail(errors: &mut Vec<PlanError>, step: &str, rule: &str, message: String) {
    errors.push(PlanError { step: Some(step.to_string()), rule: rule.to_string(), message });
}

fn fail_plan(errors: &mut Vec<PlanError>, rule: &str, message: String) {
    errors.push(PlanError { step: None, rule: rule.to_string(), message });
}

/// A map built from the steps in order: a repeated id keeps its first place and its last value.
fn last_values<T>(steps: &[RawStep], value: impl Fn(&RawStep) -> T) -> Vec<(String, T)> {
    let mut pairs: Vec<(String, T)> = Vec::new();
    for step in steps {
        let found = value(step);
        match pairs.iter_mut().find(|(id, _)| *id == step.id) {
            Some(slot) => slot.1 = found,
            None => pairs.push((step.id.clone(), found)),
        }
    }
    pairs
}

fn lookup<'a, T>(pairs: &'a [(String, T)], id: &str) -> Option<&'a T> {
    pairs.iter().find(|(known, _)| known == id).map(|(_, value)| value)
}

fn value_of<'a>(step: &'a RawStep, name: &str) -> &'a str {
    field_of(step, name).map_or("", |field| field.value.as_str())
}

fn field_of<'a>(step: &'a RawStep, name: &str) -> Option<&'a Field> {
    step.fields.iter().find(|(known, _)| known == name).map(|(_, field)| field)
}

fn kind_of(step: &RawStep) -> String {
    value_of(step, "Kind").to_lowercase()
}

/// The values of a list without repeats, in the order they first appear.
fn unique(values: &[String]) -> Vec<&str> {
    let mut seen: Vec<&str> = Vec::new();
    for value in values {
        if !seen.contains(&value.as_str()) {
            seen.push(value.as_str());
        }
    }
    seen
}

/// `withoutFences(text).split(/\r?\n/)`: the plan's lines, a `\r\n` ending taken off.
fn lines_of(text: &str) -> Vec<&str> {
    let bytes = text.as_bytes();
    let mut lines = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < bytes.len() {
        if bytes[at] == b'\n' {
            let end = if at > start && bytes[at - 1] == b'\r' { at - 1 } else { at };
            lines.push(&text[start..end]);
            start = at + 1;
        }
        at += 1;
    }
    lines.push(&text[start..]);
    lines
}

/// `/^##[ \t]+(.*?)[ \t]*$/`: the name of a section heading.
fn section_heading(line: &str) -> Option<String> {
    let after = line.strip_prefix("##")?;
    let rest = skip_blanks(after);
    if rest.len() == after.len() {
        return None;
    }
    trailing_trimmed(rest)
}

/// `/^###[ \t]+(S\d+)[ \t]*:[ \t]*(.*?)[ \t]*$/`: the id and title of a step heading.
fn step_heading(line: &str) -> Option<(String, String)> {
    let after = line.strip_prefix("###")?;
    let rest = skip_blanks(after);
    if rest.len() == after.len() {
        return None;
    }
    let digits = rest.strip_prefix('S')?;
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    let rest = skip_blanks(&digits[count..]).strip_prefix(':')?;
    let title = trailing_trimmed(skip_blanks(rest))?;
    Some((format!("S{}", &digits[..count]), title))
}

/// `(.*?)[ \t]*$`: the text with its trailing blanks taken off, when it holds no line terminator.
fn trailing_trimmed(text: &str) -> Option<String> {
    if text.chars().any(is_line_terminator) {
        return None;
    }
    Some(text.trim_end_matches(|c| c == ' ' || c == '\t').to_string())
}

/// `/^[-*][ \t]+\*\*([^*]+?)(:)?\*\*[ \t]*(:)?[ \t]*(.*)$/`: a field line's name, its two optional colons and its value.
fn field_line(line: &str) -> Option<(String, bool, bool, String)> {
    let after = line.strip_prefix('-').or_else(|| line.strip_prefix('*'))?;
    let rest = skip_blanks(after);
    if rest.len() == after.len() {
        return None;
    }
    let rest = rest.strip_prefix("**")?;
    let bytes = rest.as_bytes();
    let mut at = 1;
    while at < bytes.len() && bytes[at - 1] != b'*' {
        let colon_before = bytes.get(at) == Some(&b':') && bytes.get(at + 1..at + 3) == Some(&b"**"[..]);
        let plain = bytes.get(at..at + 2) == Some(&b"**"[..]);
        if colon_before || plain {
            let name = rest[..at].to_string();
            let marker = if colon_before { 3 } else { 2 };
            let tail = skip_blanks(&rest[at + marker..]);
            let (colon_after, tail) = match tail.strip_prefix(':') {
                Some(rest) => (true, skip_blanks(rest)),
                None => (false, tail),
            };
            if tail.chars().any(is_line_terminator) {
                return None;
            }
            return Some((name, colon_before, colon_after, tail.to_string()));
        }
        at += 1;
    }
    None
}

/// `/^[ \t]+\S/`: the line starts with blanks and then a character that is not white space.
fn indented_content(line: &str) -> bool {
    let rest = skip_blanks(line);
    rest.len() < line.len() && rest.chars().next().is_some_and(|c| !is_js_whitespace(c))
}

/// `/^[ \t]+[-*][ \t]+(.*)$/`: the text of a sub-bullet, when the line holds no line terminator.
fn bullet_line(line: &str) -> Option<&str> {
    // Only an indented line is asked, so the blanks the pattern needs are there.
    let after = skip_blanks(line);
    let rest = after.strip_prefix('-').or_else(|| after.strip_prefix('*'))?;
    let value = skip_blanks(rest);
    if value.len() == rest.len() {
        return None;
    }
    (!value.chars().any(is_line_terminator)).then_some(value)
}

/// The plan field a name spells, under `/i`.
fn field_name(name: &str) -> Option<String> {
    let lowered = js_trim(name).to_lowercase();
    PLAN_FIELDS.iter().find(|known| known.to_lowercase() == lowered).map(|known| (*known).to_string())
}

/// The sections the plan holds, in the order they appear, with the lines under each.
fn sections_of<'a>(lines: &[&'a str], errors: &mut Vec<PlanError>) -> Vec<(&'static str, Vec<&'a str>)> {
    let mut sections: Vec<(&'static str, Vec<&'a str>)> = Vec::new();
    let mut current: Option<usize> = None;
    for &line in lines {
        let Some(name) = section_heading(line) else {
            if let Some(index) = current {
                sections[index].1.push(line);
            }
            continue;
        };
        let lowered = name.to_lowercase();
        match PLAN_SECTIONS.iter().find(|section| section.to_lowercase() == lowered) {
            None => {
                fail_plan(errors, "unknown-section", format!("the plan has an unknown section \"## {name}\"; its content is not checked"));
                current = None;
            }
            Some(&known) => {
                if sections.iter().any(|(section, _)| *section == known) {
                    fail_plan(errors, "duplicate-section", format!("the plan has more than one \"## {known}\" section; its second content is not checked"));
                    current = None;
                } else {
                    sections.push((known, Vec::new()));
                    current = Some(sections.len() - 1);
                }
            }
        }
    }
    sections
}

/// The fields, the repeated field names and the prose of one step's lines.
fn fields_of(lines: &[&str]) -> (Vec<(String, Field)>, Vec<String>, Vec<String>) {
    let mut drafts: Vec<FieldDraft> = Vec::new();
    let mut fields: Vec<(String, usize)> = Vec::new();
    let mut duplicates: Vec<String> = Vec::new();
    let mut prose: Vec<String> = Vec::new();
    let mut current: Option<usize> = None;
    for &line in lines {
        if js_trim(line).is_empty() {
            continue;
        }
        let parsed = field_line(line).and_then(|(name, before, after, value)| {
            let known = field_name(&name)?;
            (before || after).then_some((known, value))
        });
        if let Some((name, value)) = parsed {
            drafts.push(FieldDraft { text: vec![value], items: Vec::new() });
            current = Some(drafts.len() - 1);
            if fields.iter().any(|(known, _)| *known == name) {
                duplicates.push(name);
            } else {
                fields.push((name, drafts.len() - 1));
            }
        } else if let Some(index) = current.filter(|_| indented_content(line)) {
            match bullet_line(line) {
                Some(value) => drafts[index].items.push(js_trim(value).to_string()),
                None => drafts[index].text.push(js_trim(line).to_string()),
            }
        } else {
            current = None;
            prose.push(line.to_string());
        }
    }
    let built = fields
        .into_iter()
        .map(|(name, index)| {
            let draft = &drafts[index];
            let joined = js_trim(&draft.text.join(" ")).to_string();
            let mut parts = vec![joined.clone()];
            parts.extend(draft.items.iter().cloned());
            (name, Field { value: js_trim(&parts.join(" ")).to_string(), text: joined, items: draft.items.clone() })
        })
        .collect();
    (built, duplicates, prose)
}

/// The steps of the Steps section, in the order the plan writes them.
fn parse_steps(lines: &[&str], errors: &mut Vec<PlanError>) -> Vec<RawStep> {
    let mut steps: Vec<(String, String, Vec<&str>)> = Vec::new();
    let mut current: Option<usize> = None;
    for &line in lines {
        if !line.starts_with("###") {
            if let Some(index) = current {
                steps[index].2.push(line);
            }
            continue;
        }
        let Some((id, title)) = step_heading(line) else {
            fail_plan(errors, "bad-step-heading", format!("the heading \"{}\" is not \"### S01: <title>\"", js_trim(line)));
            current = None;
            continue;
        };
        if steps.iter().any(|(known, _, _)| *known == id) {
            fail(errors, &id, "duplicate-step", format!("step {id} is defined twice"));
        }
        current = Some(steps.len());
        steps.push((id, title, Vec::new()));
    }
    steps
        .into_iter()
        .map(|(id, title, lines)| {
            let (fields, duplicates, prose) = fields_of(&lines);
            RawStep { id, title, fields, prose, duplicates }
        })
        .collect()
}

/// The kinds a Kind value names: split on white space, commas, slashes, bars, plus signs, ampersands and "and".
fn kinds_in(value: &str) -> Vec<String> {
    value
        .split(|c: char| is_js_whitespace(c) || matches!(c, ',' | '/' | '|' | '+' | '&'))
        .filter(|kind| !kind.is_empty() && *kind != "and")
        .map(str::to_string)
        .collect()
}

/// `/\bS\d+\b/g`: the step ids a text names, uppercased.
fn ids_in(text: &str) -> Vec<String> {
    let mut ids = Vec::new();
    let mut at = 0;
    while at < text.len() {
        match step_id_at(text, at) {
            Some(end) => {
                ids.push(text[at..end].to_ascii_uppercase());
                at = end;
            }
            None => at = advance(text, at),
        }
    }
    ids
}

/// `\bS\d+\b` at `pos`: the end of the step id.
fn step_id_at(text: &str, pos: usize) -> Option<usize> {
    if !boundary_at(text, pos) || text.as_bytes().get(pos) != Some(&b'S') {
        return None;
    }
    let count = text.as_bytes()[pos + 1..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    if count == 0 {
        return None;
    }
    let end = pos + 1 + count;
    boundary_at(text, end).then_some(end)
}

/// How many things a field lists: its sub-bullets, else its comma or semicolon separated parts.
fn count_of(field: Option<&Field>, separators: &[char]) -> usize {
    let Some(field) = field else {
        return 0;
    };
    if !field.items.is_empty() {
        return field.items.len();
    }
    field.text.split(|c| separators.contains(&c)).filter(|part| !js_trim(part).is_empty()).count()
}

/// The members of every dependency cycle, one sorted list each: Tarjan's strongly connected components.
fn cycles_of(edges: &[(String, Vec<String>)]) -> Vec<Vec<String>> {
    let mut index: HashMap<&str, usize> = HashMap::new();
    let mut low: HashMap<&str, usize> = HashMap::new();
    let mut stack: Vec<&str> = Vec::new();
    let mut on_stack: HashSet<&str> = HashSet::new();
    let mut cycles: Vec<Vec<String>> = Vec::new();
    let mut counter = 0;
    let mut frames: Vec<(&str, &[String], usize)> = Vec::new();
    for (root, _) in edges {
        if index.contains_key(root.as_str()) {
            continue;
        }
        frames.push((root.as_str(), children_of(edges, root), 0));
        index.insert(root.as_str(), counter);
        low.insert(root.as_str(), counter);
        counter += 1;
        stack.push(root.as_str());
        on_stack.insert(root.as_str());
        while let Some((node, children, next)) = frames.pop() {
            if next < children.len() {
                let child = children[next].as_str();
                frames.push((node, children, next + 1));
                if !index.contains_key(child) {
                    index.insert(child, counter);
                    low.insert(child, counter);
                    counter += 1;
                    stack.push(child);
                    on_stack.insert(child);
                    frames.push((child, children_of(edges, child), 0));
                } else if on_stack.contains(child) {
                    let value = low.get(node).copied().unwrap_or(0).min(index.get(child).copied().unwrap_or(0));
                    low.insert(node, value);
                }
            } else {
                if low.get(node).copied().unwrap_or(0) == index.get(node).copied().unwrap_or(0) {
                    let mut members: Vec<String> = Vec::new();
                    while let Some(member) = stack.pop() {
                        on_stack.remove(member);
                        members.push(member.to_string());
                        if member == node {
                            break;
                        }
                    }
                    if members.len() > 1 || children_of(edges, node).iter().any(|child| child == node) {
                        members.sort();
                        cycles.push(members);
                    }
                }
                if let Some((parent, _, _)) = frames.last() {
                    let value = low.get(*parent).copied().unwrap_or(0).min(low.get(node).copied().unwrap_or(0));
                    low.insert(*parent, value);
                }
            }
        }
    }
    cycles
}

/// The steps a step is blocked by; none when the plan never defines it.
fn children_of<'a>(edges: &'a [(String, Vec<String>)], id: &str) -> &'a [String] {
    match edges.iter().find(|(known, _)| known == id) {
        Some((_, children)) => children.as_slice(),
        None => &[],
    }
}

/// Every step that reaches `id` through the blocked-by edges, in the order a breadth-first walk finds them.
fn ancestors_of(id: &str, edges: &[(String, Vec<String>)]) -> Vec<String> {
    let mut seen: Vec<String> = Vec::new();
    let mut queue: Vec<String> = children_of(edges, id).to_vec();
    let mut at = 0;
    while at < queue.len() {
        let next = queue[at].clone();
        at += 1;
        if seen.contains(&next) {
            continue;
        }
        seen.push(next.clone());
        queue.extend(children_of(edges, &next).iter().cloned());
    }
    seen
}

/// An anchor is a fact row (a backticked path, with an optional `#symbol`), an ADR id, or `prerequisite-of:Sn`.
fn check_anchor(step: &str, anchor: &str, known: &[&str], adrs: Option<&PlanAdrs>, errors: &mut Vec<PlanError>) {
    let prerequisites = prerequisites(anchor);
    let numbers = adr_numbers(anchor);
    let facts = fact_anchors(anchor).into_iter().filter(|span| !paths_in(span.split('#').next().unwrap_or("")).is_empty()).count();
    if prerequisites.len() + numbers.len() + facts == 0 {
        fail(errors, step, "anchor-unresolved", format!("step {step} is anchored to no fact row, ADR id or prerequisite-of:Sn"));
    }
    for id in &prerequisites {
        if !known.contains(&id.as_str()) {
            fail(errors, step, "anchor-unknown-step", format!("step {step} is anchored to prerequisite-of:{id}, which is no step of the plan"));
        }
    }
    let Some(adrs) = adrs else {
        return;
    };
    for number in numbers {
        let id = format!("ADR-{number:04}");
        if !adrs.present.contains(&number) {
            fail(errors, step, "adr-absent", format!("step {step} is anchored to {id}, which is not in docs/adr"));
        } else if adrs.superseded.contains(&number) {
            fail(errors, step, "adr-superseded", format!("step {step} is anchored to {id}, which is superseded"));
        }
    }
}

/// `` `([^`]+)` ``: the text inside each inline code span, left to right and without overlap.
fn fact_anchors(text: &str) -> Vec<&str> {
    let mut spans = Vec::new();
    let mut at = 0;
    while let Some(open) = text[at..].find('`') {
        let start = at + open + 1;
        match text[start..].find('`') {
            Some(end) if end > 0 => {
                spans.push(&text[start..start + end]);
                at = start + end + 1;
            }
            Some(_) => at = start,
            None => break,
        }
    }
    spans
}

/// `/\bADR[- ]?(\d{1,5})\b/gi`: the ADR numbers a text names.
fn adr_numbers(text: &str) -> Vec<u32> {
    let mut numbers = Vec::new();
    let mut at = 0;
    while at < text.len() {
        match adr_at(text, at) {
            Some((number, end)) => {
                numbers.push(number);
                at = end;
            }
            None => at = advance(text, at),
        }
    }
    numbers
}

/// `\bADR[- ]?(\d{1,5})\b` at `pos`: the number and the end of the match.
fn adr_at(text: &str, pos: usize) -> Option<(u32, usize)> {
    if !boundary_at(text, pos) || !starts_with_ci(&text[pos..], "adr") {
        return None;
    }
    let mut at = pos + 3;
    if matches!(text.as_bytes().get(at), Some(&b'-') | Some(&b' ')) {
        at += 1;
    }
    let count = text.as_bytes()[at..].iter().take_while(|byte| byte.is_ascii_digit()).count();
    // `\d{1,5}` greedy then `\b`: a sixth digit leaves no boundary, and no shorter run finds one either.
    if count == 0 || count > 5 {
        return None;
    }
    let end = at + count;
    boundary_at(text, end).then(|| (text[at..end].parse().unwrap_or(0), end))
}

/// `/\bprerequisite-of:[ \t]*(S\d+)\b/gi`: the step ids a text names as prerequisites, uppercased.
fn prerequisites(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut at = 0;
    while at < text.len() {
        match prerequisite_at(text, at) {
            Some((id, end)) => {
                found.push(id);
                at = end;
            }
            None => at = advance(text, at),
        }
    }
    found
}

/// `\bprerequisite-of:[ \t]*(S\d+)\b` at `pos`: the step id and the end of the match.
fn prerequisite_at(text: &str, pos: usize) -> Option<(String, usize)> {
    if !boundary_at(text, pos) || !starts_with_ci(&text[pos..], "prerequisite-of:") {
        return None;
    }
    let rest = skip_blanks(&text[pos + 16..]);
    let digits = rest.strip_prefix('S').or_else(|| rest.strip_prefix('s'))?;
    let count = digits.bytes().take_while(u8::is_ascii_digit).count();
    if count == 0 {
        return None;
    }
    let end = text.len() - rest.len() + 1 + count;
    boundary_at(text, end).then(|| (format!("S{}", &digits[..count]), end))
}

/// `/\b[a-z][a-z0-9+.-]*:\/\/\S+/gi`, replaced by one space: the text with its URLs taken out.
fn replace_urls(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut copied = 0;
    let mut at = 0;
    while at < text.len() {
        match url_at(text, at) {
            Some(end) => {
                out.push_str(&text[copied..at]);
                out.push(' ');
                copied = end;
                at = end;
            }
            None => at = advance(text, at),
        }
    }
    out.push_str(&text[copied..]);
    out
}

/// `\b[a-z][a-z0-9+.-]*:\/\/\S+` under `/i` at `pos`: the end of the URL.
fn url_at(text: &str, pos: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    if !boundary_at(text, pos) || !bytes.get(pos).is_some_and(u8::is_ascii_alphabetic) {
        return None;
    }
    let mut at = pos + 1;
    while bytes.get(at).is_some_and(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'+' | b'.' | b'-')) {
        at += 1;
    }
    if bytes.get(at..at + 3) != Some(&b"://"[..]) {
        return None;
    }
    let mut end = at + 3;
    for c in text[end..].chars() {
        if is_js_whitespace(c) {
            break;
        }
        end += c.len_utf8();
    }
    (end > at + 3).then_some(end)
}

/// The path-like token that starts at `pos`, if one does: the three alternatives, in the order the pattern tries them.
fn path_like_at(text: &str, pos: usize) -> Option<usize> {
    let repetitions = repetition_ends(text, pos);
    first_alternative(text, &repetitions).or_else(|| second_alternative(text, &repetitions)).or_else(|| third_alternative(text, pos))
}

/// `(?:[\w@.-]+/)+`: the position after each repetition, in the order the greedy repetition takes them.
fn repetition_ends(text: &str, pos: usize) -> Vec<usize> {
    let mut ends = Vec::new();
    let mut at = pos;
    loop {
        let count = run_len(text, at, is_path_char);
        if count == 0 || text.as_bytes().get(at + count) != Some(&b'/') {
            return ends;
        }
        at += count + 1;
        ends.push(at);
    }
}

/// `(?:[\w@.-]+/)+[\w@-]+\.[A-Za-z]\w{0,5}\b`: the end of the first alternative's match.
fn first_alternative(text: &str, repetitions: &[usize]) -> Option<usize> {
    for &after in repetitions.iter().rev() {
        let name = run_len(text, after, is_path_name_char);
        if name == 0 || text.as_bytes().get(after + name) != Some(&b'.') {
            continue;
        }
        let letter = after + name + 1;
        if !text.as_bytes().get(letter).is_some_and(u8::is_ascii_alphabetic) {
            continue;
        }
        let tail = run_len(text, letter + 1, is_word);
        // `\w{0,5}\b`: a longer run leaves every shorter one inside the word, so no boundary is reachable.
        if tail > 5 {
            continue;
        }
        return Some(letter + 1 + tail);
    }
    None
}

/// `(?:[\w@.-]+/){2,}[\w@.-]+`: the end of the second alternative's match.
fn second_alternative(text: &str, repetitions: &[usize]) -> Option<usize> {
    let count = repetitions.len();
    if count < 2 {
        return None;
    }
    let last = repetitions[count - 1];
    let tail = run_len(text, last, is_path_char);
    if tail > 0 {
        return Some(last + tail);
    }
    // One repetition fewer still leaves the two the pattern needs, and the run it gives up is never empty.
    (count >= 3).then(|| repetitions[count - 2] + run_len(text, repetitions[count - 2], is_path_char))
}

/// `(?<![\w.-])[a-z_][\w-]*\.(?:ts|tsx|...|html)\b`: the end of the third alternative's match.
fn third_alternative(text: &str, pos: usize) -> Option<usize> {
    if text[..pos].chars().next_back().is_some_and(is_word_or_dot_or_dash) {
        return None;
    }
    let first = text[pos..].chars().next()?;
    if !(first.is_ascii_lowercase() || first == '_') {
        return None;
    }
    let dot = pos + 1 + run_len(text, pos + 1, is_word_or_dash);
    if text.as_bytes().get(dot) != Some(&b'.') {
        return None;
    }
    let start = dot + 1;
    EXTENSIONS.iter().find_map(|extension| {
        let end = start + extension.len();
        (text.as_bytes().get(start..end) == Some(extension.as_bytes()) && boundary_at(text, end)).then_some(end)
    })
}

/// `\bbehaviou?r\b` under `/i` at `pos`: the length of the match.
fn behaviour_at(text: &str, pos: usize) -> Option<usize> {
    if !boundary_at(text, pos) || !starts_with_ci(&text[pos..], "behavio") {
        return None;
    }
    let after = pos + 7;
    if starts_with_ci(&text[after..], "ur") && boundary_at(text, after + 2) {
        return Some(9);
    }
    if starts_with_ci(&text[after..], "r") && boundary_at(text, after + 1) {
        return Some(8);
    }
    None
}

/// `\b(?:no|not|never|without)\b` under `/i` at `pos`: the length of the word.
fn negation_at(text: &str, pos: usize) -> Option<usize> {
    if !boundary_at(text, pos) {
        return None;
    }
    ["no", "not", "never", "without"]
        .into_iter()
        .find(|word| starts_with_ci(&text[pos..], word) && boundary_at(text, pos + word.len()))
        .map(str::len)
}

/// `goal.split(/(?<=[.!?])\s+|\n/)`: the sentences of the Goal.
fn sentences(goal: &str) -> Vec<&str> {
    let mut pieces = Vec::new();
    let mut start = 0;
    let mut at = 0;
    while at < goal.len() {
        let separator = separator_len(goal, at);
        if separator == 0 {
            at = advance(goal, at);
        } else {
            pieces.push(&goal[start..at]);
            start = at + separator;
            at = start;
        }
    }
    pieces.push(&goal[start..]);
    pieces
}

/// `(?<=[.!?])\s+|\n` at `pos`: the length of the separator.
fn separator_len(text: &str, pos: usize) -> usize {
    let after_sentence = text[..pos].chars().next_back().is_some_and(|c| matches!(c, '.' | '!' | '?'));
    if after_sentence {
        let run = whitespace_run(text, pos);
        if run > 0 {
            return run;
        }
    }
    usize::from(text.as_bytes().get(pos) == Some(&b'\n'))
}

/// `/\bbehaviou?r\b[^.\n]*\b(?:chang\w*|differ\w*|alter\w*)|\b(?:chang\w*|alter\w*)\b[^.\n]*\bbehaviou?r\b/i`.
fn says_change(sentence: &str) -> bool {
    let mut at = 0;
    while at < sentence.len() {
        if let Some(length) = behaviour_at(sentence, at) {
            if region_holds(sentence, at + length, &["chang", "differ", "alter"]) {
                return true;
            }
        }
        at = advance(sentence, at);
    }
    let mut at = 0;
    while at < sentence.len() {
        if boundary_at(sentence, at) && ["chang", "alter"].into_iter().any(|word| starts_with_ci(&sentence[at..], word)) {
            if region_holds_behaviour(sentence, at + word_run(sentence, at)) {
                return true;
            }
        }
        at = advance(sentence, at);
    }
    false
}

/// `/\b(?:no|not|never|without)\b(?:\W+\w+){0,3}?\W+behaviou?r\b|\bbehaviou?r\b(?:\W+\w+){0,2}?\W+(?:not|never)\W+(?:\w+\W+)?(?:chang|differ|alter)/i`.
fn denies_change(sentence: &str) -> bool {
    let mut at = 0;
    while at < sentence.len() {
        if let Some(length) = negation_at(sentence, at) {
            let runs = alternating_runs(sentence, at + length);
            for count in 0..=3 {
                if runs.get(2 * count + 1).is_some_and(|&(start, _)| behaviour_at(sentence, start).is_some()) {
                    return true;
                }
            }
        }
        at = advance(sentence, at);
    }
    let mut at = 0;
    while at < sentence.len() {
        if let Some(length) = behaviour_at(sentence, at) {
            let runs = alternating_runs(sentence, at + length);
            for count in 0..=2 {
                let Some(&(start, end)) = runs.get(2 * count + 1) else {
                    continue;
                };
                let word = &sentence[start..end];
                if !(word.eq_ignore_ascii_case("not") || word.eq_ignore_ascii_case("never")) || runs.get(2 * count + 2).is_none() {
                    continue;
                }
                let after = [runs.get(2 * count + 3), runs.get(2 * count + 5)];
                if after.into_iter().flatten().any(|&(start, _)| ["chang", "differ", "alter"].into_iter().any(|word| starts_with_ci(&sentence[start..], word))) {
                    return true;
                }
            }
        }
        at = advance(sentence, at);
    }
    false
}

/// `[^.\n]*\b(?:chang\w*|differ\w*|alter\w*)` from `from`: whether one of the words starts before the next `.` or newline.
fn region_holds(text: &str, from: usize, words: &[&str]) -> bool {
    let limit = region_end(text, from);
    let mut at = from;
    while at <= limit {
        if boundary_at(text, at) && words.iter().any(|word| starts_with_ci(&text[at..], word)) {
            return true;
        }
        at = advance(text, at);
    }
    false
}

/// `[^.\n]*\bbehaviou?r\b` from `from`: whether behaviour starts before the next `.` or newline.
fn region_holds_behaviour(text: &str, from: usize) -> bool {
    let limit = region_end(text, from);
    let mut at = from;
    while at <= limit {
        if behaviour_at(text, at).is_some() {
            return true;
        }
        at = advance(text, at);
    }
    false
}

/// The first `.` or newline at or after `from`, or the end of the text.
fn region_end(text: &str, from: usize) -> usize {
    text[from..].find(|c| c == '.' || c == '\n').map_or(text.len(), |at| from + at)
}

/// The runs after `pos`, alternating non-word and word runs, as `(start, end)` pairs. `pos` is at a
/// non-word character, so the first run is the non-word one the patterns count from.
fn alternating_runs(text: &str, pos: usize) -> Vec<(usize, usize)> {
    let mut runs: Vec<(usize, usize)> = Vec::new();
    let mut start = pos;
    let mut word = false;
    for (offset, c) in text[pos..].char_indices() {
        let at = pos + offset;
        if is_word(c) != word {
            if at > start {
                runs.push((start, at));
            }
            start = at;
            word = is_word(c);
        }
    }
    if start < text.len() {
        runs.push((start, text.len()));
    }
    runs
}

/// `/^\W*(?:none|n\/a|nothing)\W*$/i`: the text says none, and nothing else.
fn says_none(text: &str) -> bool {
    let trimmed = text.trim_matches(|c: char| !is_word(c));
    ["none", "n/a", "nothing"].into_iter().any(|word| trimmed.eq_ignore_ascii_case(word))
}

/// `/^\W*none\b|\b(?:untested|not tested|no (?:existing )?tests?|nothing covers)\b/i`.
fn says_untested(text: &str) -> bool {
    let first = text.trim_start_matches(|c: char| !is_word(c));
    if starts_with_ci(first, "none") && boundary_at(first, 4) {
        return true;
    }
    holds_word(text, &UNTESTED_WORDS)
}

/// `/\b(?:tables?|columns?|migrations?|schemas?|databases?|stored data)\b/i`.
fn says_stored_data(text: &str) -> bool {
    holds_word(text, &STORED_WORDS)
}

/// `\b(?:word|...)\b` under `/i`, searched anywhere: whether the text holds one of the words.
fn holds_word(text: &str, words: &[&str]) -> bool {
    let mut at = 0;
    while at < text.len() {
        if boundary_at(text, at) && words.iter().any(|word| starts_with_ci(&text[at..], word) && boundary_at(text, at + word.len())) {
            return true;
        }
        at = advance(text, at);
    }
    false
}

/// `\b` at `pos`: a word character on exactly one side.
fn boundary_at(text: &str, pos: usize) -> bool {
    let before = text[..pos].chars().next_back().is_some_and(is_word);
    let after = text[pos..].chars().next().is_some_and(is_word);
    before != after
}

/// `\w`: an ASCII word character.
fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// `[\w@.-]`: a character a path segment may hold.
fn is_path_char(c: char) -> bool {
    is_word(c) || matches!(c, '@' | '.' | '-')
}

/// `[\w@-]`: a character a path's last segment may hold.
fn is_path_name_char(c: char) -> bool {
    is_word(c) || matches!(c, '@' | '-')
}

/// `[\w-]`: a character a bare file name may hold.
fn is_word_or_dash(c: char) -> bool {
    is_word(c) || c == '-'
}

/// `[\w.-]`: the characters the third alternative's lookbehind refuses.
fn is_word_or_dot_or_dash(c: char) -> bool {
    is_word(c) || matches!(c, '.' | '-')
}

/// JavaScript's `/m` line terminators: `.` does not match them and `$` matches before them.
fn is_line_terminator(c: char) -> bool {
    matches!(c, '\n' | '\r' | '\u{2028}' | '\u{2029}')
}

/// `[ \t]*`.
fn skip_blanks(text: &str) -> &str {
    text.trim_start_matches(|c| c == ' ' || c == '\t')
}

/// The length of the run of characters `accept` takes from `pos`.
fn run_len(text: &str, pos: usize, accept: impl Fn(char) -> bool) -> usize {
    text[pos..].chars().take_while(|c| accept(*c)).map(char::len_utf8).sum()
}

/// `\w+`: the length of the word run from `pos`.
fn word_run(text: &str, pos: usize) -> usize {
    run_len(text, pos, is_word)
}

/// `\s+`: the length of the run of JavaScript white space from `pos`.
fn whitespace_run(text: &str, pos: usize) -> usize {
    run_len(text, pos, is_js_whitespace)
}

/// The next character boundary after `at`.
fn advance(text: &str, at: usize) -> usize {
    text[at..].chars().next().map_or(at + 1, |c| at + c.len_utf8())
}

/// `prefix` under `/i`: whether the text starts with it, folding ASCII only.
fn starts_with_ci(text: &str, prefix: &str) -> bool {
    text.get(..prefix.len()).is_some_and(|head| head.eq_ignore_ascii_case(prefix))
}

/// The rules the plan-critic's checks name; a finding's location names one beside one step, as `S03: mixed-change`.
pub const PLAN_RULES: [&str; 23] = [
    "mixed-change",
    "no-safety-net",
    "big-bang-change",
    "broken-main-order",
    "no-rollback",
    "scaffold-left",
    "unanchored-step",
    "stale-hotspot",
    "adr-contradiction",
    "closes-import-cycle",
    "premature-service-split",
    "single-adapter-seam",
    "deletion-without-decision",
    "plan-consequence",
    "internals-dependence",
    "decision-spread",
    "unexplained-co-change",
    "pass-through",
    "wide-interface",
    "needs-real-infrastructure",
    "coupled-to-internals",
    "no-assertion-or-pasted-value",
    "no-fake",
];

/// The packet path of the checker's report on the plan, which says which steps the plan has.
pub const PLAN_CHECK_PATH: &str = "check/plan.json";

/// The step and the rule a plan finding's location names: `S03: mixed-change`. Either is absent when the location
/// names none the plan has.
pub fn plan_target(location: &str, step_ids: &[String]) -> (Option<&'static str>, Option<String>) {
    use crate::tickets::names;
    let text: Vec<char> = location.chars().collect();
    let rule = PLAN_RULES.iter().find(|name| names(&format!(r"(?<![\w-]){name}(?![\w-])"), "i", &text)).copied();
    let step = step_ids.iter().find(|id| names(&format!(r"(?<![\w#/]){id}(?!\w)"), "i", &text));
    (rule, step.cloned())
}

/// The step ids the checker's report holds, or none when the text does not read as one.
pub fn plan_step_ids(report: &str) -> Vec<String> {
    let Ok(parsed) = model_gateway::json::parse(report) else { return Vec::new() };
    let Some(model_gateway::json::Value::Arr(steps)) = parsed.as_object().and_then(|object| object.get("steps")) else { return Vec::new() };
    steps.iter().filter_map(|step| step.as_object().and_then(|object| object.get("id")).and_then(|id| id.as_str()).map(str::to_string)).collect()
}
