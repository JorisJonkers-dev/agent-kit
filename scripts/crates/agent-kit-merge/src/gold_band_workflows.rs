//! `agent-kit-merge converge gold-band-workflows`: the workflow templates of the personal Gold Band instance's
//! `context/workflows.json` (the app's user-level template store), one template per `installer/gold-band/workflows/
//! *.json` (the Rust port of `scripts/converge.py gold-band-workflows`, #810).
//!
//! A template the store lacks is added; one whose `workflow` or `modelBindings` differ from the file is replaced in
//! place, keeping its name and `createdAt`; one the keep file names (by workflow id, or `<id>.json`) is never touched.
//! A file bound to an agent of the work instance is not installed here, and the app's built-in templates and every
//! template the kit does not ship are left exactly as they are. A template that fails Gold Band's editor shape rules,
//! or breaks the critique rules (`workflow-lint`, in process), refuses the run unless a known-failures file beside the
//! templates names it. The store is replaced atomically, with one `workflows.json.bak`.

use crate::gold_band::{self, Processes};
use crate::json::{self, Object, Value};
use crate::{Env, Failure, Io};
use regex::Regex;
use std::collections::BTreeSet;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};

const TOOL: &str = "gold-band-workflows";

/// The app's own built-in template ids.
const BUILT_INS: [&str; 2] = ["default", "default-lightweight"];

const STORE_VERSION: &str = "0.1";

const SHAPE_KNOWN: &str = "known-shape-failures.txt";
const CRITIQUE_KNOWN: &str = "known-critique-failures.txt";

/// The work instance, whose agents a personal-instance template may not bind.
const WORK_INSTANCE: &str = "gold-band";

/// Why the store cannot be converged at all.
struct Cannot(String);

/// The flags of `converge gold-band-workflows`.
pub struct WorkflowArgs {
    pub keep: PathBuf,
    pub registry: PathBuf,
    pub workflows: Option<PathBuf>,
    pub store: Option<PathBuf>,
    pub check: bool,
}

/// One template file: its `workflow` and `modelBindings` as the store holds them, and the parsed file the lint reads.
struct Source {
    id: String,
    workflow: Value,
    bindings: Value,
    file: serde_json::Value,
}

/// The agent ids the registry gives the work instance.
fn work_agents(path: &Path) -> Result<BTreeSet<String>, Cannot> {
    let unreadable = |error: String| Cannot(format!("{} cannot be read: {error}", path.display()));
    let text = fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
    let data: serde_yaml::Value = serde_yaml::from_str(&text).map_err(|error| unreadable(error.to_string()))?;
    let agents: Option<BTreeSet<String>> = data
        .get("gold_band_instances")
        .and_then(|instances| instances.get(WORK_INSTANCE))
        .and_then(|work| work.get("agents"))
        .and_then(serde_yaml::Value::as_sequence)
        .and_then(|agents| agents.iter().map(|agent| agent.as_str().map(str::to_string)).collect());
    agents.ok_or_else(|| Cannot(format!("{}: gold_band_instances.{WORK_INSTANCE}.agents must be a list of ids", path.display())))
}

/// Python's `repr` of a JSON value read as a workflow id.
fn repr(value: Option<&Value>) -> String {
    match value {
        None | Some(Value::Null) => "None".to_string(),
        Some(Value::Str(text)) => crate::text::repr(text),
        Some(Value::Bool(flag)) => if *flag { "True" } else { "False" }.to_string(),
        Some(Value::Num(text)) => text.clone(),
        Some(other) => json::dumps(other),
    }
}

/// Every `*.json` of the directory, sorted by stem; one bad file refuses them all.
fn sources(directory: &Path) -> Result<Vec<Source>, Cannot> {
    if !directory.is_dir() {
        return Err(Cannot(format!("{} is not a directory", directory.display())));
    }
    let id_pattern = Regex::new(r"^[a-z0-9]+(-[a-z0-9]+)*$").expect("pattern");
    let mut paths: Vec<PathBuf> = fs::read_dir(directory)
        .into_iter()
        .flatten()
        .flatten()
        .map(|entry| entry.path())
        .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
        .collect();
    paths.sort_by_key(|path| path.file_stem().unwrap_or_default().to_os_string());
    let mut found = Vec::new();
    for path in paths {
        let stem = path.file_stem().unwrap_or_default().to_string_lossy().into_owned();
        let unreadable = |error: String| Cannot(format!("{} cannot be read: {error}", path.display()));
        let text = fs::read_to_string(&path).map_err(|error| unreadable(error.to_string()))?;
        let body = json::parse(&text).map_err(|Failure(error)| unreadable(error))?;
        let file: serde_json::Value = serde_json::from_str(&text).map_err(|error| unreadable(error.to_string()))?;
        let object = body.as_object();
        let (workflow, bindings) = match (object.and_then(|o| o.get("workflow")), object.and_then(|o| o.get("modelBindings"))) {
            (Some(workflow @ Value::Obj(_)), Some(bindings @ Value::Obj(_))) => (workflow.clone(), bindings.clone()),
            _ => return Err(Cannot(format!("{} must hold a `workflow` object and a `modelBindings` object", path.display()))),
        };
        let id = workflow.as_object().and_then(|w| w.get("id"));
        match id {
            Some(Value::Str(id)) if id_pattern.is_match(id) && *id == stem => {}
            other => return Err(Cannot(format!("{}: the workflow id must be the file's own name ({}), got {}", path.display(), crate::text::repr(&stem), repr(other)))),
        }
        if BUILT_INS.contains(&stem.as_str()) {
            return Err(Cannot(format!("{}: {} is the id of a built-in template of the app", path.display(), crate::text::repr(&stem))));
        }
        found.push(Source { id: stem, workflow, bindings, file });
    }
    Ok(found)
}

/// The non-comment lines of a known-failures file, each read by `name`; none without the file.
fn known(directory: &Path, file: &str, name: fn(&str) -> String) -> Result<BTreeSet<String>, Cannot> {
    let path = directory.join(file);
    if !path.is_file() {
        return Ok(BTreeSet::new());
    }
    let text = fs::read_to_string(&path).map_err(|error| Cannot(format!("{} cannot be read: {error}", path.display())))?;
    Ok(text.lines().filter(|line| !line.trim().is_empty() && !line.trim_start().starts_with('#')).map(name).collect())
}

fn without_json(name: &str) -> String {
    name.strip_suffix(".json").unwrap_or(name).to_string()
}

/// The editor's shape rules: a failing template refuses the run unless `known-shape-failures.txt` names it.
fn shape_gate(directory: &Path, sources: &[Source]) -> Result<Vec<String>, Cannot> {
    let known = known(directory, SHAPE_KNOWN, |line| without_json(line.trim()))?;
    let (mut refused, mut warnings) = (Vec::new(), Vec::new());
    for source in sources {
        let definition = source.file.get("workflow").expect("a workflow object");
        let findings = workflow_lint::shape_of_definition(definition);
        if findings.is_empty() {
            continue;
        }
        if known.contains(&source.id) {
            warnings.push(format!(
                "warning: {} fails the editor's shape rules ({} findings) and is a known failure ({SHAPE_KNOWN}, agent-kit#781); installing it anyway",
                source.id,
                findings.len()
            ));
            continue;
        }
        refused.extend(findings.iter().map(|finding| format!("{}.json: {finding}", source.id)));
    }
    if !refused.is_empty() {
        return Err(Cannot(format!(
            "a template fails Gold Band's workflow shape rules:\n  {}\nfix it, or (for a known failure) list it in {}",
            refused.join("\n  "),
            directory.join(SHAPE_KNOWN).display()
        )));
    }
    Ok(warnings)
}

/// The critique rules (#793) and the base lint, as `workflow-lint --critique` reports them: a violator refuses the run
/// unless `known-critique-failures.txt` names it.
fn critique_gate(directory: &Path, sources: &[Source]) -> Result<Vec<String>, Cannot> {
    let path = directory.join(CRITIQUE_KNOWN);
    let known = known(directory, CRITIQUE_KNOWN, |line| without_json(line.split_whitespace().next().unwrap_or_default()))?;
    let (mut refused, mut warnings) = (Vec::new(), Vec::new());
    for source in sources {
        let instance = if source.id.ends_with("-work") { workflow_lint::Instance::Work } else { workflow_lint::Instance::Personal };
        let shown = directory.join(format!("{}.json", source.id)).display().to_string();
        let linted = workflow_lint::lint(&source.file, instance).and_then(|mut found| {
            found.extend(workflow_lint::lint_critique(&source.file, instance)?);
            Ok(found)
        });
        let found = match linted {
            Ok(found) if found.is_empty() => continue,
            Ok(found) => found,
            Err(error) => {
                let said: String = format!("workflow-lint: {shown}: invalid workflow: {error}").chars().take(200).collect();
                return Ok(vec![format!("warning: the critique rules (#793) were not checked: {said}")]);
            }
        };
        if known.contains(&source.id) {
            warnings.push(format!(
                "warning: {} breaks the critique rules ({} findings) and is a known failure ({CRITIQUE_KNOWN}); installing it anyway",
                source.id,
                found.len()
            ));
        } else {
            refused.extend(found.iter().map(|finding| format!("workflow-lint: {shown}: {finding}")));
        }
    }
    if !refused.is_empty() {
        return Err(Cannot(format!(
            "a template breaks the critique rules (critic-required, critic-family):\n  {}\nfix it, or (for a known failure) list it in {}",
            refused.join("\n  "),
            path.display()
        )));
    }
    Ok(warnings)
}

/// The agents a template binds: its model bindings' `agentId`s and its worker nodes' `provider`s.
fn bound_agents(source: &Source) -> BTreeSet<String> {
    let mut agents = BTreeSet::new();
    if let Some(Value::Arr(bindings)) = source.bindings.as_object().and_then(|b| b.get("bindings")) {
        agents.extend(bindings.iter().filter_map(|b| b.as_object()?.get("agentId")).filter_map(text));
    }
    if let Some(Value::Arr(nodes)) = source.workflow.as_object().and_then(|w| w.get("nodes")) {
        let workers = nodes.iter().filter_map(Value::as_object).filter(|node| matches!(node.get("type"), Some(Value::Str(kind)) if kind == "worker"));
        agents.extend(workers.filter_map(|node| node.get("provider")).filter_map(text));
    }
    agents
}

fn text(value: &Value) -> Option<String> {
    match value {
        Value::Str(text) => Some(text.clone()),
        _ => None,
    }
}

/// The comparison form: the app drops a `null` field it re-saves, so a missing key equals a null one.
fn without_nulls(value: &Value) -> Value {
    match value {
        Value::Obj(object) => {
            let mut kept = Object::default();
            for (key, item) in object.entries() {
                if *item != Value::Null {
                    kept.set(key, without_nulls(item));
                }
            }
            Value::Obj(kept)
        }
        Value::Arr(items) => Value::Arr(items.iter().map(without_nulls).collect()),
        other => other.clone(),
    }
}

/// The store as it is, or a new empty one; a file that is not a store is refused, never replaced.
fn store(path: &Path) -> Result<Object, Cannot> {
    if !path.exists() {
        let mut store = Object::default();
        store.set("version", Value::Str(STORE_VERSION.to_string()));
        store.set("lastUsedTemplateId", Value::Null);
        store.set("lastCreatedWorkflow", Value::Null);
        store.set("templates", Value::Arr(Vec::new()));
        return Ok(store);
    }
    let unreadable = |error: String| Cannot(format!("{} cannot be read: {error}", path.display()));
    let text = fs::read_to_string(path).map_err(|error| unreadable(error.to_string()))?;
    let parsed = json::parse(&text).map_err(|Failure(error)| unreadable(error))?;
    match parsed {
        Value::Obj(store) if matches!(store.get("templates"), Some(Value::Arr(templates)) if templates.iter().all(|t| t.as_object().is_some())) => Ok(store),
        _ => Err(Cannot(format!("{} is not a workflow template store (no `templates` list); nothing was written", path.display()))),
    }
}

/// Makes a missing directory readable by this user alone.
fn private_dir(path: &Path) -> std::io::Result<()> {
    use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
    if !path.exists() {
        fs::DirBuilder::new().recursive(true).mode(0o700).create(path)?;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    }
    Ok(())
}

/// Replaces the store atomically (staged beside it, synced, renamed), keeping one backup of what it held.
fn write_store(path: &Path, store: &Value) -> std::io::Result<()> {
    use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
    let context = path.parent().unwrap_or(Path::new(""));
    private_dir(context.parent().unwrap_or(Path::new("")))?;
    private_dir(context)?;
    let named = |suffix: &str| crate::with_suffix(path, suffix);
    let (backup, staged, staged_backup) = (named(".bak"), named(".agent-kit-tmp"), named(".bak.agent-kit-tmp"));
    let written = (|| {
        if path.is_file() {
            crate::copy_keeping(path, &staged_backup).map_err(|Failure(error)| std::io::Error::other(error))?;
            fs::set_permissions(&staged_backup, fs::Permissions::from_mode(0o600))?;
            fs::rename(&staged_backup, &backup)?;
        }
        let _ = fs::remove_file(&staged);
        let mut file = fs::OpenOptions::new().write(true).create_new(true).mode(0o600).open(&staged)?;
        file.write_all((json::dumps_unicode(store) + "\n").as_bytes())?;
        file.sync_all()?;
        fs::rename(&staged, path)
    })();
    if written.is_err() {
        let _ = fs::remove_file(&staged);
        let _ = fs::remove_file(&staged_backup);
    }
    written
}

/// Everything read and checked before the store changes, or why the run stops.
fn prepare(args: &WorkflowArgs, directory: &Path, store_path: &Path, data: &Path, processes: &dyn Processes, env: Env) -> Result<Result<(BTreeSet<String>, Vec<Source>, Vec<String>, Object), String>, Cannot> {
    let work = work_agents(&args.registry)?;
    let sources = sources(directory)?;
    let mut warnings = shape_gate(directory, &sources)?;
    warnings.extend(critique_gate(directory, &sources)?);
    if !args.check {
        if let Some(running) = gold_band::running(data, env, processes) {
            return Ok(Err(running));
        }
    }
    Ok(Ok((work, sources, warnings, store(store_path)?)))
}

/// `converge gold-band-workflows`; `now` is the stamp a template it adds or changes carries.
pub fn run(args: &WorkflowArgs, now: &str, processes: &dyn Processes, io: &mut Io, env: Env) -> Result<i32, Failure> {
    let store_path = args.store.clone().unwrap_or_else(|| gold_band::settings_path(env).parent().unwrap_or(Path::new("")).join("context").join("workflows.json"));
    let data = store_path.parent().and_then(Path::parent).unwrap_or(Path::new("")).to_path_buf();
    let kit = args.registry.parent().and_then(Path::parent).unwrap_or(Path::new(""));
    let directory = args.workflows.clone().unwrap_or_else(|| kit.join("installer/gold-band/workflows"));
    let (work, sources, warnings, store) = match prepare(args, &directory, &store_path, &data, processes, env) {
        Err(Cannot(message)) => {
            writeln!(io.out, "{TOOL}: cannot converge: {message}")?;
            return Ok(3);
        }
        Ok(Err(running)) => {
            writeln!(io.out, "{TOOL}: Gold Band instance {running} is running; quit it and re-run setup (nothing was written)")?;
            return Ok(gold_band::RUNNING_EXIT);
        }
        Ok(Ok(prepared)) => prepared,
    };
    for warning in &warnings {
        writeln!(io.out, "{TOOL}: {warning}")?;
    }
    let keep = match crate::converge::keep(&args.keep, io)? {
        Ok(keep) => keep,
        Err(status) => return Ok(status),
    };
    // The store was read as one whose templates are all objects.
    let mut templates: Vec<Object> = store.get("templates").into_iter().flat_map(|templates| if let Value::Arr(items) = templates { items.clone() } else { Vec::new() }).filter_map(|t| t.as_object().cloned()).collect();
    let mut slots: Vec<(String, usize)> = Vec::new();
    for (position, template) in templates.iter().enumerate() {
        let inner = template.get("workflow").and_then(Value::as_object).and_then(|w| w.get("id"));
        for key in [template.get("id"), inner].into_iter().flatten().filter_map(text) {
            if !slots.iter().any(|(known, _)| *known == key) {
                slots.push((key, position));
            }
        }
    }
    let slot = |slots: &[(String, usize)], id: &str| slots.iter().find(|(known, _)| known == id).map(|(_, at)| *at);
    let (added_word, changed_word) = if args.check { ("would add", "would change") } else { ("added", "changed") };
    let mut lines: Vec<String> = Vec::new();
    let mut changed = false;
    for source in &sources {
        let foreign: Vec<String> = bound_agents(source).intersection(&work).cloned().collect();
        if !foreign.is_empty() {
            lines.push(format!("skipped {} (bound to the work instance's {})", source.id, foreign.join(", ")));
            continue;
        }
        let index = slot(&slots, &source.id);
        if index.is_some_and(|at| templates[at].get("isBuiltIn") == Some(&Value::Bool(true))) {
            writeln!(io.out, "{TOOL}: {}: the existing template is a built-in; nothing was written", source.id)?;
            return Ok(1);
        }
        if keep.contains(&source.id) || keep.contains(&format!("{}.json", source.id)) {
            lines.push(format!("kept {} (keep file)", source.id));
            continue;
        }
        let Some(index) = index else {
            let mut template = Object::default();
            template.set("id", Value::Str(source.id.clone()));
            template.set("name", Value::Str(source.id.clone()));
            template.set("isBuiltIn", Value::Bool(false));
            template.set("workflow", source.workflow.clone());
            template.set("modelBindings", source.bindings.clone());
            template.set("createdAt", Value::Str(now.to_string()));
            template.set("updatedAt", Value::Str(now.to_string()));
            templates.push(template);
            slots.push((source.id.clone(), templates.len() - 1));
            lines.push(format!("{added_word} {}", source.id));
            changed = true;
            continue;
        };
        let held = &templates[index];
        let same = |key: &str, wanted: &Value| without_nulls(held.get(key).unwrap_or(&Value::Null)) == without_nulls(wanted);
        if same("workflow", &source.workflow) && same("modelBindings", &source.bindings) {
            continue;
        }
        let template = &mut templates[index];
        template.set("workflow", source.workflow.clone());
        template.set("modelBindings", source.bindings.clone());
        template.set("updatedAt", Value::Str(now.to_string()));
        lines.push(format!("{changed_word} {}", source.id));
        changed = true;
    }
    for line in &lines {
        writeln!(io.out, "{line}")?;
    }
    if !changed {
        writeln!(io.out, "workflows unchanged")?;
        return Ok(0);
    }
    if args.check {
        return Ok(0);
    }
    // The one write group: the guard runs again right before it.
    if let Some(running) = gold_band::running(&data, env, processes) {
        let name = store_path.file_name().unwrap_or_default().to_string_lossy().into_owned();
        return gold_band::stopped(TOOL, &running, &[], &[name], io);
    }
    let mut written = store;
    written.set("templates", Value::Arr(templates.into_iter().map(Value::Obj).collect()));
    if let Err(error) = write_store(&store_path, &Value::Obj(written)) {
        writeln!(io.out, "{TOOL}: cannot converge: {error}; the old store is in place")?;
        return Ok(3);
    }
    Ok(0)
}

/// The stamp Python's `f"{int(time.time())}Z"` makes: whole seconds since the epoch, then `Z`.
pub fn now_stamp() -> String {
    let seconds = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map_or(0, |elapsed| elapsed.as_secs());
    format!("{seconds}Z")
}

#[cfg(test)]
#[path = "gold_band_workflows/tests.rs"]
mod tests;
