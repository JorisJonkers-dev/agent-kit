//! What decision recovery reads of a repository: the history of the sampled files and the source it holds. The node
//! runner's `domain/recovery.ts`.

use crate::code_facts::code_unit_order;
use crate::repository_packet::TrackedFile;
use model_gateway::json::number_text;

/// One commit as the history reads it: what the git port's log gives.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct HistoryEntry {
    pub commit: String,
    pub date: String,
    pub subject: String,
    pub body: String,
    pub files: Vec<String>,
}

/// The packet path of the commit log, and the folder of the issues it names.
pub const HISTORY_LOG_PATH: &str = "history/log.txt";
pub const HISTORY_ISSUES_DIR: &str = "history/issues";

/// How many sampled files the log is asked about, and how many commits it holds at most.
pub const HISTORY_MAX_PATHS: usize = 200;
pub const HISTORY_MAX_COMMITS: usize = 200;
/// How many bytes the log may take, and how many issues it fetches: history is evidence, not a second tree.
pub const HISTORY_CAP_BYTES: usize = 40_960;
pub const HISTORY_MAX_ISSUES: usize = 10;
/// Each issue is cut to this many bytes, so the issues take 20 KiB at most.
pub const HISTORY_ISSUE_BYTES: usize = 2048;
/// What a recovery sample leaves of the packet cap for history, so the whole packet stays inside the cap.
pub const HISTORY_RESERVE_BYTES: usize = HISTORY_CAP_BYTES + HISTORY_MAX_ISSUES * HISTORY_ISSUE_BYTES + 512;

const SOURCE_EXTENSIONS: [&str; 29] = [
    "c", "cc", "cjs", "clj", "cpp", "cs", "ex", "exs", "go", "h", "hpp", "java", "js", "jsx", "kt", "kts", "lua", "mjs", "nix", "php", "py", "rb", "rs",
    "scala", "sh", "swift", "ts", "tsx", "zig",
];

/// Whether a tracked path is source code, by its extension: documents, data and configuration are not.
pub fn is_source_path(path: &str) -> bool {
    let name = path.rsplit('/').next().unwrap_or_default();
    match name.rfind('.') {
        Some(dot) if dot > 0 => SOURCE_EXTENSIONS.contains(&name[dot + 1..].to_lowercase().as_str()),
        _ => false,
    }
}

/// How many lines a file has: a final newline ends the last line rather than starting another.
pub fn line_count(text: &str) -> usize {
    if text.is_empty() {
        0
    } else {
        text.split('\n').count() - usize::from(text.ends_with('\n'))
    }
}

/// How many lines of source the tracked tree holds: the measure a recovery run's depth is chosen by.
pub fn source_lines(tracked: &[TrackedFile]) -> usize {
    tracked.iter().map(|file| file.text.as_deref().map_or(0, |text| if is_source_path(&file.path) { line_count(text) } else { 0 })).sum()
}

fn older_note(count: usize) -> String {
    format!("{count} older commits did not fit the history cap of {HISTORY_CAP_BYTES} bytes.\n")
}

/// What `git log` gave, as the packet's `history/log.txt`: each commit's message and the sampled files it changed;
/// and how many commits the cap left out.
pub fn render_history_log(entries: &[HistoryEntry], commit: &str) -> (String, usize) {
    let header = [
        "# History of the sampled files".to_string(),
        String::new(),
        format!("Commits reachable from `{commit}` that changed a file under `tree/`, newest first. Each message is its author's own text: quote it for a rationale, and never take it for settled fact."),
        String::new(),
    ]
    .join("\n");
    let mut text = header;
    let mut held = 0;
    for entry in entries {
        let mut lines = vec![format!("## {} {}", entry.commit, entry.date), format!("Files: {}", entry.files.join(", ")), String::new(), entry.subject.clone()];
        if !entry.body.is_empty() {
            lines.extend([String::new(), entry.body.clone()]);
        }
        lines.extend([String::new(), String::new()]);
        let block = lines.join("\n");
        if text.len() + block.len() + older_note(entries.len()).len() > HISTORY_CAP_BYTES {
            break;
        }
        text.push_str(&block);
        held += 1;
    }
    let left = entries.len() - held;
    if left > 0 {
        text.push_str(&older_note(left));
    }
    (text, left)
}

/// The issue numbers the commit messages cite as `#123`, in the order they first appear, at most `HISTORY_MAX_ISSUES`.
pub fn cited_issues(entries: &[HistoryEntry]) -> Vec<f64> {
    let mut numbers: Vec<f64> = Vec::new();
    for entry in entries {
        let text: Vec<char> = format!("{}\n{}", entry.subject, entry.body).chars().collect();
        for (at, _) in text.iter().enumerate().filter(|(_, c)| **c == '#') {
            // `(?<![\w/#])#(\d+)\b`
            if at > 0 && (is_word(text[at - 1]) || text[at - 1] == '/' || text[at - 1] == '#') {
                continue;
            }
            let digits: String = text[at + 1..].iter().take_while(|c| c.is_ascii_digit()).collect();
            let after = text.get(at + 1 + digits.len());
            if digits.is_empty() || after.is_some_and(|c| is_word(*c)) {
                continue;
            }
            let number = digits.parse::<f64>().unwrap_or(f64::INFINITY);
            if !numbers.contains(&number) {
                numbers.push(number);
            }
        }
    }
    numbers.truncate(HISTORY_MAX_ISSUES);
    numbers
}

/// The reference a cited issue number is read as: `#123`.
pub fn issue_reference(number: f64) -> String {
    format!("#{}", number_text(number))
}

fn is_word(c: char) -> bool {
    c.is_ascii_alphanumeric() || c == '_'
}

/// The source files a recovery sample takes after the files the records name and each module's entry file: the
/// shallowest first, since a module's top is where its decisions show, then by path.
pub fn source_sample(tracked: &[TrackedFile]) -> Vec<String> {
    let mut paths: Vec<String> = tracked.iter().filter(|file| file.text.is_some() && is_source_path(&file.path)).map(|file| file.path.clone()).collect();
    paths.sort_by(|a, b| a.split('/').count().cmp(&b.split('/').count()).then_with(|| code_unit_order(a, b)));
    paths
}
