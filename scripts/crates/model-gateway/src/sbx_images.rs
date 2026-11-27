//! The Docker Sandboxes images a sandbox starts from. Docker publishes them as `sbx/<name>-image` on Docker Hub, one
//! tag per day (`YYYYMMDD-<40-hex commit>`) plus `latest`. The gateway runs the newest dated tag at least two days
//! old (a cooldown, so a bad day's build is never the one a run gets), pins it by digest, and records the choice in
//! `sbx-images.json` under the state directory. A record older than a day is refreshed by the next launch; a refresh
//! that fails keeps the record it had. Every call goes through the `Process`, `Files` and `Clock` ports.

use crate::error::GatewayError;
use crate::json::{self, Object, Value};
use crate::ports::{Clock, Files, Process, Runtime};
use crate::state_file::state_dir;
use crate::text::{dirname, join};
use crate::timestamps::to_iso;
use std::collections::{BTreeMap, HashMap};

/// A tag is only taken once it is this many days old, so a fresh build never reaches a run at once.
pub const COOLDOWN_DAYS: i64 = 2;

/// A record older than this is refreshed before the next launch that needs it.
pub const MAX_AGE_MS: i64 = 24 * 60 * 60 * 1000;

const DAY_MS: i64 = 24 * 60 * 60 * 1000;

/// How long the Docker Hub answer may take before the fetch counts as failed.
const FETCH_TIMEOUT_MS: u64 = 30_000;

/// The file the records live in, beside the gateway's other state.
pub const RECORDS_FILE: &str = "sbx-images.json";

/// One client's image: the gateway client, the Docker Hub repository under `sbx/`, and whether the sbx backend runs it.
pub struct Image {
    pub client: &'static str,
    pub repo: &'static str,
    pub enabled: bool,
}

/// Every client that has an image. Every client but Antigravity runs in a sandbox; a client listed with its
/// flag off is refused until a slice enables it.
pub const IMAGES: [Image; 6] = [
    Image { client: "pi", repo: "pi-image", enabled: true },
    Image { client: "opencode", repo: "opencode-image", enabled: true },
    Image { client: "claude", repo: "claude-image", enabled: true },
    Image { client: "codex", repo: "codex-image", enabled: true },
    Image { client: "vibe", repo: "vibe-image", enabled: true },
    Image { client: "hermes", repo: "hermes-agent-image", enabled: true },
];

/// The image of a client the gateway knows, enabled or not.
pub fn image_of(client: &str) -> Option<&'static Image> {
    IMAGES.iter().find(|image| image.client == client)
}

/// The tag a client runs, with the digest it is pinned by and the moment the registry was asked.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Record {
    pub tag: String,
    pub digest: String,
    pub checked_at: i64,
}

impl Record {
    /// The image reference sbx takes: the repository and the pinned digest, never a tag that could move.
    pub fn reference(&self, image: &Image) -> String {
        format!("docker.io/sbx/{}@{}", image.repo, self.digest)
    }

    fn to_json(&self) -> Value {
        json::object(vec![("tag", Value::str(&self.tag)), ("digest", Value::str(&self.digest)), ("checked_at", Value::Num(self.checked_at as f64))])
    }

    fn of_json(value: &Value) -> Option<Record> {
        let object = value.as_object()?;
        let checked = match object.get("checked_at")? {
            Value::Num(ms) => *ms as i64,
            _ => return None,
        };
        Some(Record { tag: object.get("tag")?.as_str()?.to_string(), digest: object.get("digest")?.as_str()?.to_string(), checked_at: checked })
    }
}

/// The records file's path: `sbx-images.json` in the gateway's state directory.
pub fn records_path(env: &HashMap<String, String>) -> Result<String, GatewayError> {
    Ok(join(&[&state_dir(env)?, RECORDS_FILE]))
}

/// The records the file holds, by client. A file that is not there holds none; one that cannot be read or parsed is
/// an error the caller reports.
pub fn read_records(files: &dyn Files, path: &str) -> Result<BTreeMap<String, Record>, String> {
    let text = match files.read_text(path) {
        Ok(text) => text,
        Err(error) if error.code == "ENOENT" => return Ok(BTreeMap::new()),
        Err(error) => return Err(error.message),
    };
    let parsed = json::parse(&text).map_err(|reason| format!("not JSON: {reason}"))?;
    let object = parsed.as_object().ok_or_else(|| "not a JSON object".to_string())?;
    let mut records = BTreeMap::new();
    for (client, value) in object.entries() {
        let record = Record::of_json(value).ok_or_else(|| format!("{client} has no tag, digest and checked_at"))?;
        records.insert(client.to_string(), record);
    }
    Ok(records)
}

/// Writes the records, whole, through the files port's atomic write.
pub fn write_records(files: &dyn Files, path: &str, records: &BTreeMap<String, Record>) -> Result<(), String> {
    let mut object = Object::new();
    for (client, record) in records {
        object.set(client, record.to_json());
    }
    files.mkdir_all(dirname(path)).map_err(|error| error.message)?;
    files.write_atomic(path, &format!("{}\n", json::pretty(&Value::Obj(object)))).map_err(|error| error.message)
}

/// One tag of a Docker Hub listing that the gateway reads.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tag {
    pub name: String,
    pub digest: String,
    pub updated: String,
}

/// The tags of a Docker Hub `repositories/<repo>/tags` answer. An entry without a name or digest is skipped.
pub fn parse_tags(text: &str) -> Result<Vec<Tag>, String> {
    let parsed = json::parse(text).map_err(|reason| format!("the Docker Hub answer does not parse: {reason}"))?;
    let Some(Value::Arr(items)) = parsed.as_object().and_then(|object| object.get("results")) else {
        return Err("the Docker Hub answer has no results list".to_string());
    };
    Ok(items
        .iter()
        .filter_map(|item| {
            let object = item.as_object()?;
            Some(Tag { name: object.get("name")?.as_str()?.to_string(), digest: object.get("digest")?.as_str()?.to_string(), updated: object.get("last_updated").and_then(Value::as_str).unwrap_or_default().to_string() })
        })
        .collect())
}

/// Whether a tag is `YYYYMMDD-<40 lowercase hex>` and its digest `sha256:<64 lowercase hex>`. `latest` is not.
fn well_formed(name: &str, digest: &str) -> bool {
    let name = name.as_bytes();
    let lower_hex = |byte: &u8| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte);
    let name_ok = name.len() == 49 && name[8] == b'-' && name[..8].iter().all(u8::is_ascii_digit) && name[9..].iter().all(lower_hex);
    let digest_ok = digest.strip_prefix("sha256:").is_some_and(|hex| hex.len() == 64 && hex.bytes().all(|byte| lower_hex(&byte)));
    name_ok && digest_ok
}

/// The newest tag dated on or before `cutoff` (`YYYYMMDD`), with its digest. Newest is the date, then the registry's
/// last-updated moment; malformed tags and digests are ignored.
pub fn choose_tag(tags: &[Tag], cutoff: &str) -> Option<(String, String)> {
    tags.iter()
        .filter(|tag| well_formed(&tag.name, &tag.digest) && &tag.name[..8] <= cutoff)
        .max_by(|left, right| (&left.name[..8], &left.updated).cmp(&(&right.name[..8], &right.updated)))
        .map(|tag| (tag.name.clone(), tag.digest.clone()))
}

/// The newest date a tag may carry at `now_ms`: two days before today, UTC, as `YYYYMMDD`.
pub fn cutoff(now_ms: i64) -> String {
    to_iso(now_ms - COOLDOWN_DAYS * DAY_MS)[..10].replace('-', "")
}

/// The tags of one image's repository, from Docker Hub through the process port.
fn fetch_tags(process: &dyn Process, image: &Image) -> Result<Vec<Tag>, String> {
    let url = format!("https://hub.docker.com/v2/repositories/sbx/{}/tags?page_size=100&ordering=last_updated", image.repo);
    let result = process.exec("curl", &["-fsS", &url], Some(FETCH_TIMEOUT_MS)).map_err(|reason| format!("curl did not run: {reason}"))?;
    if result.exit_code != 0 || result.timed_out {
        return Err(format!("Docker Hub answered exit {}: {}", result.exit_code, result.stderr.trim()));
    }
    parse_tags(&result.stdout)
}

/// One refresh of one image: the newest qualifying tag, asked of the registry now.
pub fn refresh_one(process: &dyn Process, clock: &dyn Clock, image: &Image) -> Result<Record, String> {
    let tags = fetch_tags(process, image)?;
    let now = clock.now_ms();
    let (tag, digest) = choose_tag(&tags, &cutoff(now)).ok_or_else(|| format!("no {} tag is {COOLDOWN_DAYS} days old yet", image.repo))?;
    Ok(Record { tag, digest, checked_at: now })
}

/// Whether a record is young enough to use without asking the registry again.
fn fresh(record: &Record, now_ms: i64) -> bool {
    (0..MAX_AGE_MS).contains(&(now_ms - record.checked_at))
}

/// The image reference a launch of `client` runs from, refreshing its record first when it is missing or older than
/// a day. A refresh that fails keeps the record it had (with a warning); with no record at all, the launch is refused.
pub fn ensure(process: &dyn Process, files: &dyn Files, clock: &dyn Clock, env: &HashMap<String, String>, client: &str) -> Result<(String, Vec<String>), GatewayError> {
    let Some(image) = image_of(client) else {
        return Err(GatewayError::new("sandbox-unsupported", format!("{client}: no sbx image is known for it")));
    };
    let path = records_path(env)?;
    let mut warnings = Vec::new();
    let mut records = read_records(files, &path).unwrap_or_else(|reason| {
        warnings.push(format!("{RECORDS_FILE} cannot be read ({reason}); the images are refreshed"));
        BTreeMap::new()
    });
    let existing = records.get(client).cloned();
    if let Some(record) = existing.as_ref().filter(|record| fresh(record, clock.now_ms())) {
        return Ok((record.reference(image), warnings));
    }
    match refresh_one(process, clock, image) {
        Ok(record) => {
            let reference = record.reference(image);
            records.insert(client.to_string(), record);
            if let Err(reason) = write_records(files, &path, &records) {
                warnings.push(format!("{RECORDS_FILE} was not written ({reason})"));
            }
            Ok((reference, warnings))
        }
        Err(reason) => match existing {
            Some(record) => {
                warnings.push(format!("{client}: the image refresh failed ({reason}); the {} record from {} is kept", record.tag, to_iso(record.checked_at)));
                Ok((record.reference(image), warnings))
            }
            None => Err(GatewayError::new("sandbox-unavailable", format!("{client}: no sbx image is recorded and the refresh failed ({reason}); run model-gateway sandbox refresh"))),
        },
    }
}

/// One client's entry in the CLI's answer: the bare image repository, and the tag, digest and RFC 3339 checked_at of its
/// record, or nulls when it has none.
fn view(image: &Image, record: Option<&Record>) -> Value {
    let (tag, digest, checked) = match record {
        Some(record) => (Value::str(&record.tag), Value::str(&record.digest), Value::str(&to_iso(record.checked_at))),
        None => (Value::Null, Value::Null, Value::Null),
    };
    json::object(vec![("client", Value::str(image.client)), ("image", Value::str(&format!("docker.io/sbx/{}", image.repo))), ("tag", tag), ("digest", digest), ("checked_at", checked)])
}

/// `sandbox refresh`: asks the registry for every enabled client's image and records what it chose. A client whose
/// refresh fails keeps its record; the answer names each failure in "warnings".
pub fn refresh_command(runtime: &Runtime) -> Result<Value, GatewayError> {
    let path = records_path(&runtime.env)?;
    let mut warnings = Vec::new();
    let mut records = read_records(runtime.files.as_ref(), &path).unwrap_or_else(|reason| {
        warnings.push(format!("{RECORDS_FILE} cannot be read ({reason}); only the refreshed images are recorded"));
        BTreeMap::new()
    });
    let mut clients = Vec::new();
    let mut refreshed = false;
    for image in IMAGES.iter().filter(|image| image.enabled) {
        match refresh_one(runtime.process.as_ref(), runtime.clock.as_ref(), image) {
            Ok(record) => {
                clients.push(view(image, Some(&record)));
                records.insert(image.client.to_string(), record);
                refreshed = true;
            }
            Err(reason) => match records.get(image.client) {
                Some(record) => {
                    warnings.push(format!("{}: refresh failed ({reason}); the {} record from {} is kept", image.client, record.tag, to_iso(record.checked_at)));
                    clients.push(view(image, Some(record)));
                }
                None => {
                    warnings.push(format!("{}: refresh failed ({reason}) and no record is kept", image.client));
                    clients.push(view(image, None));
                }
            },
        }
    }
    if refreshed {
        if let Err(reason) = write_records(runtime.files.as_ref(), &path, &records) {
            warnings.push(format!("{RECORDS_FILE} was not written ({reason})"));
        }
    }
    Ok(result(clients, warnings))
}

/// `sandbox status`: the records the file holds for every enabled client, with no network.
pub fn status_command(runtime: &Runtime) -> Result<Value, GatewayError> {
    let path = records_path(&runtime.env)?;
    let mut warnings = Vec::new();
    let records = read_records(runtime.files.as_ref(), &path).unwrap_or_else(|reason| {
        warnings.push(format!("{RECORDS_FILE} cannot be read ({reason})"));
        BTreeMap::new()
    });
    let clients = IMAGES.iter().filter(|image| image.enabled).map(|image| view(image, records.get(image.client))).collect();
    Ok(result(clients, warnings))
}

fn result(clients: Vec<Value>, warnings: Vec<String>) -> Value {
    json::object(vec![("clients", Value::Arr(clients)), ("warnings", Value::Arr(warnings.iter().map(|text| Value::str(text)).collect()))])
}

/// Whether every client failed to refresh and has no record: the one case `sandbox refresh` exits 1 for.
pub fn all_failed(result: &Value) -> bool {
    let Some(Value::Arr(clients)) = result.as_object().and_then(|object| object.get("clients")) else {
        return true;
    };
    clients.iter().all(|view| view.as_object().and_then(|object| object.get("tag")) == Some(&Value::Null))
}

/// The plain-text table of `sandbox refresh` or `sandbox status`: one line per client, then the warnings.
pub fn table(result: &Value) -> String {
    let object = result.as_object();
    let list = |key: &str| object.and_then(|object| object.get(key)).and_then(|value| if let Value::Arr(items) = value { Some(items.as_slice()) } else { None }).unwrap_or_default();
    let (clients, warnings) = (list("clients"), list("warnings"));
    let mut lines = Vec::new();
    for view in clients {
        let field = |key: &str| match view.as_object().and_then(|object| object.get(key)) {
            Some(Value::Str(text)) => text.clone(),
            _ => "none".to_string(),
        };
        lines.push(format!("{}: {} {} checked {}", field("client"), field("tag"), field("digest"), field("checked_at")));
    }
    lines.extend(warnings.iter().filter_map(Value::as_str).map(|text| format!("warning: {text}")));
    format!("{}\n", lines.join("\n"))
}
