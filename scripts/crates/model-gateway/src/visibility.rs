//! A repository's visibility on GitHub, which decides whether the family
//! allowlist applies to it: a public repository may send to every client, and a
//! private one only to what its allowlist entry lists.
//!
//! The lookup is `gh api repos/<owner>/<repo> --jq .visibility`. It fails closed:
//! a remote that is not on github.com, a missing or failing gh, and an answer
//! that is neither `public`, `private` nor `internal` all read as unknown, which
//! every caller treats as private. A known answer is kept for fifteen minutes in
//! the cache directory, so one delegation after another does not ask GitHub each
//! time; an unknown one is never kept.

use crate::git_remote::RemoteLocation;
use crate::json::{self, Object, Value};
use crate::ports::Runtime;
use crate::text::join;
use std::collections::HashMap;

/// What GitHub says about a repository's visibility.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Visibility {
    Public,
    /// Private or internal.
    Private,
    /// Not on github.com, or the lookup failed: read as private.
    Unknown,
}

impl Visibility {
    /// Only a repository GitHub says is public counts as public.
    pub fn is_public(self) -> bool {
        self == Visibility::Public
    }

    fn as_str(self) -> &'static str {
        match self {
            Visibility::Public => "public",
            Visibility::Private => "private",
            Visibility::Unknown => "unknown",
        }
    }
}

/// How long a known visibility is kept before GitHub is asked again.
pub const CACHE_TTL_MS: i64 = 15 * 60 * 1000;

/// How long the lookup may take before it counts as failed.
const LOOKUP_TIMEOUT_MS: u64 = 10_000;

/// Where known visibilities are kept: `$XDG_CACHE_HOME/model-gateway/repository-visibility.json`, or
/// `~/.cache/model-gateway/repository-visibility.json` when XDG_CACHE_HOME is unset or not an absolute path. `None`
/// when neither is known, and then nothing is kept.
pub fn visibility_cache_path(env: &HashMap<String, String>) -> Option<String> {
    let root = match env.get("XDG_CACHE_HOME") {
        Some(xdg) if xdg.starts_with('/') => xdg.clone(),
        _ => join(&[env.get("HOME")?, ".cache"]),
    };
    Some(join(&[&root, "model-gateway", "repository-visibility.json"]))
}

/// The repository's visibility: the cached answer while it is fresh, else what gh says now. Never an error.
pub fn repository_visibility(runtime: &Runtime, location: &RemoteLocation) -> Visibility {
    if location.host != "github.com" || location.owner.contains('/') {
        return Visibility::Unknown;
    }
    let key = format!("{}/{}/{}", location.host, location.owner, location.repo);
    let now = runtime.clock.now_ms();
    let cache = visibility_cache_path(&runtime.env);
    if let Some(cached) = cache.as_deref().and_then(|path| cached(runtime, path, &key, now)) {
        return cached;
    }
    let visibility = ask_github(runtime, location);
    if visibility != Visibility::Unknown {
        if let Some(path) = cache.as_deref() {
            remember(runtime, path, &key, visibility, now);
        }
    }
    visibility
}

fn ask_github(runtime: &Runtime, location: &RemoteLocation) -> Visibility {
    let path = format!("repos/{}/{}", location.owner, location.repo);
    match runtime.process.exec("gh", &["api", &path, "--jq", ".visibility"], Some(LOOKUP_TIMEOUT_MS)) {
        Ok(result) if result.exit_code == 0 && !result.timed_out => parse_visibility(&result.stdout),
        _ => Visibility::Unknown,
    }
}

/// gh's answer, read strictly: anything but the three words GitHub uses is unknown.
pub fn parse_visibility(stdout: &str) -> Visibility {
    match stdout.trim() {
        "public" => Visibility::Public,
        "private" | "internal" => Visibility::Private,
        _ => Visibility::Unknown,
    }
}

/// The cached visibility of `key`, while it is younger than the TTL; a cache that cannot be read is no cache.
fn cached(runtime: &Runtime, path: &str, key: &str, now: i64) -> Option<Visibility> {
    let repositories = read_cache(runtime, path)?;
    let entry = repositories.get(key)?.as_object()?;
    let checked = match entry.get("checked_at_ms")? {
        Value::Num(ms) => *ms as i64,
        _ => return None,
    };
    if checked > now || now - checked >= CACHE_TTL_MS {
        return None;
    }
    match entry.get("visibility")?.as_str()? {
        "public" => Some(Visibility::Public),
        "private" => Some(Visibility::Private),
        _ => None,
    }
}

fn read_cache(runtime: &Runtime, path: &str) -> Option<Object> {
    if !runtime.files.exists(path).ok()? {
        return None;
    }
    let parsed = json::parse(&runtime.files.read_text(path).ok()?).ok()?;
    let file = parsed.as_object()?;
    if file.get("version") != Some(&Value::Num(1.0)) {
        return None;
    }
    file.get("repositories")?.as_object().cloned()
}

/// Keeps a known visibility; a cache that cannot be written is left as it is.
fn remember(runtime: &Runtime, path: &str, key: &str, visibility: Visibility, now: i64) {
    let mut repositories = read_cache(runtime, path).unwrap_or_default();
    repositories.set(key, json::object(vec![("checked_at_ms", Value::Num(now as f64)), ("visibility", Value::str(visibility.as_str()))]));
    let file = json::object(vec![("version", Value::Num(1.0)), ("repositories", Value::Obj(repositories))]);
    let _ = runtime.files.mkdir_all(crate::text::dirname(path));
    let _ = runtime.files.write_atomic(path, &format!("{}\n", json::pretty(&file)));
}
