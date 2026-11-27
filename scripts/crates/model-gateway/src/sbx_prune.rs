//! Space the gateway's sandboxes leave in sbx. A run's sandbox is removed by `sbx rm --force` and takes its disk with
//! it (measured: `sbx df` is the same before and after a run), so what accumulates is the template images: the
//! gateway pins a new image digest per client about daily, and the digests it no longer pins stay in sbx's store.
//! This module removes those, and with `sandbox prune` the sandboxes of ended gateways too.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::{Process, Runtime};
use crate::sbx_images::{self, Record, IMAGES};
use crate::sbx_launch::{orphans, SBX, SBX_TIMEOUT_MS};
use std::collections::{BTreeMap, BTreeSet};

/// The repository the sbx store gives the images sandboxes are assembled from.
const ASSEMBLED: &str = "docker.io/library/sandbox-kit-assembled";

/// One image in sbx's store, by one name. An image with several names is listed once per name.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Template {
    pub id: String,
    pub repository: String,
    pub tag: String,
    pub size: u64,
}

impl Template {
    fn view(&self) -> Value {
        json::object(vec![("id", Value::str(&self.id)), ("repository", Value::str(&self.repository)), ("tag", Value::str(&self.tag)), ("size", Value::Num(self.size as f64))])
    }
}

/// What a prune removes, or with a dry run would remove.
#[derive(Debug, Default, PartialEq, Eq)]
pub struct Plan {
    pub sandboxes: Vec<String>,
    /// One entry per image, however many names it has.
    pub templates: Vec<Template>,
}

/// The repositories of the gateway's own client images.
fn own_repositories() -> BTreeSet<String> {
    IMAGES.iter().map(|image| format!("docker.io/sbx/{}", image.repo)).collect()
}

/// The images sbx stores, or none when sbx cannot list them (not installed, no daemon, or an answer that is not a list).
fn templates(process: &dyn Process) -> Vec<Template> {
    let Ok(result) = process.exec(SBX, &["template", "ls", "--all", "--digests", "--json"], Some(SBX_TIMEOUT_MS)) else {
        return Vec::new();
    };
    let Ok(Value::Arr(items)) = json::parse(&result.stdout) else {
        return Vec::new();
    };
    let text = |object: &Object, key: &str| object.get(key).and_then(Value::as_str).unwrap_or_default().to_string();
    items
        .iter()
        .filter_map(Value::as_object)
        .filter(|object| result.exit_code == 0 && !text(object, "id").is_empty())
        .map(|object| Template {
            id: text(object, "id"),
            repository: text(object, "repository"),
            tag: text(object, "tag"),
            size: match object.get("size") {
                Some(Value::Num(size)) if *size >= 0.0 => *size as u64,
                _ => 0,
            },
        })
        .collect()
}

/// The disk sbx's images take, from `sbx df`; none when it cannot say.
pub(crate) fn images_bytes(process: &dyn Process) -> Option<u64> {
    let result = process.exec(SBX, &["df", "--json"], Some(SBX_TIMEOUT_MS)).ok().filter(|result| result.exit_code == 0)?;
    let Value::Obj(answer) = json::parse(&result.stdout).ok()? else {
        return None;
    };
    let Some(Value::Arr(summaries)) = answer.get("summaries") else {
        return None;
    };
    let images = summaries.iter().filter_map(Value::as_object).find(|summary| summary.get("type").and_then(Value::as_str) == Some("Images"))?;
    match images.get("size") {
        Some(Value::Num(size)) if *size >= 0.0 => Some(*size as u64),
        _ => None,
    }
}

/// The digests the records pin: the images the gateway runs from.
pub fn pinned(records: &BTreeMap<String, Record>) -> BTreeSet<String> {
    records.values().map(|record| record.digest.clone()).collect()
}

/// What sbx says about its sandboxes, to find an image a sandbox still starts from; empty when sbx cannot say.
fn sandboxes_text(process: &dyn Process) -> String {
    match process.exec(SBX, &["ls", "--format", "json"], Some(SBX_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 => result.stdout,
        _ => String::new(),
    }
}

/// The images to remove. By default, an image is a candidate when the gateway's own repository names it and nothing
/// else does but the assembled copy sbx makes of it; with `all`, every image is, except what is pinned. An image a
/// sandbox still mentions is never one.
pub fn plan_templates(process: &dyn Process, pins: &BTreeSet<String>, all: bool) -> Vec<Template> {
    let own = own_repositories();
    let mut by_id: BTreeMap<String, Vec<Template>> = BTreeMap::new();
    for template in templates(process) {
        by_id.entry(template.id.clone()).or_default().push(template);
    }
    // Only asked of sbx when an image is a candidate.
    let in_use = if by_id.keys().any(|id| !pins.contains(id)) { sandboxes_text(process) } else { String::new() };
    let mut chosen = Vec::new();
    for (id, mut names) in by_id {
        let ours = names.iter().any(|name| own.contains(&name.repository));
        let only_ours = names.iter().all(|name| own.contains(&name.repository) || name.repository == ASSEMBLED);
        let used = in_use.contains(&id) || names.iter().any(|name| !name.tag.is_empty() && in_use.contains(&format!("{}:{}", name.repository, name.tag)));
        if pins.contains(&id) || used || !(all || (ours && only_ours)) {
            continue;
        }
        chosen.push(names.swap_remove(0));
    }
    chosen
}

/// Removes the images, one `sbx template rm` each. Returns the ones removed and a warning for each that was not.
pub fn remove_templates(process: &dyn Process, chosen: Vec<Template>) -> (Vec<Template>, Vec<String>) {
    let mut removed = Vec::new();
    let mut warnings = Vec::new();
    for template in chosen {
        match process.exec(SBX, &["template", "rm", "--force", &template.id], Some(SBX_TIMEOUT_MS)) {
            Ok(result) if result.exit_code == 0 => removed.push(template),
            Ok(result) => warnings.push(format!("the image {} ({}) was not removed (sbx template rm exited {}: {})", template.id, template.repository, result.exit_code, result.stderr.trim())),
            Err(reason) => warnings.push(format!("the image {} ({}) was not removed ({reason})", template.id, template.repository)),
        }
    }
    (removed, warnings)
}

/// The images of the gateway's clients that the records no longer pin, removed. Run after a refresh pinned a new digest.
/// Nothing here fails a launch: what cannot be removed comes back as warnings.
pub fn remove_superseded(process: &dyn Process, records: &BTreeMap<String, Record>) -> Vec<String> {
    let chosen = plan_templates(process, &pinned(records), false);
    remove_templates(process, chosen).1
}

/// `sandbox prune [--all] [--dry-run]`: the sandboxes of ended gateways and the images the records do not pin.
pub fn command(input: &Object, runtime: &Runtime) -> Result<Value, GatewayError> {
    let flag = |name: &str| input.get(name) == Some(&Value::Bool(true));
    let (all, dry_run) = (flag("all"), flag("dry_run"));
    let mut warnings = Vec::new();
    let path = sbx_images::records_path(&runtime.env)?;
    // With no readable record nothing says which images are pinned, so no image is removed.
    let records = sbx_images::read_records(runtime.files.as_ref(), &path);
    if let Err(reason) = &records {
        warnings.push(format!("{} cannot be read ({reason}); no image is removed", sbx_images::RECORDS_FILE));
    }
    let pins = records.as_ref().map(pinned).ok();
    let process = runtime.process.as_ref();
    let mut plan = Plan { sandboxes: orphans(process), templates: pins.map(|pins| plan_templates(process, &pins, all)).unwrap_or_default() };
    let before = if dry_run { None } else { images_bytes(process) };
    if !dry_run {
        let mut gone = Vec::new();
        for name in std::mem::take(&mut plan.sandboxes) {
            match process.exec(SBX, &["rm", "--force", &name], Some(SBX_TIMEOUT_MS)) {
                Ok(result) if result.exit_code == 0 => gone.push(name),
                _ => warnings.push(format!("the sandbox {name} of an ended run was not removed; remove it with sbx rm --force {name}")),
            }
        }
        plan.sandboxes = gone;
        let (removed, failed) = remove_templates(process, std::mem::take(&mut plan.templates));
        plan.templates = removed;
        warnings.extend(failed);
    }
    // What sbx's own count of its images fell by, which counts shared layers once; the listed sizes where it cannot say.
    let freed = before.zip(images_bytes(process)).map(|(before, after)| before.saturating_sub(after));
    Ok(answer(&plan, freed, all, dry_run, &warnings))
}

fn answer(plan: &Plan, freed: Option<u64>, all: bool, dry_run: bool, warnings: &[String]) -> Value {
    let freed = freed.unwrap_or_else(|| plan.templates.iter().map(|template| template.size).sum());
    json::object(vec![
        ("dry_run", Value::Bool(dry_run)),
        ("all", Value::Bool(all)),
        ("sandboxes", Value::Arr(plan.sandboxes.iter().map(|name| Value::str(name)).collect())),
        ("templates", Value::Arr(plan.templates.iter().map(Template::view).collect())),
        ("freed_bytes", Value::Num(freed as f64)),
        ("warnings", Value::Arr(warnings.iter().map(|text| Value::str(text)).collect())),
    ])
}

/// The plain-text answer of `sandbox prune`: what was (or would be) removed, the size of those images, the warnings.
pub fn table(result: &Value) -> String {
    let object = result.as_object();
    let list = |key: &str| object.and_then(|object| object.get(key)).and_then(|value| if let Value::Arr(items) = value { Some(items.as_slice()) } else { None }).unwrap_or_default();
    let verb = if object.and_then(|object| object.get("dry_run")) == Some(&Value::Bool(true)) { "would remove" } else { "removed" };
    let mut lines = Vec::new();
    for name in list("sandboxes").iter().filter_map(Value::as_str) {
        lines.push(format!("{verb} sandbox {name}"));
    }
    for template in list("templates").iter().filter_map(Value::as_object) {
        let field = |key: &str| template.get(key).and_then(Value::as_str).unwrap_or_default();
        let size = if let Some(Value::Num(size)) = template.get("size") { *size } else { 0.0 };
        let name = if field("tag").is_empty() { field("repository").to_string() } else { format!("{}:{}", field("repository"), field("tag")) };
        lines.push(format!("{verb} image {} {name} ({:.1} MB)", field("id"), size / 1_000_000.0));
    }
    let freed = if let Some(Value::Num(bytes)) = object.and_then(|object| object.get("freed_bytes")) { *bytes } else { 0.0 };
    lines.push(format!("{verb} {:.1} MB (a dry run adds up the listed image sizes, which count shared layers once each)", freed / 1_000_000.0));
    lines.extend(list("warnings").iter().filter_map(Value::as_str).map(|text| format!("warning: {text}")));
    format!("{}\n", lines.join("\n"))
}
