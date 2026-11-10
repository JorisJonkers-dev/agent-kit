//! The code-navigation eval: a frozen question set and the runner that asks it.
//!
//! `validate` checks the question file (schema, family counts, repo deny
//! list, pins). `run` snapshots each pinned repo with `git archive`, asks an
//! agent every question `n` times, grades the answers and appends one JSON
//! line per run to `results.jsonl`. `table` turns that file into per-question
//! and per-family Markdown tables. Everything that touches a process goes
//! through an injected [`Exec`], so the unit tests never spawn an agent or git.
//! std only: the JSON reader below is the crate's own.

use std::collections::HashMap;
use std::ffi::OsString;
use std::io::Write;
use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::Instant;

// ---------------------------------------------------------------- JSON

/// A parsed JSON value. Objects keep their key order.
#[derive(Debug, Clone, PartialEq)]
pub enum Json {
    Null,
    Bool(bool),
    Num(f64),
    Str(String),
    Arr(Vec<Json>),
    Obj(Vec<(String, Json)>),
}

impl Json {
    /// The value of `key` when `self` is an object holding it.
    pub fn get(&self, key: &str) -> Option<&Json> {
        match self {
            Json::Obj(items) => items.iter().find(|(k, _)| k == key).map(|(_, v)| v),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        match self {
            Json::Str(s) => Some(s),
            _ => None,
        }
    }

    pub fn as_arr(&self) -> Option<&[Json]> {
        match self {
            Json::Arr(a) => Some(a),
            _ => None,
        }
    }

    pub fn as_f64(&self) -> Option<f64> {
        match self {
            Json::Num(n) => Some(*n),
            _ => None,
        }
    }

    /// A non-negative whole number as `u64`.
    pub fn as_u64(&self) -> Option<u64> {
        match self {
            Json::Num(n) if *n >= 0.0 && n.fract() == 0.0 => Some(*n as u64),
            _ => None,
        }
    }
}

/// How deep JSON arrays and objects may nest.
const MAX_DEPTH: usize = 64;

/// Parse one complete JSON document.
pub fn parse_json(text: &str) -> Result<Json, String> {
    let mut p = Parser {
        chars: text.chars().collect(),
        pos: 0,
        depth: 0,
    };
    let value = p.value()?;
    p.skip_ws();
    if p.pos < p.chars.len() {
        return Err(format!("trailing characters at {}", p.pos));
    }
    Ok(value)
}

struct Parser {
    chars: Vec<char>,
    pos: usize,
    depth: usize,
}

impl Parser {
    fn skip_ws(&mut self) {
        while self.pos < self.chars.len() && self.chars[self.pos].is_whitespace() {
            self.pos += 1;
        }
    }

    fn peek(&self) -> Option<char> {
        self.chars.get(self.pos).copied()
    }

    fn expect(&mut self, c: char) -> Result<(), String> {
        if self.peek() == Some(c) {
            self.pos += 1;
            Ok(())
        } else {
            Err(format!("expected '{c}' at {}", self.pos))
        }
    }

    fn literal(&mut self, word: &str, value: Json) -> Result<Json, String> {
        for c in word.chars() {
            self.expect(c)?;
        }
        Ok(value)
    }

    fn value(&mut self) -> Result<Json, String> {
        self.skip_ws();
        match self.peek() {
            None => Err("unexpected end of input".to_string()),
            Some('n') => self.literal("null", Json::Null),
            Some('t') => self.literal("true", Json::Bool(true)),
            Some('f') => self.literal("false", Json::Bool(false)),
            Some('"') => self.string().map(Json::Str),
            Some('[') => self.nested(Parser::array),
            Some('{') => self.nested(Parser::object),
            Some(c) if c == '-' || c.is_ascii_digit() => self.number(),
            Some(c) => Err(format!("unexpected '{c}' at {}", self.pos)),
        }
    }

    fn number(&mut self) -> Result<Json, String> {
        let start = self.pos;
        while let Some(c) = self.peek() {
            if c.is_ascii_digit() || matches!(c, '-' | '+' | '.' | 'e' | 'E') {
                self.pos += 1;
            } else {
                break;
            }
        }
        let text: String = self.chars[start..self.pos].iter().collect();
        text.parse::<f64>()
            .map(Json::Num)
            .map_err(|_| format!("bad number '{text}' at {start}"))
    }

    fn hex4(&mut self) -> Result<u32, String> {
        let mut n = 0;
        for _ in 0..4 {
            let d = self
                .peek()
                .and_then(|c| c.to_digit(16))
                .ok_or_else(|| format!("bad \\u escape at {}", self.pos))?;
            n = n * 16 + d;
            self.pos += 1;
        }
        Ok(n)
    }

    fn string(&mut self) -> Result<String, String> {
        self.expect('"')?;
        let mut out = String::new();
        loop {
            let c = self.peek().ok_or("unterminated string")?;
            self.pos += 1;
            match c {
                '"' => return Ok(out),
                '\\' => {
                    let e = self.peek().ok_or("unterminated escape")?;
                    self.pos += 1;
                    match e {
                        '"' => out.push('"'),
                        '\\' => out.push('\\'),
                        '/' => out.push('/'),
                        'b' => out.push('\u{8}'),
                        'f' => out.push('\u{c}'),
                        'n' => out.push('\n'),
                        'r' => out.push('\r'),
                        't' => out.push('\t'),
                        'u' => out.push(self.unicode()?),
                        other => return Err(format!("bad escape '\\{other}'")),
                    }
                }
                c if (c as u32) < 0x20 => return Err("control character in string".to_string()),
                c => out.push(c),
            }
        }
    }

    /// The char after `\u`, joining a surrogate pair when one starts.
    fn unicode(&mut self) -> Result<char, String> {
        let hi = self.hex4()?;
        let code = if (0xD800..0xDC00).contains(&hi) {
            self.expect('\\')?;
            self.expect('u')?;
            let lo = self.hex4()?;
            if !(0xDC00..0xE000).contains(&lo) {
                return Err("bad surrogate pair".to_string());
            }
            0x10000 + ((hi - 0xD800) << 10) + (lo - 0xDC00)
        } else {
            hi
        };
        char::from_u32(code).ok_or_else(|| "bad unicode escape".to_string())
    }

    /// Runs a container parser one level deeper, refusing absurd nesting.
    fn nested(&mut self, parse: fn(&mut Parser) -> Result<Json, String>) -> Result<Json, String> {
        if self.depth >= MAX_DEPTH {
            return Err(format!("nesting deeper than {MAX_DEPTH} levels"));
        }
        self.depth += 1;
        let result = parse(self);
        self.depth -= 1;
        result
    }

    fn array(&mut self) -> Result<Json, String> {
        self.expect('[')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some(']') {
            self.pos += 1;
            return Ok(Json::Arr(items));
        }
        loop {
            items.push(self.value()?);
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some(']') => {
                    self.pos += 1;
                    return Ok(Json::Arr(items));
                }
                _ => return Err(format!("expected ',' or ']' at {}", self.pos)),
            }
        }
    }

    fn object(&mut self) -> Result<Json, String> {
        self.expect('{')?;
        let mut items = Vec::new();
        self.skip_ws();
        if self.peek() == Some('}') {
            self.pos += 1;
            return Ok(Json::Obj(items));
        }
        loop {
            self.skip_ws();
            let key = self.string()?;
            self.skip_ws();
            self.expect(':')?;
            items.push((key, self.value()?));
            self.skip_ws();
            match self.peek() {
                Some(',') => self.pos += 1,
                Some('}') => {
                    self.pos += 1;
                    return Ok(Json::Obj(items));
                }
                _ => return Err(format!("expected ',' or '}}' at {}", self.pos)),
            }
        }
    }
}

/// `s` as a quoted JSON string with the required escapes.
pub fn json_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if (c as u32) < 0x20 => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

// ------------------------------------------------------- question set

/// The five question families and the count each must have.
pub const FAMILIES: [(&str, usize); 5] = [
    ("name-known", 6),
    ("concept", 5),
    ("cross-repo", 4),
    ("refs", 3),
    ("freshness", 2),
];

#[derive(Debug, Clone, PartialEq)]
pub struct GoldFile {
    pub repo: String,
    pub path: String,
}

#[derive(Debug, Clone, PartialEq)]
pub struct Question {
    pub id: String,
    pub family: String,
    pub repo: String,
    pub question: String,
    pub gold_files: Vec<GoldFile>,
    pub must_contain: Vec<String>,
    pub must_not_contain: Vec<String>,
    pub commit: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct QuestionSet {
    pub pins: Vec<(String, String)>,
    pub questions: Vec<Question>,
}

impl QuestionSet {
    fn pin(&self, repo: &str) -> Option<&str> {
        self.pins
            .iter()
            .find(|(r, _)| r == repo)
            .map(|(_, s)| s.as_str())
    }
}

/// Whether `repo` may never appear in the question set.
pub fn is_denied(repo: &str) -> bool {
    let r = repo.to_lowercase();
    matches!(
        r.as_str(),
        "nix-config" | "fleet-infra" | "estate-kit-papers" | "coursework" | "msc-thesis" | "workspace"
    ) || r.starts_with("homelab-")
        || r.ends_with("-course-material")
        || r.contains("nedap")
}

fn is_sha(s: &str) -> bool {
    s.len() == 40 && s.chars().all(|c| c.is_ascii_hexdigit())
}

fn req_str(obj: &Json, key: &str, ctx: &str, errs: &mut Vec<String>) -> String {
    match obj.get(key).and_then(Json::as_str) {
        Some(s) => s.to_string(),
        None => {
            errs.push(format!("{ctx}: missing or non-string '{key}'"));
            String::new()
        }
    }
}

fn str_list(obj: &Json, key: &str, required: bool, ctx: &str, errs: &mut Vec<String>) -> Vec<String> {
    match obj.get(key) {
        None if !required => Vec::new(),
        Some(Json::Arr(items)) => {
            let mut out = Vec::new();
            for item in items {
                match item.as_str() {
                    Some(s) => out.push(s.to_string()),
                    None => errs.push(format!("{ctx}: '{key}' must hold only strings")),
                }
            }
            out
        }
        _ => {
            errs.push(format!("{ctx}: missing or non-array '{key}'"));
            Vec::new()
        }
    }
}

fn parse_question(q: &Json, index: usize, errs: &mut Vec<String>) -> Question {
    let ctx = match q.get("id").and_then(Json::as_str) {
        Some(id) => format!("question {id}"),
        None => format!("question #{}", index + 1),
    };
    let id = req_str(q, "id", &ctx, errs);
    let family = req_str(q, "family", &ctx, errs);
    let repo = req_str(q, "repo", &ctx, errs);
    let question = req_str(q, "question", &ctx, errs);
    req_str(q, "gold_answer", &ctx, errs);
    let mut gold_files = Vec::new();
    match q.get("gold_files").and_then(Json::as_arr) {
        Some(files) => {
            for f in files {
                gold_files.push(GoldFile {
                    repo: req_str(f, "repo", &ctx, errs),
                    path: req_str(f, "path", &ctx, errs),
                });
            }
        }
        None => errs.push(format!("{ctx}: missing or non-array 'gold_files'")),
    }
    let must_contain = str_list(q, "must_contain", true, &ctx, errs);
    if must_contain.is_empty() {
        errs.push(format!("{ctx}: 'must_contain' must not be empty"));
    }
    let must_not_contain = str_list(q, "must_not_contain", false, &ctx, errs);
    let commit = match q.get("commit") {
        None => None,
        Some(Json::Str(s)) => Some(s.clone()),
        Some(_) => {
            errs.push(format!("{ctx}: 'commit' must be a string"));
            None
        }
    };
    Question {
        id,
        family,
        repo,
        question,
        gold_files,
        must_contain,
        must_not_contain,
        commit,
    }
}

/// Read the question file's structure; every problem found is returned.
pub fn parse_questions(text: &str) -> Result<QuestionSet, Vec<String>> {
    let root = parse_json(text).map_err(|e| vec![format!("invalid JSON: {e}")])?;
    let mut errs = Vec::new();
    if root.get("schema").and_then(Json::as_u64) != Some(1) {
        errs.push("'schema' must be 1".to_string());
    }
    let mut pins = Vec::new();
    match root.get("pins") {
        Some(Json::Obj(items)) => {
            for (repo, sha) in items {
                match sha.as_str() {
                    Some(s) if is_sha(s) => pins.push((repo.clone(), s.to_string())),
                    _ => errs.push(format!("pin {repo}: not a 40-hex sha")),
                }
            }
        }
        _ => errs.push("missing or non-object 'pins'".to_string()),
    }
    let mut questions = Vec::new();
    match root.get("questions").and_then(Json::as_arr) {
        Some(items) => {
            for (i, q) in items.iter().enumerate() {
                questions.push(parse_question(q, i, &mut errs));
            }
        }
        None => errs.push("missing or non-array 'questions'".to_string()),
    }
    if errs.is_empty() {
        Ok(QuestionSet { pins, questions })
    } else {
        Err(errs)
    }
}

/// The semantic checks: counts, ids, deny list, pins.
pub fn check_set(set: &QuestionSet) -> Vec<String> {
    let mut errs = Vec::new();
    for (repo, _) in &set.pins {
        if is_denied(repo) {
            errs.push(format!("pin {repo}: repo is on the deny list"));
        }
        let safe = !repo.is_empty()
            && !repo.starts_with('.')
            && repo.chars().all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'));
        if !safe {
            errs.push(format!("pin {repo}: repo name must be letters, digits, '.', '_' or '-'"));
        }
    }
    let mut seen: Vec<&str> = Vec::new();
    for q in &set.questions {
        if seen.contains(&q.id.as_str()) {
            errs.push(format!("question {}: duplicate id", q.id));
        }
        seen.push(&q.id);
        if !FAMILIES.iter().any(|(f, _)| *f == q.family) {
            errs.push(format!("question {}: unknown family '{}'", q.id, q.family));
        }
        let mut repos = vec![q.repo.as_str()];
        repos.extend(q.gold_files.iter().map(|g| g.repo.as_str()));
        for repo in repos {
            if is_denied(repo) {
                errs.push(format!("question {}: repo {repo} is on the deny list", q.id));
            } else if set.pin(repo).is_none() {
                errs.push(format!("question {}: repo {repo} is not in pins", q.id));
            }
        }
        if let (Some(commit), Some(pin)) = (&q.commit, set.pin(&q.repo)) {
            if commit != pin {
                errs.push(format!("question {}: commit does not match the pin for {}", q.id, q.repo));
            }
        }
    }
    for (family, want) in FAMILIES {
        let have = set.questions.iter().filter(|q| q.family == family).count();
        if have != want {
            errs.push(format!("family {family}: expected {want} questions, found {have}"));
        }
    }
    errs
}

// ------------------------------------------------------ process seam

/// One process to run.
#[derive(Debug, Clone, PartialEq)]
pub struct Invocation {
    pub program: String,
    pub args: Vec<String>,
    pub env: Vec<(String, String)>,
    pub cwd: Option<PathBuf>,
}

/// What a finished process left behind.
#[derive(Debug, Clone, PartialEq)]
pub struct Output {
    pub code: i32,
    pub stdout: String,
    pub stderr: String,
}

/// Runs an [`Invocation`]; the unit tests inject a fake.
pub type Exec<'a> = &'a dyn Fn(&Invocation) -> Result<Output, String>;

/// The real thing: spawn the process and wait for it.
pub fn real_exec(inv: &Invocation) -> Result<Output, String> {
    let mut cmd = Command::new(&inv.program);
    cmd.args(&inv.args).envs(inv.env.iter().map(|(k, v)| (k, v)));
    if let Some(cwd) = &inv.cwd {
        cmd.current_dir(cwd);
    }
    let out = cmd.output().map_err(|e| format!("cannot run {}: {e}", inv.program))?;
    Ok(Output {
        code: out.status.code().unwrap_or(-1),
        stdout: String::from_utf8_lossy(&out.stdout).into_owned(),
        stderr: String::from_utf8_lossy(&out.stderr).into_owned(),
    })
}

fn git(args: &[&str]) -> Invocation {
    Invocation {
        program: "git".to_string(),
        args: args.iter().map(|s| s.to_string()).collect(),
        env: Vec::new(),
        cwd: None,
    }
}

/// Check every pin exists as a commit in its clone under `root`.
pub fn check_pins(set: &QuestionSet, root: &Path, exec: Exec) -> Vec<String> {
    let mut errs = Vec::new();
    for (repo, sha) in &set.pins {
        let dir = root.join(repo).to_string_lossy().into_owned();
        let spec = format!("{sha}^{{commit}}");
        let inv = git(&["-C", &dir, "cat-file", "-e", &spec]);
        match exec(&inv) {
            Ok(o) if o.code == 0 => {}
            _ => errs.push(format!("pin {repo}: {sha} is not a commit in {dir}")),
        }
    }
    errs
}

/// Snapshot one pinned repo into `dest` (skipped when `dest` exists).
pub fn snapshot(repo_dir: &Path, sha: &str, dest: &Path, exec: Exec) -> Result<(), String> {
    let name = dest.file_name().map(|n| n.to_string_lossy().into_owned()).unwrap_or_default();
    let marker = dest.with_file_name(format!("{name}.sha"));
    match std::fs::read_to_string(&marker) {
        Ok(found) if found.trim() == sha => return Ok(()),
        Ok(found) => {
            return Err(format!(
                "snapshot {} holds commit {}, not {sha}; use a new --out",
                dest.display(),
                found.trim()
            ))
        }
        Err(_) if dest.exists() => {
            return Err(format!("snapshot {} has no commit marker; use a new --out", dest.display()))
        }
        Err(_) => {}
    }
    let script = "mkdir -p \"$3\" && git -C \"$1\" archive --format=tar -o \"$3.tar\" \"$2\" \
                  && tar -x -C \"$3\" -f \"$3.tar\"; rc=$?; rm -f \"$3.tar\"; \
                  [ $rc -eq 0 ] || rm -rf \"$3\"; exit $rc";
    let inv = Invocation {
        program: "sh".to_string(),
        args: vec![
            "-c".to_string(),
            script.to_string(),
            "sh".to_string(),
            repo_dir.to_string_lossy().into_owned(),
            sha.to_string(),
            dest.to_string_lossy().into_owned(),
        ],
        env: Vec::new(),
        cwd: None,
    };
    match exec(&inv)? {
        o if o.code == 0 => std::fs::write(&marker, sha)
            .map_err(|e| format!("cannot write {}: {e}", marker.display())),
        o => Err(format!("snapshot of {} failed: {}", repo_dir.display(), o.stderr.trim())),
    }
}

// ------------------------------------------------------------ agents

/// Which agent CLI answers the questions.
#[derive(Debug, Clone, PartialEq)]
pub enum Agent {
    Claude,
    /// A shell command template; see [`build_invocation`].
    Generic(String),
}

impl Agent {
    pub fn name(&self) -> &'static str {
        match self {
            Agent::Claude => "claude",
            Agent::Generic(_) => "generic",
        }
    }
}

/// The prompt every agent gets for a question.
pub fn build_prompt(q: &Question) -> String {
    format!(
        "{}\n\nAnswer in a few sentences. Name the file paths the answer comes from, \
         written as <repo>/<path>. Stay read-only: do not edit, create or delete anything.",
        q.question
    )
}

/// `s` single-quoted for `sh`.
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
}

/// The process that asks the agent. The generic template gets `{model}`
/// substituted (quoted); the prompt travels only in `CODE_NAV_PROMPT`.
pub fn build_invocation(agent: &Agent, model: &str, prompt: &str, cwd: &Path) -> Invocation {
    match agent {
        Agent::Claude => Invocation {
            program: "claude".to_string(),
            args: ["-p", prompt, "--model", model, "--output-format", "stream-json", "--verbose", "--allowedTools", "Read", "Grep", "Glob"]
                .iter()
                .map(|s| s.to_string())
                .collect(),
            env: Vec::new(),
            cwd: Some(cwd.to_path_buf()),
        },
        Agent::Generic(template) => Invocation {
            program: "sh".to_string(),
            args: vec!["-c".to_string(), template.replace("{model}", &shell_quote(model))],
            env: vec![("CODE_NAV_PROMPT".to_string(), prompt.to_string())],
            cwd: Some(cwd.to_path_buf()),
        },
    }
}

/// What an agent run reported, before grading.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct AgentOut {
    pub answer: String,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_calls: u64,
    pub files_opened: Vec<String>,
}

fn num_field(v: &Json, key: &str) -> u64 {
    v.get(key).and_then(Json::as_u64).unwrap_or(0)
}

/// Read claude's `stream-json` output: count tool uses, note files Read,
/// take the answer and usage from the final `result` event.
pub fn parse_claude(stdout: &str) -> Result<AgentOut, String> {
    let mut out = AgentOut::default();
    let mut result_seen = false;
    for line in stdout.lines().filter(|l| !l.trim().is_empty()) {
        let ev = parse_json(line).map_err(|e| format!("bad stream-json line: {e}"))?;
        match ev.get("type").and_then(Json::as_str) {
            Some("assistant") => {
                let blocks = ev
                    .get("message")
                    .and_then(|m| m.get("content"))
                    .and_then(Json::as_arr)
                    .unwrap_or(&[]);
                for b in blocks.iter().filter(|b| b.get("type").and_then(Json::as_str) == Some("tool_use")) {
                    out.tool_calls += 1;
                    if b.get("name").and_then(Json::as_str) == Some("Read") {
                        if let Some(p) = b.get("input").and_then(|i| i.get("file_path")).and_then(Json::as_str) {
                            out.files_opened.push(p.to_string());
                        }
                    }
                }
            }
            Some("result") => {
                result_seen = true;
                if ev.get("is_error") == Some(&Json::Bool(true)) {
                    return Err("claude reported an error result".to_string());
                }
                out.answer = ev.get("result").and_then(Json::as_str).unwrap_or("").to_string();
                if let Some(u) = ev.get("usage") {
                    out.input_tokens = num_field(u, "input_tokens");
                    out.output_tokens = num_field(u, "output_tokens");
                    out.cache_read_tokens = num_field(u, "cache_read_input_tokens");
                }
            }
            _ => {}
        }
    }
    if result_seen {
        Ok(out)
    } else {
        Err("no result event in claude output".to_string())
    }
}

/// Read the generic adapter's single JSON object.
pub fn parse_generic(stdout: &str) -> Result<AgentOut, String> {
    let v = parse_json(stdout.trim()).map_err(|e| format!("malformed agent output: {e}"))?;
    let answer = v
        .get("answer")
        .and_then(Json::as_str)
        .ok_or("agent output lacks a string 'answer'")?
        .to_string();
    let files_opened = v
        .get("files_opened")
        .and_then(Json::as_arr)
        .unwrap_or(&[])
        .iter()
        .filter_map(|f| f.as_str().map(str::to_string))
        .collect();
    Ok(AgentOut {
        answer,
        input_tokens: num_field(&v, "input_tokens"),
        output_tokens: num_field(&v, "output_tokens"),
        cache_read_tokens: num_field(&v, "cache_read_tokens"),
        tool_calls: num_field(&v, "tool_calls"),
        files_opened,
    })
}

// ----------------------------------------------------------- grading

/// `(correct, gold_recall)` for an answer.
pub fn grade(q: &Question, answer: &str, files_opened: &[String]) -> (bool, f64) {
    let lower = answer.to_lowercase();
    let correct = q.must_contain.iter().all(|s| lower.contains(&s.to_lowercase()))
        && !q.must_not_contain.iter().any(|s| lower.contains(&s.to_lowercase()));
    let recall = if q.gold_files.is_empty() {
        1.0
    } else {
        // A gold file counts only under its own repo: `<repo>/<path>`.
        let hit = q
            .gold_files
            .iter()
            .filter(|g| {
                let full = format!("{}/{}", g.repo, g.path);
                answer.contains(&full) || files_opened.iter().any(|f| f.contains(&full))
            })
            .count();
        hit as f64 / q.gold_files.len() as f64
    };
    (correct, recall)
}

/// One graded run, as written to `results.jsonl`.
#[derive(Debug, Clone, PartialEq)]
pub struct ResultRow {
    pub id: String,
    pub family: String,
    pub repo: String,
    pub agent: String,
    pub model: String,
    pub run: u64,
    pub correct: bool,
    pub gold_recall: f64,
    pub input_tokens: u64,
    pub output_tokens: u64,
    pub cache_read_tokens: u64,
    pub tool_calls: u64,
    pub wall_ms: u64,
    pub error: Option<String>,
    pub answer: String,
}

impl ResultRow {
    pub fn to_json_line(&self) -> String {
        let error = match &self.error {
            Some(e) => json_string(e),
            None => "null".to_string(),
        };
        format!(
            "{{\"id\":{},\"family\":{},\"repo\":{},\"agent\":{},\"model\":{},\"run\":{},\"correct\":{},\
             \"gold_recall\":{:.4},\"input_tokens\":{},\"output_tokens\":{},\"cache_read_tokens\":{},\
             \"tool_calls\":{},\"wall_ms\":{},\"error\":{},\"answer\":{}}}",
            json_string(&self.id),
            json_string(&self.family),
            json_string(&self.repo),
            json_string(&self.agent),
            json_string(&self.model),
            self.run,
            self.correct,
            self.gold_recall,
            self.input_tokens,
            self.output_tokens,
            self.cache_read_tokens,
            self.tool_calls,
            self.wall_ms,
            error,
            json_string(&self.answer),
        )
    }

    pub fn from_json(v: &Json) -> Result<ResultRow, String> {
        let s = |k: &str| {
            v.get(k)
                .and_then(Json::as_str)
                .map(str::to_string)
                .ok_or_else(|| format!("result row lacks string '{k}'"))
        };
        Ok(ResultRow {
            id: s("id")?,
            family: s("family")?,
            repo: s("repo")?,
            agent: s("agent")?,
            model: s("model")?,
            run: num_field(v, "run"),
            correct: v.get("correct") == Some(&Json::Bool(true)),
            gold_recall: v.get("gold_recall").and_then(Json::as_f64).unwrap_or(0.0),
            input_tokens: num_field(v, "input_tokens"),
            output_tokens: num_field(v, "output_tokens"),
            cache_read_tokens: num_field(v, "cache_read_tokens"),
            tool_calls: num_field(v, "tool_calls"),
            wall_ms: num_field(v, "wall_ms"),
            error: v.get("error").and_then(Json::as_str).map(str::to_string),
            answer: s("answer")?,
        })
    }
}

/// Ask one question once and grade the answer. Failures become the row's
/// `error`; they never abort the caller.
pub fn run_question(q: &Question, run: u64, agent: &Agent, model: &str, cwd: &Path, exec: Exec) -> ResultRow {
    let inv = build_invocation(agent, model, &build_prompt(q), cwd);
    let start = Instant::now();
    let res = exec(&inv);
    let wall_ms = start.elapsed().as_millis() as u64;
    let parsed = match res {
        Err(e) => Err(e),
        Ok(o) if o.code != 0 => Err(format!("agent exited {}: {}", o.code, o.stderr.trim())),
        Ok(o) => match agent {
            Agent::Claude => parse_claude(&o.stdout),
            Agent::Generic(_) => parse_generic(&o.stdout),
        },
    };
    let (out, error) = match parsed {
        Ok(out) => (out, None),
        Err(e) => (AgentOut::default(), Some(e)),
    };
    let (correct, gold_recall) = if error.is_some() {
        (false, 0.0)
    } else {
        grade(q, &out.answer, &out.files_opened)
    };
    ResultRow {
        id: q.id.clone(),
        family: q.family.clone(),
        repo: q.repo.clone(),
        agent: agent.name().to_string(),
        model: model.to_string(),
        run,
        correct,
        gold_recall,
        input_tokens: out.input_tokens,
        output_tokens: out.output_tokens,
        cache_read_tokens: out.cache_read_tokens,
        tool_calls: out.tool_calls,
        wall_ms,
        error,
        answer: out.answer,
    }
}

// ------------------------------------------------------------ tables

pub fn median(values: &[f64]) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let mut v = values.to_vec();
    v.sort_by(|a, b| a.partial_cmp(b).unwrap());
    let n = v.len();
    if n % 2 == 1 {
        v[n / 2]
    } else {
        (v[n / 2 - 1] + v[n / 2]) / 2.0
    }
}

pub fn mean(values: &[f64]) -> f64 {
    if values.is_empty() {
        0.0
    } else {
        values.iter().sum::<f64>() / values.len() as f64
    }
}

fn fmt_num(n: f64) -> String {
    if n.fract() == 0.0 {
        format!("{}", n as i64)
    } else {
        format!("{n:.1}")
    }
}

fn cell(rows: &[&ResultRow]) -> String {
    if rows.is_empty() {
        return "-".to_string();
    }
    let good: Vec<&ResultRow> = rows.iter().copied().filter(|r| r.error.is_none()).collect();
    let errors = rows.len() - good.len();
    let ok = good.iter().filter(|r| r.correct).count();
    let tokens: Vec<f64> = good.iter().map(|r| (r.input_tokens + r.output_tokens) as f64).collect();
    let calls: Vec<f64> = good.iter().map(|r| r.tool_calls as f64).collect();
    let recall: Vec<f64> = good.iter().map(|r| r.gold_recall).collect();
    let mut cell = format!(
        "{}/{} ok, {} tok, {} calls, recall {:.2}",
        ok,
        rows.len(),
        fmt_num(median(&tokens)),
        fmt_num(median(&calls)),
        mean(&recall)
    );
    if errors > 0 {
        cell.push_str(&format!(", {errors} errors"));
    }
    cell
}

fn unique<'a>(items: impl Iterator<Item = &'a str>) -> Vec<&'a str> {
    let mut out: Vec<&str> = Vec::new();
    for i in items {
        if !out.contains(&i) {
            out.push(i);
        }
    }
    out
}

fn table(header: &[String], body: Vec<Vec<String>>) -> String {
    let mut out = format!("| {} |\n|{}|\n", header.join(" | "), vec!["---"; header.len()].join("|"));
    for row in body {
        out.push_str(&format!("| {} |\n", row.join(" | ")));
    }
    out
}

/// The per-question and per-family Markdown tables for `rows`.
pub fn render_tables(rows: &[ResultRow]) -> String {
    if rows.is_empty() {
        return "no results\n".to_string();
    }
    let label_of = |r: &ResultRow| format!("{}/{}", r.agent, r.model);
    let mut labels: Vec<String> = Vec::new();
    for r in rows {
        let l = label_of(r);
        if !labels.contains(&l) {
            labels.push(l);
        }
    }

    let mut head = vec!["question".to_string()];
    head.extend(labels.iter().cloned());
    let mut body = Vec::new();
    for id in unique(rows.iter().map(|r| r.id.as_str())) {
        let first = rows.iter().find(|r| r.id == id).unwrap();
        let mut line = vec![format!("{} ({}, {})", id, first.family, first.repo)];
        for l in &labels {
            let sel: Vec<&ResultRow> = rows.iter().filter(|r| r.id == id && label_of(r) == *l).collect();
            line.push(cell(&sel));
        }
        body.push(line);
    }
    let mut out = format!("Per question\n\n{}\n", table(&head, body));

    head[0] = "family".to_string();
    let mut body = Vec::new();
    for fam in unique(rows.iter().map(|r| r.family.as_str())) {
        let mut line = vec![fam.to_string()];
        for l in &labels {
            let sel: Vec<&ResultRow> = rows.iter().filter(|r| r.family == fam && label_of(r) == *l).collect();
            line.push(cell(&sel));
        }
        body.push(line);
    }
    out.push_str(&format!("Per family\n\n{}", table(&head, body)));
    out
}

/// Read a `results.jsonl` file's text.
pub fn parse_results(text: &str) -> Result<Vec<ResultRow>, String> {
    let mut rows = Vec::new();
    for (i, line) in text.lines().enumerate().filter(|(_, l)| !l.trim().is_empty()) {
        let v = parse_json(line).map_err(|e| format!("results line {}: {e}", i + 1))?;
        rows.push(ResultRow::from_json(&v).map_err(|e| format!("results line {}: {e}", i + 1))?);
    }
    Ok(rows)
}

// --------------------------------------------------------------- CLI

const USAGE: &str = "usage:
  code-nav-eval validate --questions <file> [--repos-root <dir>]
  code-nav-eval run --questions <file> --repos-root <dir> --agent <claude|generic> --model <id>
                    --runs <n> --out <dir> [--command '<template>'] [--only <id,...>] [--dry-run]
  code-nav-eval table --results <results.jsonl>
";

/// Flags of one subcommand: `value` flags take an argument, `switch` flags do not.
fn parse_flags(args: &[String], value: &[&str], switch: &[&str]) -> Result<HashMap<String, String>, String> {
    let mut map = HashMap::new();
    let mut it = args.iter();
    while let Some(a) = it.next() {
        let name = a.strip_prefix("--").unwrap_or("");
        if switch.contains(&name) {
            map.insert(name.to_string(), String::new());
        } else if value.contains(&name) {
            let v = it.next().ok_or(format!("{a} needs a value"))?;
            map.insert(name.to_string(), v.clone());
        } else {
            return Err(format!("unknown argument '{a}'"));
        }
    }
    Ok(map)
}

fn required<'a>(flags: &'a HashMap<String, String>, name: &str) -> Result<&'a str, String> {
    flags
        .get(name)
        .map(String::as_str)
        .ok_or(format!("--{name} is required"))
}

fn load_checked(path: &str, repos_root: Option<&str>, exec: Exec) -> Result<QuestionSet, Vec<String>> {
    let text = std::fs::read_to_string(path).map_err(|e| vec![format!("cannot read {path}: {e}")])?;
    let set = parse_questions(&text)?;
    let mut errs = check_set(&set);
    if let Some(root) = repos_root {
        errs.extend(check_pins(&set, Path::new(root), exec));
    }
    if errs.is_empty() {
        Ok(set)
    } else {
        Err(errs)
    }
}

fn cmd_validate(args: &[String], out: &mut dyn Write, err: &mut dyn Write, exec: Exec) -> i32 {
    let flags = match parse_flags(args, &["questions", "repos-root"], &[]) {
        Ok(f) => f,
        Err(e) => return usage_error(&e, err),
    };
    let questions = match required(&flags, "questions") {
        Ok(q) => q,
        Err(e) => return usage_error(&e, err),
    };
    match load_checked(questions, flags.get("repos-root").map(String::as_str), exec) {
        Ok(set) => {
            let _ = writeln!(out, "ok: {} questions", set.questions.len());
            0
        }
        Err(errs) => {
            for e in errs {
                let _ = writeln!(err, "error: {e}");
            }
            1
        }
    }
}

fn usage_error(msg: &str, err: &mut dyn Write) -> i32 {
    let _ = writeln!(err, "error: {msg}\n{USAGE}");
    2
}

fn cmd_table(args: &[String], out: &mut dyn Write, err: &mut dyn Write) -> i32 {
    let flags = match parse_flags(args, &["results"], &[]) {
        Ok(f) => f,
        Err(e) => return usage_error(&e, err),
    };
    let path = match required(&flags, "results") {
        Ok(p) => p,
        Err(e) => return usage_error(&e, err),
    };
    let rows = std::fs::read_to_string(path)
        .map_err(|e| format!("cannot read {path}: {e}"))
        .and_then(|t| parse_results(&t));
    match rows {
        Ok(rows) => {
            let _ = write!(out, "{}", render_tables(&rows));
            0
        }
        Err(e) => {
            let _ = writeln!(err, "error: {e}");
            1
        }
    }
}

fn cmd_run(args: &[String], out: &mut dyn Write, err: &mut dyn Write, exec: Exec) -> i32 {
    let flags = match parse_flags(
        args,
        &["questions", "repos-root", "agent", "model", "runs", "out", "command", "only"],
        &["dry-run"],
    ) {
        Ok(f) => f,
        Err(e) => return usage_error(&e, err),
    };
    let get = |n: &str| required(&flags, n).map(str::to_string);
    let parsed = (|| -> Result<_, String> {
        let agent = match get("agent")?.as_str() {
            "claude" => Agent::Claude,
            "generic" => Agent::Generic(get("command").map_err(|_| "--agent generic needs --command".to_string())?),
            other => return Err(format!("unknown agent '{other}' (claude or generic)")),
        };
        let runs: u64 = get("runs")?
            .parse()
            .ok()
            .filter(|n| *n >= 1)
            .ok_or("--runs must be a whole number of at least 1")?;
        Ok((get("questions")?, get("repos-root")?, agent, get("model")?, runs, get("out")?))
    })();
    let (questions, repos_root, agent, model, runs, out_dir) = match parsed {
        Ok(p) => p,
        Err(e) => return usage_error(&e, err),
    };
    let set = match load_checked(&questions, Some(&repos_root), exec) {
        Ok(s) => s,
        Err(errs) => {
            for e in errs {
                let _ = writeln!(err, "error: {e}");
            }
            return 1;
        }
    };
    let mut selected: Vec<&Question> = set.questions.iter().collect();
    if let Some(only) = flags.get("only") {
        let ids: Vec<&str> = only.split(',').collect();
        if let Some(bad) = ids.iter().find(|i| !set.questions.iter().any(|q| q.id == **i)) {
            let _ = writeln!(err, "error: --only names unknown question '{bad}'");
            return 1;
        }
        selected.retain(|q| ids.contains(&q.id.as_str()));
    }
    if flags.contains_key("dry-run") {
        for q in &selected {
            for run in 1..=runs {
                let _ = writeln!(out, "plan: {} run {}", q.id, run);
            }
        }
        return 0;
    }

    let out_dir = PathBuf::from(out_dir);
    let snap = out_dir.join("snap");
    if let Err(e) = std::fs::create_dir_all(&snap) {
        let _ = writeln!(err, "error: cannot create {}: {e}", snap.display());
        return 1;
    }
    for (repo, sha) in &set.pins {
        if let Err(e) = snapshot(&Path::new(&repos_root).join(repo), sha, &snap.join(repo), exec) {
            let _ = writeln!(err, "error: {e}");
            return 1;
        }
    }
    let results_path = out_dir.join("results.jsonl");
    let mut file = match std::fs::OpenOptions::new().create(true).append(true).open(&results_path) {
        Ok(f) => f,
        Err(e) => {
            let _ = writeln!(err, "error: cannot open {}: {e}", results_path.display());
            return 1;
        }
    };
    let mut rows = Vec::new();
    for q in &selected {
        for run in 1..=runs {
            let row = run_question(q, run, &agent, &model, &snap, exec);
            if let Err(e) = writeln!(file, "{}", row.to_json_line()) {
                let _ = writeln!(err, "error: cannot write results: {e}");
                return 1;
            }
            rows.push(row);
        }
    }
    let _ = write!(out, "{}", render_tables(&rows));
    0
}

/// Dispatch one command line; output and errors go to the given writers.
pub fn cli(args: &[String], out: &mut dyn Write, err: &mut dyn Write, exec: Exec) -> i32 {
    match args.first().map(String::as_str) {
        None | Some("help") | Some("--help") | Some("-h") => {
            let _ = write!(out, "{USAGE}");
            0
        }
        Some("validate") => cmd_validate(&args[1..], out, err, exec),
        Some("run") => cmd_run(&args[1..], out, err, exec),
        Some("table") => cmd_table(&args[1..], out, err),
        Some(other) => usage_error(&format!("unknown command '{other}'"), err),
    }
}

/// The process entry: the real stdout, stderr and process spawner.
pub fn run(args: impl IntoIterator<Item = OsString>) -> i32 {
    let args: Vec<String> = args.into_iter().map(|a| a.to_string_lossy().into_owned()).collect();
    cli(&args, &mut std::io::stdout(), &mut std::io::stderr(), &real_exec)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::cell::RefCell;
    use std::sync::atomic::{AtomicUsize, Ordering};

    const SHA_A: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SHA_B: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn temp_dir() -> PathBuf {
        static N: AtomicUsize = AtomicUsize::new(0);
        let d = std::env::temp_dir().join(format!(
            "code-nav-eval-test-{}-{}",
            std::process::id(),
            N.fetch_add(1, Ordering::SeqCst)
        ));
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ok(stdout: &str) -> Result<Output, String> {
        Ok(Output { code: 0, stdout: stdout.to_string(), stderr: String::new() })
    }

    fn s(v: &[&str]) -> Vec<String> {
        v.iter().map(|x| x.to_string()).collect()
    }

    fn question_json(id: &str, family: &str) -> String {
        format!(
            r#"{{"id":"{id}","family":"{family}","repo":"alpha","question":"Where is X?","gold_answer":"in a.rs",
            "gold_files":[{{"repo":"alpha","path":"src/a.rs"}}],"must_contain":["a.rs"],"extra":1}}"#
        )
    }

    /// A valid 20-question document; `tweak` can edit the text first.
    fn doc(tweak: impl Fn(String) -> String) -> String {
        let mut qs = Vec::new();
        for (fam, n) in FAMILIES {
            for i in 0..n {
                qs.push(question_json(&format!("{fam}-{i}"), fam));
            }
        }
        tweak(format!(
            r#"{{"schema":1,"pins":{{"alpha":"{SHA_A}","beta":"{SHA_B}"}},"questions":[{}]}}"#,
            qs.join(",")
        ))
    }

    fn q() -> Question {
        Question {
            id: "NK-01".into(),
            family: "name-known".into(),
            repo: "alpha".into(),
            question: "Where is X?".into(),
            gold_files: vec![GoldFile { repo: "alpha".into(), path: "src/a.rs".into() }],
            must_contain: s(&["Foo"]),
            must_not_contain: s(&["bar"]),
            commit: None,
        }
    }

    // ---- JSON

    #[test]
    fn json_scalars_and_nesting() {
        let v = parse_json(r#" {"a":[1,-2.5,1e2,true,false,null,"x"],"b":{"c":{}},"d":[]} "#).unwrap();
        let a = v.get("a").unwrap().as_arr().unwrap();
        assert_eq!(a[0].as_u64(), Some(1));
        assert_eq!(a[1].as_f64(), Some(-2.5));
        assert_eq!(a[1].as_u64(), None);
        assert_eq!(a[2].as_f64(), Some(100.0));
        assert_eq!(a[3], Json::Bool(true));
        assert_eq!(a[4], Json::Bool(false));
        assert_eq!(a[5], Json::Null);
        assert_eq!(a[6].as_str(), Some("x"));
        assert_eq!(v.get("b").unwrap().get("c"), Some(&Json::Obj(vec![])));
        assert_eq!(v.get("d"), Some(&Json::Arr(vec![])));
        assert_eq!(v.get("zzz"), None);
        assert_eq!(Json::Null.get("a"), None);
        assert_eq!(Json::Null.as_str(), None);
        assert_eq!(Json::Null.as_arr(), None);
        assert_eq!(Json::Null.as_f64(), None);
        assert_eq!(Json::Null.as_u64(), None);
        assert_eq!(Json::Num(-1.0).as_u64(), None);
    }

    #[test]
    fn json_string_escapes() {
        let v = parse_json(r#""a\"\\\/\b\f\n\r\t\u00e9\ud83d\ude00z""#).unwrap();
        assert_eq!(v.as_str(), Some("a\"\\/\u{8}\u{c}\n\r\t\u{e9}\u{1f600}z"));
    }

    #[test]
    fn json_errors() {
        for bad in [
            "", "nul", "tru", "@", "[1,", "[1 2]", "{\"a\" 1}", "{\"a\":1 \"b\":2}", "{1:2}", "\"abc",
            "\"\\", "\"\\x\"", "\"\\u12\"", "\"\\u12zz\"", "\"\\ud83d\"", "\"\\ud83d\\u0041\"",
            "\"\\udc00\"", "\"a\nb\"", "1 2", "-", "{", "[",
        ] {
            assert!(parse_json(bad).is_err(), "should fail: {bad:?}");
        }
    }

    #[test]
    fn json_string_quoting() {
        assert_eq!(json_string("a\"\\\n\r\t\u{1}é"), "\"a\\\"\\\\\\n\\r\\t\\u0001é\"");
        let round = "q\"\\\n\u{2}x";
        assert_eq!(parse_json(&json_string(round)).unwrap().as_str(), Some(round));
    }

    // ---- validation

    #[test]
    fn denied_repos() {
        for r in [
            "nix-config", "fleet-infra", "estate-kit-papers", "homelab-x", "coursework", "a-course-material",
            "msc-thesis", "workspace", "My-Nedap-Tool", "Workspace",
        ] {
            assert!(is_denied(r), "{r}");
        }
        for r in ["agent-kit", "homelab", "course-material-x", "workspaces"] {
            assert!(!is_denied(r), "{r}");
        }
    }

    #[test]
    fn valid_set_parses_and_checks() {
        let set = parse_questions(&doc(|d| d)).unwrap();
        assert_eq!(set.questions.len(), 20);
        assert_eq!(set.pins.len(), 2);
        assert_eq!(set.questions[0].gold_files[0].path, "src/a.rs");
        assert!(set.questions[0].must_not_contain.is_empty());
        assert!(check_set(&set).is_empty());
    }

    fn errs_of(d: String) -> Vec<String> {
        match parse_questions(&d) {
            Err(e) => e,
            Ok(set) => check_set(&set),
        }
    }

    fn has(errs: &[String], needle: &str) -> bool {
        errs.iter().any(|e| e.contains(needle))
    }

    #[test]
    fn structural_errors() {
        assert!(has(&errs_of("{".into()), "invalid JSON"));
        let e = errs_of(r#"{"schema":2,"pins":[],"questions":{}}"#.into());
        assert!(has(&e, "'schema'") && has(&e, "'pins'") && has(&e, "'questions'"));
        let e = errs_of(r#"{"schema":1,"pins":{"a":"short","b":5},"questions":[]}"#.into());
        assert!(has(&e, "pin a") && has(&e, "pin b"));
        let e = errs_of(format!(
            r#"{{"schema":1,"pins":{{"a":"{SHA_A}"}},"questions":[{{}},{{"id":"X","family":1}}]}}"#
        ));
        assert!(has(&e, "question #1: missing or non-string 'id'"));
        assert!(has(&e, "question X: missing or non-string 'family'"));
        assert!(has(&e, "'gold_files'") && has(&e, "'must_contain'") && has(&e, "'gold_answer'"));
        let e = errs_of(format!(
            r#"{{"schema":1,"pins":{{"a":"{SHA_A}"}},"questions":[{{"id":"X","family":"concept","repo":"a",
            "question":"q","gold_answer":"g","gold_files":[{{"repo":"a"}}],"must_contain":[1],
            "must_not_contain":"no","commit":5}}]}}"#
        ));
        assert!(has(&e, "missing or non-string 'path'"));
        assert!(has(&e, "'must_contain' must hold only strings"));
        assert!(has(&e, "missing or non-array 'must_not_contain'"));
        assert!(has(&e, "'commit' must be a string"));
    }

    #[test]
    fn semantic_errors() {
        let e = errs_of(doc(|d| d.replacen("name-known-1", "name-known-0", 1)));
        assert!(has(&e, "duplicate id"));
        let e = errs_of(doc(|d| d.replacen(r#""family":"concept""#, r#""family":"bogus""#, 1)));
        assert!(has(&e, "unknown family 'bogus'"));
        assert!(has(&e, "family concept: expected 5 questions, found 4"));
        let e = errs_of(doc(|d| d.replacen(r#""repo":"alpha","question""#, r#""repo":"nix-config","question""#, 1)));
        assert!(has(&e, "repo nix-config is on the deny list"));
        let e = errs_of(doc(|d| d.replacen(r#""repo":"alpha","question""#, r#""repo":"gamma","question""#, 1)));
        assert!(has(&e, "repo gamma is not in pins"));
        let e = errs_of(doc(|d| d.replacen(r#"{"repo":"alpha","path""#, r#"{"repo":"gamma","path""#, 1)));
        assert!(has(&e, "repo gamma is not in pins"));
        let e = errs_of(doc(|d| d.replace(&format!(r#""alpha":"{SHA_A}""#), r#""workspace":"cccccccccccccccccccccccccccccccccccccccc""#)));
        assert!(has(&e, "pin workspace: repo is on the deny list"));
        let e = errs_of(doc(|d| d.replacen(r#""extra":1"#, &format!(r#""commit":"{SHA_B}""#), 1)));
        assert!(has(&e, "commit does not match the pin"));
        let e = errs_of(doc(|d| d.replacen(r#""extra":1"#, &format!(r#""commit":"{SHA_A}""#), 1)));
        assert!(e.is_empty(), "{e:?}");
    }

    #[test]
    fn pin_checks_use_git() {
        let set = parse_questions(&doc(|d| d)).unwrap();
        let seen = RefCell::new(Vec::new());
        let exec = |i: &Invocation| {
            seen.borrow_mut().push(i.clone());
            if i.args[1].ends_with("/beta") {
                ok("").map(|mut o| {
                    o.code = 1;
                    o
                })
            } else {
                ok("")
            }
        };
        let errs = check_pins(&set, Path::new("/root"), &exec);
        assert_eq!(errs.len(), 1);
        assert!(errs[0].contains("pin beta"));
        let calls = seen.borrow();
        assert_eq!(calls[0].program, "git");
        assert_eq!(calls[0].args, s(&["-C", "/root/alpha", "cat-file", "-e", &format!("{SHA_A}^{{commit}}")]));
        let failing = |_: &Invocation| Err("no git".to_string());
        assert_eq!(check_pins(&set, Path::new("/r"), &failing).len(), 2);
    }

    // ---- prompt, adapters

    #[test]
    fn prompt_and_quote() {
        let p = build_prompt(&q());
        assert!(p.starts_with("Where is X?"));
        assert!(p.contains("<repo>/<path>") && p.contains("read-only"));
        assert_eq!(shell_quote("a'b"), "'a'\\''b'");
    }

    #[test]
    fn invocations() {
        let cwd = Path::new("/snap");
        let c = build_invocation(&Agent::Claude, "m1", "P", cwd);
        assert_eq!(c.program, "claude");
        assert_eq!(c.args[..4], s(&["-p", "P", "--model", "m1"]));
        assert!(c.args.ends_with(&s(&["--allowedTools", "Read", "Grep", "Glob"])));
        assert_eq!(c.cwd.as_deref(), Some(cwd));
        let g = build_invocation(&Agent::Generic("run --m {model} \"$CODE_NAV_PROMPT\"".into()), "m'1", "P $(x)", cwd);
        assert_eq!(g.program, "sh");
        assert_eq!(g.args, s(&["-c", "run --m 'm'\\''1' \"$CODE_NAV_PROMPT\""]));
        assert_eq!(g.env, vec![("CODE_NAV_PROMPT".to_string(), "P $(x)".to_string())]);
        assert!(!g.args[1].contains("$(x)"));
        assert_eq!(Agent::Claude.name(), "claude");
        assert_eq!(Agent::Generic(String::new()).name(), "generic");
    }

    const STREAM: &str = r#"{"type":"system","subtype":"init"}

{"type":"assistant","message":{"content":[{"type":"text","text":"hi"},{"type":"tool_use","name":"Read","input":{"file_path":"/s/alpha/src/a.rs"}},{"type":"tool_use","name":"Grep","input":{"pattern":"x"}},{"type":"tool_use","name":"Read","input":{}}]}}
{"type":"assistant"}
{"type":"user"}
{"type":"result","is_error":false,"result":"See alpha/src/a.rs","usage":{"input_tokens":10,"output_tokens":5,"cache_read_input_tokens":7}}
"#;

    #[test]
    fn claude_stream() {
        let o = parse_claude(STREAM).unwrap();
        assert_eq!(o.tool_calls, 3);
        assert_eq!(o.files_opened, s(&["/s/alpha/src/a.rs"]));
        assert_eq!(o.answer, "See alpha/src/a.rs");
        assert_eq!((o.input_tokens, o.output_tokens, o.cache_read_tokens), (10, 5, 7));
        let bare = parse_claude(r#"{"type":"result","result":"x"}"#).unwrap();
        assert_eq!((bare.answer.as_str(), bare.input_tokens), ("x", 0));
        let noresult = parse_claude(r#"{"type":"result"}"#).unwrap();
        assert_eq!(noresult.answer, "");
        assert!(parse_claude("{\"type\":\"system\"}").unwrap_err().contains("no result"));
        assert!(parse_claude("not json").unwrap_err().contains("bad stream-json"));
        assert!(parse_claude(r#"{"type":"result","is_error":true,"result":"x"}"#).is_err());
    }

    #[test]
    fn generic_output() {
        let o = parse_generic(r#" {"answer":"a","input_tokens":3,"output_tokens":4,"cache_read_tokens":1,"tool_calls":2,"files_opened":["f",5]} "#).unwrap();
        assert_eq!(o, AgentOut { answer: "a".into(), input_tokens: 3, output_tokens: 4, cache_read_tokens: 1, tool_calls: 2, files_opened: s(&["f"]) });
        let min = parse_generic(r#"{"answer":"a"}"#).unwrap();
        assert_eq!((min.cache_read_tokens, min.files_opened.len()), (0, 0));
        assert!(parse_generic("garbage").unwrap_err().contains("malformed"));
        assert!(parse_generic("{}").unwrap_err().contains("answer"));
    }

    // ---- grading

    #[test]
    fn grading() {
        let mut k = q();
        assert_eq!(grade(&k, "FOO in src/a.rs", &[]), (true, 0.0));
        assert_eq!(grade(&k, "foo in alpha/src/a.rs", &[]), (true, 1.0));
        assert_eq!(grade(&k, "foo", &s(&["/snap/alpha/src/a.rs"])), (true, 1.0));
        assert_eq!(grade(&k, "foo", &s(&["/snap/alpha/other.rs"])), (true, 0.0));
        assert_eq!(grade(&k, "nothing", &[]), (false, 0.0));
        assert!(!grade(&k, "foo BAR", &[]).0);
        k.gold_files.push(GoldFile { repo: "alpha".into(), path: "b.rs".into() });
        assert_eq!(grade(&k, "foo alpha/src/a.rs", &[]).1, 0.5);
        k.gold_files.clear();
        assert_eq!(grade(&k, "foo", &[]), (true, 1.0));
    }

    // ---- run_question

    #[test]
    fn run_question_paths() {
        let cwd = Path::new("/snap");
        let g = Agent::Generic("x".into());
        let good = |_: &Invocation| ok(r#"{"answer":"Foo in alpha/src/a.rs","input_tokens":2,"output_tokens":3,"tool_calls":4}"#);
        let r = run_question(&q(), 2, &g, "m", cwd, &good);
        assert!(r.correct && r.error.is_none());
        assert_eq!((r.run, r.input_tokens, r.output_tokens, r.tool_calls, r.gold_recall), (2, 2, 3, 4, 1.0));
        assert_eq!((r.agent.as_str(), r.model.as_str(), r.family.as_str()), ("generic", "m", "name-known"));

        let cl = |_: &Invocation| ok(STREAM);
        let r = run_question(&q(), 1, &Agent::Claude, "m", cwd, &cl);
        assert!(!r.correct && r.error.is_none());
        assert_eq!(r.tool_calls, 3);

        let bad = |_: &Invocation| ok("nope");
        assert!(run_question(&q(), 1, &g, "m", cwd, &bad).error.unwrap().contains("malformed"));
        let nz = |_: &Invocation| Ok(Output { code: 3, stdout: String::new(), stderr: " boom\n".into() });
        let r = run_question(&q(), 1, &g, "m", cwd, &nz);
        assert_eq!(r.error.as_deref(), Some("agent exited 3: boom"));
        assert!(!r.correct);
        let fail = |_: &Invocation| Err("spawn".to_string());
        assert_eq!(run_question(&q(), 1, &g, "m", cwd, &fail).error.as_deref(), Some("spawn"));
    }

    // ---- results and tables

    fn row(id: &str, fam: &str, model: &str, correct: bool, tok: u64, calls: u64, recall: f64) -> ResultRow {
        ResultRow {
            id: id.into(), family: fam.into(), repo: "alpha".into(), agent: "generic".into(), model: model.into(),
            run: 1, correct, gold_recall: recall, input_tokens: tok, output_tokens: 0, cache_read_tokens: 0,
            tool_calls: calls, wall_ms: 5, error: None, answer: "a \"b\"\nc".into(),
        }
    }

    #[test]
    fn result_line_roundtrip() {
        let mut r = row("A", "concept", "m", true, 10, 2, 0.5);
        let line = r.to_json_line();
        assert!(!line.contains('\n'));
        assert!(line.contains("\"error\":null") && line.contains("a \\\"b\\\"\\nc"));
        let back = ResultRow::from_json(&parse_json(&line).unwrap()).unwrap();
        assert_eq!(back, r);
        r.error = Some("bad \"x\"".into());
        let back = ResultRow::from_json(&parse_json(&r.to_json_line()).unwrap()).unwrap();
        assert_eq!(back.error.as_deref(), Some("bad \"x\""));
        assert!(ResultRow::from_json(&parse_json("{}").unwrap()).unwrap_err().contains("'id'"));
    }

    #[test]
    fn stats() {
        assert_eq!(median(&[]), 0.0);
        assert_eq!(median(&[3.0, 1.0, 2.0]), 2.0);
        assert_eq!(median(&[4.0, 1.0, 2.0, 3.0]), 2.5);
        assert_eq!(mean(&[]), 0.0);
        assert_eq!(mean(&[1.0, 2.0]), 1.5);
        assert_eq!(fmt_num(2.0), "2");
        assert_eq!(fmt_num(2.5), "2.5");
    }

    #[test]
    fn tables() {
        assert_eq!(render_tables(&[]), "no results\n");
        let mut other = row("B-1", "refs", "m2", true, 7, 1, 1.0);
        other.agent = "claude".into();
        let rows = vec![
            row("A-1", "concept", "m1", true, 10, 2, 1.0),
            row("A-1", "concept", "m1", false, 20, 3, 0.5),
            row("A-1", "concept", "m1", true, 30, 4, 0.0),
            row("A-1", "concept", "m1", true, 40, 5, 0.5),
            other,
        ];
        let t = render_tables(&rows);
        assert!(t.contains("| question | generic/m1 | claude/m2 |"));
        assert!(t.contains("| A-1 (concept, alpha) | 3/4 ok, 25 tok, 3.5 calls, recall 0.50 | - |"));
        assert!(t.contains("| B-1 (refs, alpha) | - | 1/1 ok, 7 tok, 1 calls, recall 1.00 |"));
        assert!(t.contains("| family | generic/m1 | claude/m2 |"));
        assert!(t.contains("| refs | - | 1/1 ok"));
        assert!(t.is_ascii());
    }

    #[test]
    fn results_file_parsing() {
        let r = row("A", "concept", "m", true, 1, 1, 1.0);
        let text = format!("{}\n\n{}\n", r.to_json_line(), r.to_json_line());
        assert_eq!(parse_results(&text).unwrap().len(), 2);
        assert!(parse_results("{").unwrap_err().contains("line 1"));
        assert!(parse_results("{}").unwrap_err().contains("line 1"));
    }

    // ---- CLI

    fn call(args: &[&str], exec: Exec) -> (i32, String, String) {
        let (mut o, mut e) = (Vec::new(), Vec::new());
        let code = cli(&s(args), &mut o, &mut e, exec);
        (code, String::from_utf8(o).unwrap(), String::from_utf8(e).unwrap())
    }

    fn fake(i: &Invocation) -> Result<Output, String> {
        match i.program.as_str() {
            "git" => ok(""),
            "sh" if i.args[0] == "-c" && i.args.len() > 2 => {
                std::fs::create_dir_all(&i.args[5]).unwrap();
                ok("")
            }
            _ => ok(r#"{"answer":"a.rs","input_tokens":1,"output_tokens":1}"#),
        }
    }

    #[test]
    fn cli_help_and_unknown() {
        for a in [&[][..], &["help"], &["--help"], &["-h"]] {
            let (c, o, _) = call(a, &fake);
            assert_eq!(c, 0);
            assert!(o.contains("usage:"));
        }
        let (c, _, e) = call(&["bogus"], &fake);
        assert_eq!(c, 2);
        assert!(e.contains("unknown command"));
    }

    #[test]
    fn cli_validate() {
        let dir = temp_dir();
        let good = dir.join("q.json");
        std::fs::write(&good, doc(|d| d)).unwrap();
        let g = good.to_str().unwrap();
        assert_eq!(call(&["validate", "--questions", g], &fake), (0, "ok: 20 questions\n".into(), String::new()));
        assert_eq!(call(&["validate", "--questions", g, "--repos-root", "/r"], &fake).0, 0);
        let nogit = |_: &Invocation| Err("x".to_string());
        let (c, _, e) = call(&["validate", "--questions", g, "--repos-root", "/r"], &nogit);
        assert_eq!(c, 1);
        assert!(e.contains("error: pin alpha"));
        let bad = dir.join("bad.json");
        std::fs::write(&bad, "{").unwrap();
        assert_eq!(call(&["validate", "--questions", bad.to_str().unwrap()], &fake).0, 1);
        assert_eq!(call(&["validate", "--questions", "/no/such/file"], &fake).0, 1);
        assert_eq!(call(&["validate"], &fake).0, 2);
        assert_eq!(call(&["validate", "--nope"], &fake).0, 2);
        assert_eq!(call(&["validate", "--questions"], &fake).0, 2);
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cli_run_end_to_end() {
        let dir = temp_dir();
        let qf = dir.join("q.json");
        std::fs::write(&qf, doc(|d| d)).unwrap();
        let out = dir.join("out");
        let (qs, os) = (qf.to_str().unwrap(), out.to_str().unwrap());
        let base = ["run", "--questions", qs, "--repos-root", "/r", "--agent", "generic", "--command", "x",
            "--model", "m", "--runs", "2", "--out", os];

        let mut dry = base.to_vec();
        dry.extend(["--dry-run", "--only", "concept-0,refs-1"]);
        let (c, o, _) = call(&dry, &fake);
        assert_eq!(c, 0);
        assert_eq!(o.lines().count(), 4);
        assert!(o.contains("plan: concept-0 run 2"));
        assert!(!out.exists());

        let mut real = base.to_vec();
        real.extend(["--only", "concept-0"]);
        let (c, o, e) = call(&real, &fake);
        assert_eq!(c, 0, "{e}");
        assert!(o.contains("| concept-0 (concept, alpha) | 2/2 ok"));
        assert!(out.join("snap/alpha").is_dir() && out.join("snap/beta").is_dir());
        let text = std::fs::read_to_string(out.join("results.jsonl")).unwrap();
        assert_eq!(text.lines().count(), 2);
        // Second run appends and reuses the snapshots.
        assert_eq!(call(&real, &fake).0, 0);
        assert_eq!(std::fs::read_to_string(out.join("results.jsonl")).unwrap().lines().count(), 4);
        // The table command reads the file back.
        let (c, o, _) = call(&["table", "--results", out.join("results.jsonl").to_str().unwrap()], &fake);
        assert_eq!(c, 0);
        assert!(o.contains("Per family") && o.contains("4/4 ok"));

        // Claude agent goes through the same path.
        let claude_fake = |i: &Invocation| if i.program == "claude" { ok(STREAM) } else { fake(i) };
        let out2 = dir.join("out2");
        let c = call(&["run", "--questions", qs, "--repos-root", "/r", "--agent", "claude", "--model", "m",
            "--runs", "1", "--out", out2.to_str().unwrap(), "--only", "refs-0"], &claude_fake);
        assert_eq!(c.0, 0);
        assert!(c.1.contains("claude/m"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cli_run_errors() {
        let dir = temp_dir();
        let qf = dir.join("q.json");
        std::fs::write(&qf, doc(|d| d)).unwrap();
        let (qs, os) = (qf.to_str().unwrap(), dir.join("o").to_str().unwrap().to_string());
        let ok_args = |extra: &[&str]| {
            let mut v = s(&["run", "--questions", qs, "--repos-root", "/r", "--model", "m", "--out", &os]);
            v.extend(s(extra));
            v
        };
        let run = |a: Vec<String>, e: Exec| {
            let (mut o, mut er) = (Vec::new(), Vec::new());
            (cli(&a, &mut o, &mut er, e), String::from_utf8(er).unwrap())
        };
        assert_eq!(run(ok_args(&["--runs", "1"]), &fake).0, 2); // no agent
        assert_eq!(run(ok_args(&["--runs", "1", "--agent", "other"]), &fake).0, 2);
        assert_eq!(run(ok_args(&["--runs", "1", "--agent", "generic"]), &fake).0, 2); // no command
        assert_eq!(run(ok_args(&["--runs", "0", "--agent", "claude"]), &fake).0, 2);
        assert_eq!(run(ok_args(&["--runs", "x", "--agent", "claude"]), &fake).0, 2);
        assert_eq!(run(ok_args(&["--agent", "claude"]), &fake).0, 2); // no runs
        assert_eq!(run(ok_args(&["--bogus"]), &fake).0, 2);
        let (c, e) = run(ok_args(&["--runs", "1", "--agent", "claude", "--only", "nope"]), &fake);
        assert_eq!(c, 1);
        assert!(e.contains("unknown question 'nope'"));
        // Validation failure (pin missing) stops the run.
        let nogit = |_: &Invocation| Err("x".to_string());
        assert_eq!(run(ok_args(&["--runs", "1", "--agent", "claude"]), &nogit).0, 1);
        // Snapshot failure.
        let badsnap = |i: &Invocation| {
            if i.program == "sh" {
                Ok(Output { code: 1, stdout: String::new(), stderr: "tar".into() })
            } else {
                ok("")
            }
        };
        let (c, e) = run(ok_args(&["--runs", "1", "--agent", "claude"]), &badsnap);
        assert_eq!(c, 1);
        assert!(e.contains("snapshot of"));
        // Unwritable out dir: a file where the directory should be.
        let blocker = dir.join("blocker");
        std::fs::write(&blocker, "").unwrap();
        let mut a = ok_args(&["--runs", "1", "--agent", "claude"]);
        let at = a.iter().position(|x| x == &os).unwrap();
        a[at] = blocker.to_str().unwrap().to_string();
        let (c, e) = run(a, &fake);
        assert_eq!(c, 1);
        assert!(e.contains("cannot create"));
        // results.jsonl cannot be opened: it is a directory.
        let out3 = dir.join("o3");
        std::fs::create_dir_all(out3.join("snap/alpha")).unwrap();
        std::fs::create_dir_all(out3.join("snap/beta")).unwrap();
        std::fs::write(out3.join("snap/alpha.sha"), SHA_A).unwrap();
        std::fs::write(out3.join("snap/beta.sha"), SHA_B).unwrap();
        std::fs::create_dir_all(out3.join("results.jsonl")).unwrap();
        let mut a = ok_args(&["--runs", "1", "--agent", "claude"]);
        let at = a.iter().position(|x| x == &os).unwrap();
        a[at] = out3.to_str().unwrap().to_string();
        let (c, e) = run(a, &fake);
        assert_eq!(c, 1);
        assert!(e.contains("cannot open"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn cli_table_errors() {
        assert_eq!(call(&["table"], &fake).0, 2);
        assert_eq!(call(&["table", "--x"], &fake).0, 2);
        let (c, _, e) = call(&["table", "--results", "/no/such"], &fake);
        assert_eq!(c, 1);
        assert!(e.contains("cannot read"));
        let dir = temp_dir();
        let f = dir.join("r.jsonl");
        std::fs::write(&f, "{").unwrap();
        assert_eq!(call(&["table", "--results", f.to_str().unwrap()], &fake).0, 1);
        std::fs::write(&f, "").unwrap();
        let (c, o, _) = call(&["table", "--results", f.to_str().unwrap()], &fake);
        assert_eq!((c, o.as_str()), (0, "no results\n"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn snapshot_skips_existing_and_reports_spawn_errors() {
        let dir = temp_dir();
        let ok = |_: &Invocation| -> Result<Output, String> {
            Ok(Output { code: 0, stdout: String::new(), stderr: String::new() })
        };
        let dest = dir.join("snap-repo");
        assert!(snapshot(Path::new("/r"), SHA_A, &dest, &ok).is_ok());
        assert_eq!(std::fs::read_to_string(dir.join("snap-repo.sha")).unwrap(), SHA_A);
        let boom = |_: &Invocation| -> Result<Output, String> { panic!("must not run") };
        assert!(snapshot(Path::new("/r"), SHA_A, &dest, &boom).is_ok());
        let other = snapshot(Path::new("/r"), SHA_B, &dest, &boom).unwrap_err();
        assert!(other.contains("use a new --out"));
        std::fs::create_dir_all(dir.join("unmarked")).unwrap();
        let unmarked = snapshot(Path::new("/r"), SHA_A, &dir.join("unmarked"), &boom).unwrap_err();
        assert!(unmarked.contains("no commit marker"));
        let fail = |_: &Invocation| Err("no sh".to_string());
        assert_eq!(snapshot(Path::new("/r"), SHA_A, &dir.join("new"), &fail), Err("no sh".to_string()));
        let bad = |_: &Invocation| Ok(Output { code: 1, stdout: String::new(), stderr: "boom".into() });
        assert!(snapshot(Path::new("/r"), SHA_A, &dir.join("bad"), &bad).unwrap_err().contains("boom"));
        std::fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn real_exec_runs_processes() {
        let inv = Invocation {
            program: "sh".into(),
            args: s(&["-c", "echo $CNE_X; echo err >&2; pwd; exit 3"]),
            env: vec![("CNE_X".into(), "hi".into())],
            cwd: Some(std::env::temp_dir()),
        };
        let o = real_exec(&inv).unwrap();
        assert_eq!(o.code, 3);
        assert!(o.stdout.starts_with("hi\n") && o.stderr == "err\n");
        let nocwd = Invocation { cwd: None, args: s(&["-c", "kill -9 $$"]), ..inv.clone() };
        assert_eq!(real_exec(&nocwd).unwrap().code, -1);
        let missing = Invocation { program: "/no/such/prog".into(), ..inv };
        assert!(real_exec(&missing).unwrap_err().contains("cannot run"));
    }

    #[test]
    fn run_entry_prints_help() {
        assert_eq!(run(vec![OsString::from("--help")]), 0);
    }
}
