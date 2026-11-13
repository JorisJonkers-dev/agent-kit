//! Routing's `packet`: how big a repository packet may be.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::triage::matches_glob;

/// The repo map's cap for a routing file that states none.
const DEFAULT_REPO_MAP_CAP_BYTES: f64 = 20_480.0;
/// The manifest's cap for a routing file that states none.
const DEFAULT_MANIFEST_CAP_BYTES: f64 = 30_720.0;

/// Routing's `packet`: how big a repository packet may be. The cap counts every packet file; the facts cap counts
/// the computed fact files, beside `facts/INDEX.md`, which is always sent; a sampled file is sent in windows of
/// `window_lines` lines. `exclude` holds the globs of the paths never sent, per exclusion kind, in the order
/// JavaScript enumerates them. Past `manifest_cap_bytes`, `tree/MANIFEST.tsv` counts its least-ranked paths per
/// directory instead of listing them.
#[derive(Clone, Debug, PartialEq)]
pub struct PacketLimits {
    pub cap_bytes: f64,
    pub facts_cap_bytes: f64,
    /// How big a repository packet's `tree/MANIFEST.tsv` may be before it counts paths per directory.
    pub manifest_cap_bytes: f64,
    /// How big a code packet's `tree/REPO-MAP.md` may be, inside the cap.
    pub repo_map_cap_bytes: f64,
    pub window_lines: f64,
    pub exclude: Vec<(String, Vec<String>)>,
}

/// ADR-0004's starting limits, for a routing file that states none; the phase-0 live check sets the final ones in
/// routing.
pub fn default_packet_limits() -> PacketLimits {
    PacketLimits {
        cap_bytes: 153_600.0,
        exclude: vec![("generated".to_string(), Vec::new()), ("lock".to_string(), Vec::new()), ("vendored".to_string(), Vec::new())],
        facts_cap_bytes: 30_720.0,
        manifest_cap_bytes: DEFAULT_MANIFEST_CAP_BYTES,
        repo_map_cap_bytes: DEFAULT_REPO_MAP_CAP_BYTES,
        window_lines: 400.0,
    }
}

/// Reads routing's `packet`; a cap it leaves out keeps its default.
pub fn read_packet_limits(fields: &Fields<'_>) -> Result<PacketLimits, ShapeError> {
    let exclude = fields.object("exclude")?;
    Ok(PacketLimits {
        cap_bytes: fields.positive_integer("cap_bytes")?,
        exclude: vec![
            ("generated".to_string(), exclude.strings("generated")?),
            ("lock".to_string(), exclude.strings("lock")?),
            ("vendored".to_string(), exclude.strings("vendored")?),
        ],
        facts_cap_bytes: fields.positive_integer("facts_cap_bytes")?,
        manifest_cap_bytes: if fields.has("manifest_cap_bytes") { fields.positive_integer("manifest_cap_bytes")? } else { DEFAULT_MANIFEST_CAP_BYTES },
        repo_map_cap_bytes: if fields.has("repo_map_cap_bytes") { fields.positive_integer("repo_map_cap_bytes")? } else { DEFAULT_REPO_MAP_CAP_BYTES },
        window_lines: fields.positive_integer("window_lines")?,
    })
}

/// Why a tracked path is never sent: it is vendored, generated, or a lock file, in the order the first match is taken.
pub const EXCLUSION_KINDS: [&str; 3] = ["vendored", "generated", "lock"];

/// Why a path is never sent: its kind, and the routing glob it matched or `marker` for a generated-file marker.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Exclusion {
    pub kind: &'static str,
    pub rule: String,
}

/// How many leading lines may carry a generated-file marker.
const MARKER_LINES: usize = 5;

/// Whether a file says in its first lines that a tool generated it (`@generated`, "Do not edit").
pub fn is_generated(text: &str) -> bool {
    let head = text.split('\n').take(MARKER_LINES).collect::<Vec<_>>().join("\n").to_ascii_lowercase();
    head.contains("@generated") || head.contains("do not edit")
}

/// Why a path, with its text when it is known, is never sent: the first exclusion glob it matches, else a
/// generated-file marker in its first lines. A glob JavaScript cannot build a pattern from is an `Err` with its message.
pub fn exclusion_of(path: &str, text: Option<&str>, limits: &PacketLimits) -> Result<Option<Exclusion>, String> {
    for kind in EXCLUSION_KINDS {
        let globs = limits.exclude.iter().find(|(name, _)| name == kind).map(|(_, globs)| globs.as_slice()).unwrap_or_default();
        for glob in globs {
            if matches_glob(path, glob)? {
                return Ok(Some(Exclusion { kind, rule: glob.clone() }));
            }
        }
    }
    Ok(text.filter(|text| is_generated(text)).map(|_| Exclusion { kind: "generated", rule: "marker".to_string() }))
}

/// What an exclusion says about its file, for a line of the diff or the facts index.
pub fn exclusion_reason(exclusion: &Exclusion) -> String {
    if exclusion.rule == "marker" {
        "a generated file: its first lines say a tool generated it".to_string()
    } else {
        format!("a {} file: it matches the routing glob `{}`", exclusion.kind, exclusion.rule)
    }
}

/// A tracked path that is not excluded, with its text, or `None` for a binary file.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct TrackedFile {
    pub path: String,
    pub text: Option<String>,
}
