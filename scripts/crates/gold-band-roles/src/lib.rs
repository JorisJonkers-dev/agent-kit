//! The Gold Band profile generator: the critique pipeline's roles rendered
//! as Gold Band profiles, and the convergence that keeps the profiles
//! directory there (#345).
//!
//! Gold Band calls a critic a **profile**: a Markdown file with YAML front
//! matter it reads from `~/.gold-band/presets/profiles/<profile name>.md`.
//! This crate writes the six profiles the critique roles describe — the
//! decision-gap critic, the spec red team, the ticket linter, the diff
//! reviewer, the feature reviewer and the adversarial verifier (the
//! pipeline's refuter) — with the exact front matter Gold Band parses (`id`,
//! `name`, `summary`, `createdAt`, `updatedAt`, `dynamicTemplate: false`)
//! and a body matching its sources rule for rule: the role file's text
//! verbatim, then the shared preamble and output contract verbatim, then
//! the finding schema the contract points at, then the lens texts the role
//! uses. Gold Band's own built-in profiles are reused, never generated and
//! never touched.
//!
//! The role text is what the critique runner sends a critic, but the runner
//! also enforces what surrounds it — read-only proof, packet confinement,
//! the quote check, schema enforcement, the refute round, the family
//! allowlist and the nedap refusal, plan-limit routing — and none of that exists on this path. Each
//! profile's closing section states those rules and says the role cannot
//! enforce them, and `docs/gold-band-roles.md` maps each role to the model
//! families that may run it: families the model gateway's catalogue
//! offers, never the openai (Codex) family, never the author's family.
//!
//! Usage:
//!
//! ```text
//! gold-band-roles render <CRITIQUE_DIR> <OUT_DIR>
//! gold-band-roles converge <CRITIQUE_DIR> <PROFILES_DIR> --keep <FILE> --retired <FILE> [--check]
//! ```
//!
//! `converge` renders the six profiles into PROFILES_DIR, removes a file
//! whose name the retired file lists and a generated one (front matter `id`
//! starting `pf-`, not a Gold Band built-in `pf-builtin-*`) that is no longer
//! one of the six, and never touches a file without such an id (a hand-made
//! role) or a built-in; the keep file's `keep:` list protects a file from
//! that removal. With `--check` it writes nothing, and it prints `unchanged`
//! when nothing changed. Exit codes: 0 success, 1 a file failed (or a
//! checked run with work pending), 2 bad usage. Success prints to stdout,
//! problems to stderr.

use std::ffi::{OsStr, OsString};
use std::fs;
use std::path::Path;

/// The crate's name, which prefixes every problem line.
pub const NAME: &str = env!("CARGO_PKG_NAME");

/// The fixed moment every generated profile is stamped with: a second run
/// of unchanged sources writes nothing.
pub const STAMP: &str = "2025-01-01T00:00:00Z";

/// The prefix of every id this crate writes, and of the Gold Band built-ins
/// the convergence never touches.
pub const PREFIX: &str = "pf-";

/// One generated profile: the file Gold Band reads it by, the front matter
/// it carries, and the critique files its body is assembled from verbatim.
pub struct Role {
    /// The profile name — Gold Band reads the file as `<file>.md`.
    pub file: &'static str,
    /// The front matter id, `pf-…`.
    pub id: &'static str,
    /// The front matter name.
    pub name: &'static str,
    /// One line of front matter summary.
    pub summary: &'static str,
    /// The critique role file, under `roles/`, the body opens with.
    pub source: &'static str,
    /// The lens files, under `lenses/`, the role uses, in routing's order.
    pub lenses: &'static [&'static str],
}

/// The six roles, in the order the critique pipeline's stages run them
/// (`skills/workstation/critique/routing.json` names the role and lenses each
/// stage's critics run; the refuter, the adversarial verifier, runs with no
/// lens).
pub const ROLES: [Role; 6] = [
    Role {
        file: "decision-gap-critic",
        id: "pf-gap",
        name: "Decision-gap critic",
        summary:
            "Asks the owner the questions the decisions a feature was planned from never settled.",
        source: "gap-critic.md",
        lenses: &["unasked-questions", "failure-modes", "integration"],
    },
    Role {
        file: "spec-red-team",
        id: "pf-spec-redteam",
        name: "Spec red team",
        summary:
            "Attacks a spec for contradictions, omissions, inventions and untestable requirements.",
        source: "spec-redteam.md",
        lenses: &[
            "fidelity",
            "testability",
            "failure-modes",
            "security",
            "integration",
            "operability",
            "simplicity",
        ],
    },
    Role {
        file: "ticket-linter",
        id: "pf-ticket-linter",
        name: "Ticket linter",
        summary: "Lints the structure of the tickets cut from a spec.",
        source: "ticket-linter.md",
        lenses: &["testability", "simplicity"],
    },
    Role {
        file: "diff-reviewer",
        id: "pf-diff-reviewer",
        name: "Diff reviewer",
        summary: "Second review of one ticket's diff for bugs, unmet criteria and security issues.",
        source: "diff-reviewer.md",
        lenses: &["failure-modes", "security", "integration"],
    },
    Role {
        file: "feature-reviewer",
        id: "pf-feature-reviewer",
        name: "Feature reviewer",
        summary: "Reviews a whole feature for the defects that live between its tickets.",
        source: "feature-reviewer.md",
        lenses: &["integration", "failure-modes", "operability", "simplicity"],
    },
    Role {
        file: "adversarial-verifier",
        id: "pf-adversarial-verifier",
        name: "Adversarial verifier",
        summary: "Refutes or downgrades one accepted critical finding against the files.",
        source: "refuter.md",
        lenses: &[],
    },
];

/// The closing section of every generated profile: the critique runner's
/// guarantees a Gold Band path loses. The profile states them because it
/// cannot enforce them — the acceptance is that the role text states the
/// rules the runner enforces and says the role cannot enforce them.
const CLOSING: &str = "## What this role cannot enforce\n\n\
This text comes from the critique pipeline, where the runner — not the role\n\
text — enforces the rules around it. Running the role as a Gold Band profile\n\
loses that machinery, so this section states each guarantee the role relies\n\
on without being able to enforce it: an agent taking the role is bound by\n\
its workflow, not by this text.\n\n\
- **Read-only proof.** The runner proves every critic ran read-only and\n\
  catches one that wrote. A Gold Band agent reading this profile is bound\n\
  only by the reach its workflow gives it.\n\
- **Packet confinement.** The reviews above read a frozen packet the runner\n\
  picked, and every file, citation and search stays inside it. An agent on\n\
  this path reads whatever its reach allows, which may reach past the\n\
  artifact and its reference files.\n\
- **The quote check.** In the pipeline a quote that appears in no listed\n\
  file discards its finding. Nothing here checks a quote, so check each one\n\
  against the files before acting on a finding.\n\
- **Schema enforcement.** The runner validates every finding a critic\n\
  returns against the finding schema above and discards one that fails\n\
  it. Nothing here checks a finding's shape, so validate each one\n\
  against that schema before acting on it.\n\
- **The refute round.** An accepted critical finding went to one more critic\n\
  of a family that neither raised it nor is the orchestrator's. On this path\n\
  only a workflow that runs the adversarial-verifier profile gets that, and\n\
  nothing runs it on its own.\n\
- **The family allowlist and the nedap refusal.** The runner consults the\n\
  machine's family allowlist entry recorded for the repository's remote:\n\
  a repository with no recorded entry reviews under the personal default\n\
  — every critic family, code but no history — a declined entry or a\n\
  `nedap` remote sends nothing, and a repository with no remote is\n\
  refused. Which family runs this profile is decided by the Gold Band\n\
  workflow alone, and the family allowlist is not enforced on this path.\n\
- **Plan-limit routing.** The runner reads the model gateway's plan limits\n\
  before it starts a critic and routes or drops the critic within them. A\n\
  Gold Band run spends on its own plan; nothing here checks a window.\n";

/// The front matter Gold Band parses, written exactly as its parser reads
/// it: `id` first, then `name`, `summary`, the fixed stamps and the
/// templating switch off.
fn front_matter(role: &Role) -> String {
    format!(
        "---\nid: {}\nname: {}\nsummary: {}\ncreatedAt: {}\nupdatedAt: {}\ndynamicTemplate: false\n---\n",
        role.id, role.name, role.summary, STAMP, STAMP
    )
}

/// Appends one verbatim section, ending it and adding the blank line that
/// separates it from the next.
fn section(text: &mut String, body: &str) {
    text.push_str(body);
    if !text.ends_with('\n') {
        text.push('\n');
    }
    text.push('\n');
}

/// Reads one source file of the critique pipeline, relative to its directory,
/// naming the file and the error when it cannot be read.
fn read(critique: &Path, file: &str) -> Result<String, String> {
    let path = critique.join(file);
    fs::read_to_string(&path).map_err(|error| format!("{}: {}", path.display(), error))
}

/// The full text of one profile: the front matter, then the body — the role
/// file, the shared preamble, the output contract, the finding schema it
/// points at, the role's lenses — then the closing section. Every source
/// line appears verbatim, in order.
pub fn render(critique: &Path, role: &Role) -> Result<String, String> {
    let mut text = front_matter(role);
    text.push('\n');
    section(
        &mut text,
        &read(critique, &format!("roles/{}", role.source))?,
    );
    section(&mut text, &read(critique, "prompts/preamble.md")?);
    section(&mut text, &read(critique, "prompts/output-contract.md")?);
    let schema = read(critique, "schemas/finding.schema.json")?;
    section(&mut text, &format!("```json\n{}\n```", schema.trim_end()));
    for lens in role.lenses {
        section(&mut text, &read(critique, &format!("lenses/{lens}.md"))?);
    }
    section(&mut text, CLOSING);
    Ok(text)
}

/// Renders the six profiles into `out`, each written atomically and only
/// when its content changed; returns the profile names it wrote.
pub fn render_dir(critique: &Path, out: &Path) -> Result<Vec<String>, String> {
    let rendered: Vec<String> = ROLES
        .iter()
        .map(|role| render(critique, role))
        .collect::<Result<_, _>>()?;
    fs::create_dir_all(out).map_err(|error| format!("{}: {}", out.display(), error))?;
    let mut written = Vec::new();
    for (role, body) in ROLES.iter().zip(&rendered) {
        if write_if_changed(&out.join(format!("{}.md", role.file)), body)? {
            written.push(role.file.to_string());
        }
    }
    Ok(written)
}

/// Writes `text` to `path` through a sibling temporary file renamed over
/// it, and returns whether the file changed. A file that already holds
/// exactly this text is left alone.
fn write_if_changed(path: &Path, text: &str) -> Result<bool, String> {
    if fs::read_to_string(path).is_ok_and(|current| current == text) {
        return Ok(false);
    }
    let temporary = path.with_file_name(format!(
        ".{}.tmp",
        path.file_name()
            .map(|name| name.to_string_lossy())
            .unwrap_or_default()
    ));
    let shown = temporary.display();
    fs::write(&temporary, text).map_err(|error| format!("{shown}: {error}"))?;
    fs::rename(&temporary, path).map_err(|error| format!("{shown}: {error}"))?;
    Ok(true)
}

/// Whether `path` differs from `text`: what converge would write with
/// `check` on, which writes nothing.
fn out_of_date(path: &Path, text: &str) -> bool {
    match fs::read_to_string(path) {
        Ok(current) => current != text,
        Err(_) => true,
    }
}

/// Removes one converged-away profile, naming the file and the error when it
/// cannot be removed.
fn remove_file(path: &Path) -> Result<(), String> {
    fs::remove_file(path).map_err(|error| format!("{}: {}", path.display(), error))
}

/// The front matter `id` of a profile file: the `id:` value between the
/// opening `---` lines. `None` when the file has none — Gold Band falls back
/// to the file name there, and convergence leaves such a file (a hand-made
/// role) alone.
pub fn front_matter_id(text: &str) -> Option<String> {
    let rest = text.strip_prefix("---")?;
    let mut id = None;
    for line in rest.lines() {
        let line = line.trim();
        if line == "---" {
            return id;
        }
        if id.is_none() {
            id = line
                .strip_prefix("id:")
                .map(|value| value.trim().to_string());
        }
    }
    None
}

/// The profile file names in `dir`, sorted; a directory that does not exist
/// yet holds none.
fn profile_files(dir: &Path) -> Result<Vec<String>, String> {
    let entries = match fs::read_dir(dir) {
        Ok(entries) => entries,
        Err(_) if !dir.exists() => return Ok(Vec::new()),
        Err(error) => return Err(format!("{}: {}", dir.display(), error)),
    };
    let mut names: Vec<String> = entries
        .flatten()
        .filter(|entry| entry.file_type().is_ok_and(|kind| kind.is_file()))
        .map(|entry| entry.file_name().to_string_lossy().to_string())
        .filter(|name| name.ends_with(".md"))
        .collect();
    names.sort();
    Ok(names)
}

/// Whether convergence may remove this profile file: a retired name, or a
/// generated one (front matter id `pf-…`, not a Gold Band built-in
/// `pf-builtin-*`) that is no longer one of the six — unless the keep list
/// names it, or it is one of the six file names the render refreshes. A file
/// with no front matter id, or a built-in, is never removed.
fn removal(
    name: &str,
    id: Option<&str>,
    names: &[String],
    keep: &[String],
    retired: &[String],
) -> bool {
    let Some(id) = id else {
        return false;
    };
    if names.iter().any(|current| current == name) {
        return false;
    }
    if !id.starts_with(PREFIX) || id.starts_with("pf-builtin-") {
        return false;
    }
    retired.iter().any(|gone| gone == name) || !keep.iter().any(|kept| kept == name)
}

/// What one converge run did — or, with `check`, would do.
#[derive(Debug, PartialEq, Eq)]
pub struct Outcome {
    /// The profile files removed, in listing order.
    pub removed: Vec<String>,
    /// The profile names written, in the roles' order.
    pub written: Vec<String>,
}

impl Outcome {
    /// True when the directory already held exactly the six.
    pub fn is_unchanged(&self) -> bool {
        self.removed.is_empty() && self.written.is_empty()
    }
}

/// Converges the profiles directory to the six generated profiles: renders
/// them, removes a retired file and a generated file that is no longer one
/// of them, and leaves every hand-made file and Gold Band built-in alone
/// (`keep` protects a file from that removal). With `check`, nothing is
/// written, removed or created; the outcome then reports pending work.
pub fn converge(
    critique: &Path,
    profiles: &Path,
    keep: &[String],
    retired: &[String],
    check: bool,
) -> Result<Outcome, String> {
    let rendered: Vec<String> = ROLES
        .iter()
        .map(|role| render(critique, role))
        .collect::<Result<_, _>>()?;
    let names: Vec<String> = ROLES
        .iter()
        .map(|role| format!("{}.md", role.file))
        .collect();
    let mut outcome = Outcome {
        removed: Vec::new(),
        written: Vec::new(),
    };
    for name in profile_files(profiles)? {
        let id = fs::read_to_string(profiles.join(&name))
            .ok()
            .and_then(|text| front_matter_id(&text));
        if removal(&name, id.as_deref(), &names, keep, retired) {
            outcome.removed.push(name);
        }
    }
    if !check {
        for name in &outcome.removed {
            remove_file(&profiles.join(name))?;
        }
        fs::create_dir_all(profiles)
            .map_err(|error| format!("{}: {}", profiles.display(), error))?;
    }
    for (role, body) in ROLES.iter().zip(&rendered) {
        let path = profiles.join(format!("{}.md", role.file));
        let wrote = if check {
            out_of_date(&path, body)
        } else {
            write_if_changed(&path, body)?
        };
        if wrote {
            outcome.written.push(role.file.to_string());
        }
    }
    Ok(outcome)
}

/// The names a keep or retired file lists: one top-level `keep:` or
/// `retired:` list, each entry one `- <name>` line, the shape converge.py's
/// own keep file holds. A file without the list names nothing; a key that
/// holds anything else, or a line under it that is not a plain entry, is
/// refused — nothing is removed on a file the owner cannot have meant.
pub fn name_list(text: &str, key: &str) -> Result<Vec<String>, String> {
    let opening = format!("{key}:");
    let mut names: Vec<String> = Vec::new();
    let mut under = false;
    for line in text.lines() {
        let indented = line.starts_with([' ', '\t']);
        let entry = line.trim();
        if !indented {
            if let Some(rest) = entry.strip_prefix(&opening) {
                if !rest.trim().is_empty() {
                    return Err(format!("`{key}` must be a list of names, not a value"));
                }
                under = true;
            } else if !entry.is_empty() {
                under = false;
            }
        } else if under {
            if entry.is_empty() || entry == "-" {
                continue;
            }
            let Some(name) = entry.strip_prefix("- ") else {
                return Err(format!("`{key}` must be a list of names"));
            };
            let name = name.trim().trim_matches('"');
            if !name.is_empty() {
                names.push(name.to_string());
            }
        }
    }
    Ok(names)
}

/// One keep or retired file's list: a missing file lists nothing; present,
/// it is read and its `key:` list parsed.
fn name_file(path: &OsStr, key: &str) -> Result<Vec<String>, String> {
    let path = Path::new(path);
    if !path.exists() {
        return Ok(Vec::new());
    }
    let text =
        fs::read_to_string(path).map_err(|error| format!("{}: {}", path.display(), error))?;
    name_list(&text, key)
}

/// The usage lines every bad command line prints.
fn usage() -> String {
    format!(
        "usage: {NAME} render <CRITIQUE_DIR> <OUT_DIR>\n       {NAME} converge <CRITIQUE_DIR> <PROFILES_DIR> --keep <FILE> --retired <FILE> [--check]"
    )
}

/// Runs the `render` command line and returns its exit code.
fn render_run(rest: &[OsString]) -> i32 {
    match rest {
        [critique, out] => match render_dir(Path::new(critique), Path::new(out)) {
            Ok(written) => {
                println!(
                    "{NAME}: rendered {} profiles to {} (wrote {})",
                    ROLES.len(),
                    out.to_string_lossy(),
                    written.len()
                );
                0
            }
            Err(problem) => {
                eprintln!("{NAME}: {problem}");
                1
            }
        },
        _ => {
            eprintln!("{}", usage());
            2
        }
    }
}

/// The `--keep`, `--retired` and `--check` flags of a converge command line:
/// the keep and retired files and the check switch, or `None` when the flags
/// do not match the usage.
fn converge_flags(flags: &[OsString]) -> Option<(OsString, OsString, bool)> {
    let mut keep: Option<OsString> = None;
    let mut retired: Option<OsString> = None;
    let mut check = false;
    let mut index = 0;
    while index < flags.len() {
        let flag = flags[index].as_os_str();
        if flag == OsStr::new("--check") {
            check = true;
            index += 1;
        } else {
            let value = flags.get(index + 1)?;
            // A flag missing its value ends the line or runs into the next
            // flag: either way it never swallows that flag as its value.
            if value.as_os_str().to_string_lossy().starts_with("--") {
                return None;
            }
            if flag == OsStr::new("--keep") {
                keep = Some(value.clone());
            } else if flag == OsStr::new("--retired") {
                retired = Some(value.clone());
            } else {
                return None;
            }
            index += 2;
        }
    }
    Some((keep?, retired?, check))
}

/// Prints one converge outcome and returns its exit code: `unchanged` when
/// there was nothing to do, each change as its own line — and a checked run
/// says what it would do and fails the check instead.
fn report(outcome: &Outcome, check: bool) -> i32 {
    if outcome.is_unchanged() {
        println!("unchanged");
        return 0;
    }
    let (removes, writes) = if check {
        ("would remove", "would write")
    } else {
        ("removed", "wrote")
    };
    for name in &outcome.removed {
        println!("{removes} {name}");
    }
    for name in &outcome.written {
        println!("{writes} {name}");
    }
    if check {
        eprintln!("{NAME}: the profiles directory is not converged");
        1
    } else {
        0
    }
}

/// Runs the `converge` command line and returns its exit code.
fn converge_run(rest: &[OsString]) -> i32 {
    let (critique, profiles, flags) = match rest {
        [critique, profiles, flags @ ..] => (critique, profiles, flags),
        _ => {
            eprintln!("{}", usage());
            return 2;
        }
    };
    let Some((keep_file, retired_file, check)) = converge_flags(flags) else {
        eprintln!("{}", usage());
        return 2;
    };
    let keep = match name_file(&keep_file, "keep") {
        Ok(list) => list,
        Err(problem) => {
            eprintln!("{NAME}: {problem}");
            return 1;
        }
    };
    let retired = match name_file(&retired_file, "retired") {
        Ok(list) => list,
        Err(problem) => {
            eprintln!("{NAME}: {problem}");
            return 1;
        }
    };
    match converge(
        Path::new(critique),
        Path::new(profiles),
        &keep,
        &retired,
        check,
    ) {
        Ok(outcome) => report(&outcome, check),
        Err(problem) => {
            eprintln!("{NAME}: {problem}");
            1
        }
    }
}

/// Runs one command line and returns its exit code: 0 success (`unchanged`
/// printed when converge changed nothing), 1 a file failed or a checked
/// converge with work pending, 2 bad usage. Success prints to stdout,
/// problems to stderr.
pub fn run<I>(args: I) -> i32
where
    I: IntoIterator<Item = OsString>,
{
    let rest: Vec<OsString> = args.into_iter().collect();
    match rest.split_first() {
        None => {
            eprintln!("{}", usage());
            2
        }
        Some((command, rest)) => {
            if command == OsStr::new("render") {
                render_run(rest)
            } else if command == OsStr::new("converge") {
                converge_run(rest)
            } else {
                eprintln!("{}", usage());
                2
            }
        }
    }
}

// The unit tests below execute every line of this file, which is the
// workspace's 100 percent line-coverage standard (docs/RUST.md); the single
// executable line in src/main.rs, the exit wrapper, is the one they cannot.
#[cfg(test)]
mod tests {
    use super::*;
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicUsize, Ordering};

    /// The repository root, three levels above this crate's manifest.
    fn kit_root() -> PathBuf {
        Path::new(env!("CARGO_MANIFEST_DIR"))
            .ancestors()
            .nth(3)
            .expect("the crate sits at scripts/crates/gold-band-roles")
            .to_path_buf()
    }

    /// The critique pipeline's own directory, the source of every profile.
    fn critique_dir() -> PathBuf {
        kit_root()
            .join("skills")
            .join("workstation")
            .join("critique")
    }

    /// A unique empty scratch directory under the temp directory: every call
    /// returns its own, so two calls with one tag never share a path.
    fn scratch(tag: &str) -> PathBuf {
        static NEXT: AtomicUsize = AtomicUsize::new(0);
        let next = NEXT.fetch_add(1, Ordering::Relaxed);
        let dir = std::env::temp_dir().join(format!(
            "gold-band-roles-{tag}-{}-{next}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&dir);
        fs::create_dir_all(&dir).expect("scratch directory");
        dir
    }

    /// Writes `text` to a unique file under the temp directory, for the keep
    /// and retired lists.
    fn list_file(tag: &str, text: &str) -> PathBuf {
        let path =
            std::env::temp_dir().join(format!("gold-band-roles-{tag}-{}.list", std::process::id()));
        fs::write(&path, text).expect("list file");
        path
    }

    /// One source file of one role's body, in body order: the role file, the
    /// shared preamble and output contract, the finding schema, the lenses.
    fn sources(role: &Role) -> Vec<PathBuf> {
        let critique = critique_dir();
        let mut files = vec![
            critique.join("roles").join(role.source),
            critique.join("prompts").join("preamble.md"),
            critique.join("prompts").join("output-contract.md"),
            critique.join("schemas").join("finding.schema.json"),
        ];
        files.extend(
            role.lenses
                .iter()
                .map(|lens| critique.join("lenses").join(format!("{lens}.md"))),
        );
        files
    }

    /// True when every non-empty line of `source` appears in `text` as a
    /// whole line, in order.
    fn subsumes(text: &str, source: &str) -> bool {
        let mut rest = text.lines();
        for line in source.lines() {
            if line.trim().is_empty() {
                continue;
            }
            if !rest.any(|candidate| candidate == line) {
                return false;
            }
        }
        true
    }

    /// A minimal critique directory that renders every role, with `except`
    /// naming the one lens it omits (or `None` for a complete one).
    fn fixture_critique(tag: &str, except: Option<&str>) -> PathBuf {
        let dir = scratch(tag);
        for role in ROLES {
            let path = dir.join("roles").join(role.source);
            fs::create_dir_all(path.parent().expect("roles dir")).expect("roles dir");
            fs::write(
                path,
                format!(
                    "# Role: {name}\n\n- `fixture-check`: a line.\n",
                    name = role.name
                ),
            )
            .expect("role file");
        }
        fs::create_dir_all(dir.join("prompts")).expect("prompts dir");
        fs::write(
            dir.join("prompts").join("preamble.md"),
            "# Preamble\n\nRead everything.\n",
        )
        .expect("preamble");
        fs::write(
            dir.join("prompts").join("output-contract.md"),
            "# Output contract\n\nEnd with json.\n\nThe schema:\n",
        )
        .expect("output contract");
        fs::create_dir_all(dir.join("schemas")).expect("schemas dir");
        fs::write(
            dir.join("schemas").join("finding.schema.json"),
            "{\"findings\": []}\n",
        )
        .expect("schema");
        fs::create_dir_all(dir.join("lenses")).expect("lenses dir");
        for lens in ROLES
            .iter()
            .flat_map(|role| role.lenses.iter().copied())
            .chain(["fidelity"])
        {
            if Some(lens) == except {
                continue;
            }
            fs::write(
                dir.join("lenses").join(format!("{lens}.md")),
                format!("# Lens: {lens}\n\n- `lens-check`: a line.\n"),
            )
            .expect("lens file");
        }
        dir
    }

    /// Seeds one profile file into `dir`: with `id`, the front matter a
    /// generated or built-in one carries; without, a hand-made role.
    fn seed_profile(dir: &Path, name: &str, id: Option<&str>) {
        let text = match id {
            Some(id) => format!("---\nid: {id}\nname: {name}\nsummary: seed\n---\n\n# Seed\n"),
            None => format!("# {name}\n\nA hand-made role with no front matter id.\n"),
        };
        fs::write(dir.join(name), text).expect("seed profile");
    }

    #[test]
    fn every_role_carries_its_source_files_line_by_line_in_order() {
        for role in ROLES {
            let text = render(&critique_dir(), &role).expect("rendered");
            for path in sources(&role) {
                let body = fs::read_to_string(&path).expect("source file");
                assert!(
                    subsumes(&text, &body),
                    "{} drops lines of {}",
                    role.file,
                    path.display()
                );
            }
            assert!(text.contains("## What this role cannot enforce"));
        }
    }

    #[test]
    fn the_front_matter_is_literal() {
        assert_eq!(STAMP, "2025-01-01T00:00:00Z");
        for (role, want) in ROLES.iter().zip([
            "---\nid: pf-gap\nname: Decision-gap critic\nsummary: Asks the owner the questions the decisions a feature was planned from never settled.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
            "---\nid: pf-spec-redteam\nname: Spec red team\nsummary: Attacks a spec for contradictions, omissions, inventions and untestable requirements.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
            "---\nid: pf-ticket-linter\nname: Ticket linter\nsummary: Lints the structure of the tickets cut from a spec.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
            "---\nid: pf-diff-reviewer\nname: Diff reviewer\nsummary: Second review of one ticket's diff for bugs, unmet criteria and security issues.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
            "---\nid: pf-feature-reviewer\nname: Feature reviewer\nsummary: Reviews a whole feature for the defects that live between its tickets.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
            "---\nid: pf-adversarial-verifier\nname: Adversarial verifier\nsummary: Refutes or downgrades one accepted critical finding against the files.\ncreatedAt: 2025-01-01T00:00:00Z\nupdatedAt: 2025-01-01T00:00:00Z\ndynamicTemplate: false\n---\n",
        ]) {
            let text = render(&critique_dir(), role).expect("rendered");
            assert!(text.starts_with(want), "{}: the front matter is off", role.file);
            assert!(
                text[want.len()..].starts_with('\n'),
                "{}: the body opens with a blank line after the front matter",
                role.file
            );
        }
    }

    #[test]
    fn templating_is_off() {
        for role in ROLES {
            let text = render(&fixture_critique("templating", None), &role).expect("rendered");
            assert!(
                text.contains("dynamicTemplate: false"),
                "{} is templated",
                role.file
            );
        }
    }

    #[test]
    fn a_second_render_writes_nothing() {
        let critique = fixture_critique("idempotent", None);
        let out = scratch("idempotent-out");
        let first = render_dir(&critique, &out).expect("first render");
        assert_eq!(first.len(), ROLES.len());
        let second = render_dir(&critique, &out).expect("second render");
        assert!(second.is_empty(), "a second run changes nothing");
    }

    #[test]
    fn a_missing_source_is_refused_before_anything_is_written() {
        let critique = scratch("empty-critique");
        let out = std::env::temp_dir().join(format!(
            "gold-band-roles-empty-critique-out-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&out);
        assert_eq!(
            run(vec![
                "render".into(),
                critique.clone().into_os_string(),
                out.clone().into_os_string(),
            ]),
            1
        );
        assert!(!out.exists(), "nothing is written for a missing source");
    }

    #[test]
    fn an_out_path_that_is_a_file_is_refused() {
        assert_eq!(
            run(vec![
                "render".into(),
                fixture_critique("render-out-file", None).into_os_string(),
                list_file("render-out-file", "not a directory").into_os_string(),
            ]),
            1
        );
    }

    #[test]
    fn a_missing_lens_is_named_in_the_refusal() {
        let critique = fixture_critique("lens-missing", Some("fidelity"));
        let problem = render(&critique, &ROLES[1]).expect_err("fidelity is missing");
        assert!(
            problem.contains("lenses"),
            "the refusal names the lens: {problem}"
        );
        assert!(
            problem.contains("fidelity.md"),
            "the refusal names the lens: {problem}"
        );
    }

    #[test]
    fn a_retired_role_disappears_and_a_kept_hand_made_one_survives() {
        let profiles = scratch("converge");
        seed_profile(&profiles, "made-by-hand.md", None);
        seed_profile(
            &profiles,
            "built-in-favourite.md",
            Some("pf-builtin-favourite"),
        );
        seed_profile(&profiles, "old-critic.md", Some("pf-old"));
        seed_profile(
            &profiles,
            "stale-verifier.md",
            Some("pf-adversarial-verifier-2"),
        );
        seed_profile(&profiles, "kept-old-role.md", Some("pf-old-kept"));
        seed_profile(&profiles, "gap-critic-old-name.md", Some("pf-gap"));
        let keep = ["kept-old-role.md".to_string()];
        let retired = ["old-critic.md".to_string()];
        let critique = fixture_critique("converge", None);
        let outcome = converge(&critique, &profiles, &keep, &retired, false).expect("converged");
        assert_eq!(
            outcome.removed,
            [
                "gap-critic-old-name.md".to_string(),
                "old-critic.md".to_string(),
                "stale-verifier.md".to_string(),
            ]
        );
        assert_eq!(outcome.written.len(), ROLES.len());
        for kept in [
            "made-by-hand.md",
            "built-in-favourite.md",
            "kept-old-role.md",
        ] {
            assert!(profiles.join(kept).exists(), "{kept} is gone");
        }
        for gone in [
            "old-critic.md",
            "stale-verifier.md",
            "gap-critic-old-name.md",
        ] {
            assert!(!profiles.join(gone).exists(), "{gone} is still there");
        }
        for role in ROLES {
            assert!(
                profiles.join(format!("{}.md", role.file)).exists(),
                "{} is missing",
                role.file
            );
        }
        let again = converge(&critique, &profiles, &keep, &retired, false).expect("again");
        assert!(again.is_unchanged(), "the second run changed nothing");
    }

    #[test]
    fn a_gold_band_builtin_and_a_hand_made_role_are_untouched_by_the_retired_list() {
        let profiles = scratch("untouched");
        seed_profile(&profiles, "made-by-hand.md", None);
        seed_profile(
            &profiles,
            "built-in-favourite.md",
            Some("pf-builtin-favourite"),
        );
        fs::create_dir_all(profiles.join("not-a-profile.md")).expect("a directory named like one");
        fs::write(profiles.join("notes.txt"), "not a profile at all\n")
            .expect("a non-profile file");
        let retired = [
            "made-by-hand.md".to_string(),
            "built-in-favourite.md".to_string(),
        ];
        let outcome = converge(
            &fixture_critique("untouched", None),
            &profiles,
            &[],
            &retired,
            false,
        )
        .expect("converged");
        assert!(
            outcome.removed.is_empty(),
            "nothing without a generated id is removed"
        );
        assert!(
            profiles.join("not-a-profile.md").is_dir(),
            "a directory is not a profile"
        );
        assert!(
            profiles.join("notes.txt").exists(),
            "a non-profile file is left alone"
        );
    }

    #[test]
    fn check_reports_pending_work_and_writes_nothing() {
        let profiles = scratch("check");
        seed_profile(&profiles, "old-critic.md", Some("pf-old"));
        let retired = ["old-critic.md".to_string()];
        let before: Vec<String> = fs::read_dir(&profiles)
            .expect("profiles dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        let outcome = converge(
            &fixture_critique("check", None),
            &profiles,
            &[],
            &retired,
            true,
        )
        .expect("checked");
        assert_eq!(outcome.removed, ["old-critic.md".to_string()]);
        assert_eq!(outcome.written.len(), ROLES.len());
        let after: Vec<String> = fs::read_dir(&profiles)
            .expect("profiles dir")
            .map(|entry| {
                entry
                    .expect("entry")
                    .file_name()
                    .to_string_lossy()
                    .to_string()
            })
            .collect();
        assert_eq!(before, after, "a checked run writes nothing");
    }

    #[test]
    fn converge_works_on_a_directory_that_does_not_exist_yet() {
        let critique = fixture_critique("fresh", None);
        let profiles =
            std::env::temp_dir().join(format!("gold-band-roles-fresh-dir-{}", std::process::id()));
        let _ = fs::remove_dir_all(&profiles);
        let outcome = converge(&critique, &profiles, &[], &[], false).expect("converged");
        assert_eq!(outcome.written.len(), ROLES.len());
        for role in ROLES {
            assert!(
                profiles.join(format!("{}.md", role.file)).exists(),
                "{} is missing",
                role.file
            );
        }
        let _ = fs::remove_dir_all(&profiles);
    }

    #[test]
    fn a_profiles_path_that_is_a_file_is_refused() {
        let file = list_file("profiles-file", "not a directory");
        let outcome = converge(
            &fixture_critique("profiles-file", None),
            &file,
            &[],
            &[],
            false,
        );
        assert!(outcome.is_err(), "converging onto a file must fail");
    }

    #[test]
    fn the_temporary_file_is_named_after_its_target() {
        let out = scratch("atomic");
        fs::create_dir_all(out.join("decision-gap-critic.md"))
            .expect("a directory where a file goes");
        let problem = render_dir(&fixture_critique("atomic", None), &out)
            .expect_err("the profile cannot be renamed over a directory");
        assert!(problem.contains("decision-gap-critic.md"), "{problem}");
    }

    #[test]
    fn a_blocked_sibling_is_named_when_the_temporary_write_fails() {
        let out = scratch("blocked-tmp");
        fs::create_dir_all(out.join(".decision-gap-critic.md.tmp"))
            .expect("a directory in the way");
        let problem = render_dir(&fixture_critique("blocked-tmp", None), &out)
            .expect_err("the temporary file cannot be written over a directory");
        assert!(problem.contains(".decision-gap-critic.md.tmp"), "{problem}");
    }

    #[test]
    fn out_of_date_covers_missing_equal_and_different() {
        let dir = scratch("out-of-date");
        let path = dir.join("profile.md");
        assert!(out_of_date(&path, "body"), "a missing file is out of date");
        fs::write(&path, "body").expect("profile file");
        assert!(!out_of_date(&path, "body"), "an equal file is current");
        assert!(
            out_of_date(&path, "changed"),
            "a different file is out of date"
        );
    }

    #[test]
    fn remove_file_names_a_file_it_cannot_remove() {
        let missing = scratch("remove-file").join("none.md");
        let problem = remove_file(&missing).expect_err("the file is not there");
        assert!(problem.contains("none.md"), "{problem}");
    }

    #[test]
    fn front_matter_ids_are_read_from_between_the_markers() {
        assert_eq!(
            front_matter_id("---\nid: pf-gap\nname: x\n---\nbody\n"),
            Some("pf-gap".to_string())
        );
        assert_eq!(
            front_matter_id("---\r\n  id:   pf-gap  \r\n---\r\n"),
            Some("pf-gap".to_string())
        );
        assert_eq!(front_matter_id("---\nname: x\n---\n"), None);
        assert_eq!(front_matter_id("# No front matter\n"), None);
        assert_eq!(front_matter_id("---\nid: never closed\n"), None);
    }

    #[test]
    fn name_lists_are_read_and_malformed_ones_are_refused() {
        let listed =
            "keep:\n  - made-by-hand.md\n  - \"quoted.md\"\n\n  - \nretired:\n  - old.md\n";
        assert_eq!(
            name_list(listed, "keep").expect("keep list"),
            ["made-by-hand.md".to_string(), "quoted.md".to_string()]
        );
        assert_eq!(
            name_list(listed, "retired").expect("retired list"),
            ["old.md".to_string()]
        );
        assert_eq!(
            name_list("# nothing\n", "keep").expect("an absent list keeps nothing"),
            Vec::<String>::new()
        );
        assert!(name_list("keep: everything\n", "keep").is_err());
        assert!(name_list("keep:\n  just a scalar\n", "keep").is_err());
    }

    #[test]
    fn a_missing_list_file_lists_nothing_and_an_unreadable_one_is_refused() {
        let missing = Path::new("/no-such/gold-band-roles-keep-file");
        assert_eq!(
            name_file(missing.as_os_str(), "keep").expect("missing"),
            Vec::<String>::new()
        );
        let directory = scratch("unreadable-list");
        assert!(
            name_file(directory.as_os_str(), "keep").is_err(),
            "a directory is not a list file"
        );
    }

    #[test]
    fn run_renders_converges_and_refuses_bad_usage() {
        let critique = fixture_critique("run", None);
        let out = scratch("run-out");
        let keep = list_file("run-keep", "keep:\n  - made-by-hand.md\n");
        let retired = list_file("run-retired", "retired:\n  - old-critic.md\n");
        let resolved = [
            "converge".to_string(),
            critique.to_string_lossy().to_string(),
            out.to_string_lossy().to_string(),
            "--keep".to_string(),
            keep.to_string_lossy().to_string(),
            "--retired".to_string(),
            retired.to_string_lossy().to_string(),
        ]
        .into_iter()
        .map(OsString::from)
        .collect::<Vec<_>>();
        assert_eq!(run(resolved.clone()), 0, "the first converge converges");
        seed_profile(&out, "old-critic.md", Some("pf-old"));
        let mut checked = resolved.clone();
        checked.push("--check".into());
        assert_eq!(run(checked), 1, "a checked run with pending removal fails");
        assert_eq!(
            run(resolved.clone()),
            0,
            "the converge writes the pending work"
        );
        assert_eq!(
            run(resolved),
            0,
            "a second converge prints unchanged and succeeds"
        );
        let empty = std::env::temp_dir().join(format!(
            "gold-band-roles-checked-absent-{}",
            std::process::id()
        ));
        let _ = fs::remove_dir_all(&empty);
        let keep = list_file("run-keep-two", "keep:\n");
        let retired = list_file("run-retired-empty", "retired:\n");
        assert_eq!(
            run(vec![
                "converge".into(),
                critique.into_os_string(),
                empty.clone().into_os_string(),
                "--keep".into(),
                keep.into_os_string(),
                "--retired".into(),
                retired.into_os_string(),
                "--check".into(),
            ]),
            1,
            "a checked run with six pending writes fails"
        );
        assert!(
            !empty.exists(),
            "a checked run writes nothing, not even the directory"
        );
    }

    #[test]
    fn run_refuses_every_bad_command_line() {
        let critique = critique_dir();
        let out = scratch("bad-usage-out");
        let profiles = scratch("bad-usage-profiles");
        let two = |word: &str| vec![OsString::from(word), critique.clone().into_os_string()];
        assert_eq!(run(std::iter::empty()), 2);
        assert_eq!(run(vec!["dance".into()]), 2);
        assert_eq!(run(vec!["render".into()]), 2);
        assert_eq!(run(two("render")), 2);
        assert_eq!(
            run(vec![
                "render".into(),
                critique.clone().into_os_string(),
                out.clone().into_os_string(),
                "extra".into(),
            ]),
            2
        );
        assert_eq!(run(vec!["converge".into()]), 2);
        assert_eq!(run(two("converge")), 2);
        let begin = |flags: Vec<OsString>| {
            let mut args = vec![
                OsString::from("converge"),
                critique.clone().into_os_string(),
                profiles.clone().into_os_string(),
            ];
            args.extend(flags);
            args
        };
        assert_eq!(
            run(begin(vec!["--check".into()])),
            2,
            "converge needs both list files"
        );
        assert_eq!(
            run(begin(vec!["--keep".into()])),
            2,
            "a flag without a value"
        );
        assert_eq!(
            run(begin(vec![
                "--keep".into(),
                "nowhere".into(),
                "--retired".into(),
                "nowhere".into(),
                "--dance".into(),
            ])),
            2
        );
    }

    #[test]
    fn run_refuses_a_malformed_keep_or_retired_file() {
        let critique = fixture_critique("malformed-keep", None);
        let profiles = scratch("malformed-keep-out");
        let keep = list_file("malformed-keep", "keep: everything\n");
        let retired = list_file("malformed-keep-retired", "retired:\n");
        assert_eq!(
            run(vec![
                "converge".into(),
                critique.clone().into_os_string(),
                profiles.clone().into_os_string(),
                "--keep".into(),
                keep.into_os_string(),
                "--retired".into(),
                retired.into_os_string(),
            ]),
            1
        );
        let keep = list_file("malformed-keep-two", "keep:\n");
        let retired = list_file("malformed-retired", "retired: all of them\n");
        assert_eq!(
            run(vec![
                "converge".into(),
                critique.into_os_string(),
                profiles.into_os_string(),
                "--keep".into(),
                keep.into_os_string(),
                "--retired".into(),
                retired.into_os_string(),
            ]),
            1
        );
    }

    #[test]
    fn a_missing_keep_value_does_not_swallow_the_check_flag() {
        let flags: Vec<OsString> = ["--keep", "--check", "--retired", "r"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(converge_flags(&flags), None);
    }

    #[test]
    fn a_missing_retired_value_is_also_a_usage_error() {
        let flags: Vec<OsString> = ["--keep", "k", "--retired", "--check"]
            .into_iter()
            .map(OsString::from)
            .collect();
        assert_eq!(converge_flags(&flags), None);
    }

    #[test]
    fn the_closing_section_states_every_rule_the_runner_enforces() {
        for rule in ["read-only", "quote", "schema", "refute", "plan-limit"] {
            assert!(
                CLOSING.to_lowercase().contains(rule),
                "the closing section omits `{rule}`"
            );
        }
    }

    #[test]
    fn the_closing_section_states_the_allowlists_real_behaviour() {
        let stated = CLOSING
            .to_lowercase()
            .split_whitespace()
            .collect::<Vec<_>>()
            .join(" ");
        for phrase in [
            "personal default",
            "every critic family",
            "code but no history",
            "declined entry",
            "nedap",
            "no remote is refused",
        ] {
            assert!(
                stated.contains(phrase),
                "the allowlist bullet omits `{phrase}`"
            );
        }
    }

    #[test]
    fn the_role_to_agent_table_names_no_codex_and_only_catalogued_families() {
        let doc = fs::read_to_string(kit_root().join("docs").join("gold-band-roles.md"))
            .expect("the doc");
        let catalogue = gateway_families();
        let rows: Vec<Vec<String>> = doc
            .lines()
            .filter(|line| line.starts_with('|'))
            .map(|line| {
                line.trim()
                    .trim_matches('|')
                    .split('|')
                    .map(|cell| cell.trim().to_string())
                    .collect()
            })
            .collect();
        assert_eq!(
            rows.len(),
            8,
            "the table holds the header, the rule and six roles"
        );
        let known = [
            "mistral",
            "zhipu",
            "deepseek",
            "google",
            "anthropic",
            "openai",
            "alibaba",
            "moonshot",
            "xiaomi",
            "minimax",
            "xai",
            "tencent",
            "meituan",
            "stepfun",
            "codex",
        ];
        for row in rows.iter().skip(2) {
            assert_eq!(row.len(), 3, "the table has three columns");
            for cell in row {
                for word in cell.split(|c: char| !c.is_ascii_alphanumeric()) {
                    let word = word.to_ascii_lowercase();
                    if !known.contains(&word.as_str()) {
                        continue;
                    }
                    assert!(
                        catalogue.contains(&word),
                        "docs/gold-band-roles.md names `{word}`, which the gateway catalogue does not offer"
                    );
                    assert!(
                        word != "openai" && word != "codex",
                        "docs/gold-band-roles.md names `{word}` in a table cell"
                    );
                }
            }
        }
        let roles: Vec<&str> = rows[2..].iter().map(|row| row[0].as_str()).collect();
        assert_eq!(
            roles,
            [
                "Decision-gap critic",
                "Spec red team",
                "Ticket linter",
                "Diff reviewer",
                "Feature reviewer",
                "Adversarial verifier",
            ]
        );
        assert!(doc.contains("The family allowlist is not enforced on this Gold Band path"));
    }

    /// The model families the gateway's catalogue offers
    /// (`skills/workstation/model-gateway/defaults.json`).
    fn gateway_families() -> Vec<String> {
        let text = fs::read_to_string(
            kit_root()
                .join("skills")
                .join("workstation")
                .join("model-gateway")
                .join("defaults.json"),
        )
        .expect("the gateway catalogue");
        text.split("\"family\": \"")
            .skip(1)
            .filter_map(|rest| rest.split('"').next())
            .map(str::to_ascii_lowercase)
            .collect()
    }
}
