//! A stage's depth signals as routing configures them: the thresholds, risk terms and labels that select deep.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::js_regex::Regex;
use crate::manifest::{DepthChoice, DepthSignal, MeasuredValue};
use model_gateway::text::{is_js_whitespace, js_trim};

/// One stage's depth signals as routing configures them. Any one that fires selects deep.
#[derive(Clone, Debug, PartialEq)]
pub struct DepthSignalsConfig {
    /// Per measure, the value above which the stage goes deep.
    pub thresholds: Vec<(String, f64)>,
    /// Terms whose presence in the artifact, as a whole word and in any case, selects deep.
    pub risk_terms: Vec<String>,
    /// Tracker labels, in any case, that select deep.
    pub labels: Vec<String>,
}

/// The signals of a stage whose routing states none.
pub fn no_depth_signals() -> DepthSignalsConfig {
    DepthSignalsConfig { labels: Vec::new(), risk_terms: Vec::new(), thresholds: Vec::new() }
}

/// Reads a stage's `depth_signals`. Routing that names a measure its stage does not know is rejected.
pub fn read_depth_signals_config(fields: &Fields<'_>, stage: &str) -> Result<DepthSignalsConfig, ShapeError> {
    let known = measure_names(stage);
    let thresholds = if fields.has("thresholds") { fields.number_map("thresholds")? } else { Vec::new() };
    for (measure, _) in &thresholds {
        if !known.contains(&measure.as_str()) {
            let names = if known.is_empty() { "none".to_string() } else { known.join(", ") };
            return Err(ShapeError(format!(
                "stages.{stage}.depth_signals.thresholds.{measure} is not a measure of stage {stage}; it knows {names}"
            )));
        }
    }
    Ok(DepthSignalsConfig {
        labels: if fields.has("labels") { fields.strings("labels")? } else { Vec::new() },
        risk_terms: if fields.has("risk_terms") { fields.strings("risk_terms")? } else { Vec::new() },
        thresholds,
    })
}

/// The size measures a stage knows, by the name routing gives their thresholds, in the order the node runner's
/// table holds them. A stage gains depth signals by adding a measure there; the routing shape stays the same.
fn measure_names(stage: &str) -> &'static [&'static str] {
    match stage {
        "architecture-review" | "decision-recovery" => &["context_maps", "source_lines"],
        "adr-review" => &["adrs"],
        "plan" => &["steps", "stored_data_steps"],
        "decisions" => &["decisions", "words"],
        "feature-review" => &[],
        "spec" => &["user_stories", "words"],
        "ticket-review" => &["changed_lines", "risk_path_hits"],
        "tickets" => &["tickets"],
        _ => &[],
    }
}

/// What a stage's depth signals are read from. The spec stage needs the artifact's text and its tracker labels; a code
/// stage adds its change.
#[derive(Clone, Debug, Default)]
pub struct DepthEvidence<'a> {
    /// The artifact as the packet freezes it.
    pub text: &'a str,
    /// The artifact's tracker labels; empty for a markdown file.
    pub labels: &'a [String],
    /// A code stage's change, measured as risk triage measures it: its changed lines, and how many touched paths match
    /// a risk path.
    pub change: Option<(f64, f64)>,
    /// At the tickets stage: how many tickets the spec has.
    pub tickets: Option<usize>,
    /// At an ADR review: how many records are under review.
    pub adrs: Option<usize>,
    /// At decision recovery and architecture review: how many lines of source the repository tracks.
    pub source_lines: Option<usize>,
    /// At decision recovery and architecture review: how many context maps the repository has.
    pub context_maps: Option<usize>,
    /// At the plan stage: how many steps the plan has, and how many of its migrate and contract steps touch stored data.
    pub plan_steps: Option<usize>,
    pub stored_data_steps: Option<usize>,
}

/// The value of one measure of a stage.
fn measure(name: &str, evidence: &DepthEvidence) -> f64 {
    let count = |value: Option<usize>| value.unwrap_or(0) as f64;
    match name {
        "context_maps" => count(evidence.context_maps),
        "source_lines" => count(evidence.source_lines),
        "adrs" => count(evidence.adrs),
        "steps" => count(evidence.plan_steps),
        "stored_data_steps" => count(evidence.stored_data_steps),
        "decisions" => count_decisions(evidence.text),
        "words" => count_words(evidence.text),
        "user_stories" => count_user_stories(evidence.text),
        "changed_lines" => evidence.change.map_or(0.0, |change| change.0),
        "risk_path_hits" => evidence.change.map_or(0.0, |change| change.1),
        _ => count(evidence.tickets),
    }
}

/// Chooses a run's depth. An explicit depth always wins; the signals are still read, so the summary can show what they
/// would have chosen.
pub fn choose_depth(stage: &str, config: &DepthSignalsConfig, evidence: &DepthEvidence, explicit: Option<&str>) -> (String, DepthChoice) {
    let measures: Vec<(String, MeasuredValue)> = measure_names(stage)
        .iter()
        .filter_map(|name| {
            let threshold = config.thresholds.iter().find(|(measured, _)| measured == name)?.1;
            Some((name.to_string(), MeasuredValue { threshold, value: measure(name, evidence) }))
        })
        .collect();
    let carried: Vec<String> = evidence.labels.iter().map(|label| label.to_lowercase()).collect();
    let mut signals: Vec<DepthSignal> = measures
        .iter()
        .filter(|(_, measured)| measured.value > measured.threshold)
        .map(|(name, measured)| DepthSignal::Threshold { measure: name.clone(), threshold: measured.threshold, value: measured.value })
        .collect();
    signals.extend(config.risk_terms.iter().filter(|term| contains_term(evidence.text, term)).map(|term| DepthSignal::RiskTerm { term: term.clone() }));
    signals.extend(config.labels.iter().filter(|label| carried.contains(&label.to_lowercase())).map(|label| DepthSignal::Label { label: label.clone() }));
    let depth = match explicit {
        Some(depth) => depth.to_string(),
        None if signals.is_empty() => "base".to_string(),
        None => "deep".to_string(),
    };
    (depth, DepthChoice { chosen_by: if explicit.is_none() { "signals" } else { "argument" }.to_string(), measures, signals })
}

fn pattern(source: &str) -> Regex {
    Regex::new(source, false).expect("the depth patterns compile")
}

fn matches(regex: &Regex, text: &str) -> bool {
    regex.find_at(&text.chars().collect::<Vec<_>>(), 0).is_some()
}

/// The numbered list items under the spec's `User Stories` heading, up to the next heading of the same or a higher
/// level. Subheadings inside it, such as the groups #167 uses, do not end it.
fn count_user_stories(text: &str) -> f64 {
    let (heading, title, item) = (pattern(r"^(#{1,6})\s+(.*?)\s*#*\s*$"), Regex::with_flags(r"^user stories$", "i").expect("compiles"), pattern(r"^\s*\d+[.)]\s+[^\s]"));
    let mut level: Option<usize> = None;
    let mut count = 0.0;
    for line in text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)) {
        let chars: Vec<char> = line.chars().collect();
        if let Some(found) = heading.find_at(&chars, 0) {
            let depth = found.group(&chars, 1).unwrap_or_default().chars().count();
            if level.is_some_and(|level| depth <= level) {
                level = None;
            }
            let name: Vec<char> = found.group(&chars, 2).unwrap_or_default().chars().collect();
            if level.is_none() && title.find_at(&name, 0).is_some() {
                level = Some(depth);
            }
        } else if level.is_some() && matches(&item, line) {
            count += 1.0;
        }
    }
    count
}

/// The decisions a decisions-stage artifact records: each glossary term (a line opening with a bold term and a colon,
/// `**Term**:`) and each top-level list item, the shape a decision summary takes.
fn count_decisions(text: &str) -> f64 {
    let (term, entry) = (pattern(r"^\*\*[^*\n]+(?::\*\*|\*\*\s*:)"), pattern(r"^(?:[-*+]|\d+[.)])\s+[^\s]"));
    text.split('\n').map(|line| line.strip_suffix('\r').unwrap_or(line)).filter(|line| matches(&term, line) || matches(&entry, line)).count() as f64
}

fn count_words(text: &str) -> f64 {
    text.split(is_js_whitespace).filter(|word| !word.is_empty()).count() as f64
}

/// A letter, a number or an underscore: what the whole-word test of a risk term refuses beside the term.
fn is_term_character(c: char) -> bool {
    c.is_alphabetic() || c.is_numeric() || c == '_'
}

/// A character as `/iu` compares it: JavaScript's simple case folding, which the dotted capital I and the dotless
/// small i have none of.
fn fold(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) if c != '\u{130}' && c != '\u{131}' => single.to_lowercase().next().unwrap_or(c),
        _ => c,
    }
}

/// The term as a whole word or phrase, in any case; white space in it matches any run of white space.
fn contains_term(text: &str, term: &str) -> bool {
    let words: Vec<Vec<char>> = js_trim(term).split(is_js_whitespace).map(|word| word.chars().map(fold).collect()).collect();
    let text: Vec<char> = text.chars().collect();
    (0..=text.len()).any(|start| {
        if start > 0 && is_term_character(text[start - 1]) {
            return false;
        }
        let mut at = start;
        for (index, word) in words.iter().enumerate() {
            if index > 0 {
                let run = text[at..].iter().take_while(|c| is_js_whitespace(**c)).count();
                if run == 0 {
                    return false;
                }
                at += run;
            }
            if text.len() < at + word.len() || text[at..at + word.len()].iter().map(|c| fold(*c)).ne(word.iter().copied()) {
                return false;
            }
            at += word.len();
        }
        !text.get(at).is_some_and(|c| is_term_character(*c))
    })
}
