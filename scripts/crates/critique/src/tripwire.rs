//! The third layer of read-only enforcement (#167): what a critic could have written to, compared before and after it
//! ran. The node runner's `domain/readonly.ts` and `application/tripwire.ts`.
//!
//! Each critic works in its own copy of the packet, so a critic that writes to its packet is caught without voiding the
//! others. Around each critic the tripwire also watches the run's packet, which every copy is made from, and the
//! repository's working tree.

use crate::error::CritiqueError;
use crate::git::PorcelainEntry;
use crate::manifest::RunManifest;
use crate::runtime::Runtime;
use model_gateway::sha256::sha256_hex;
use crate::paths::join;
use std::collections::{BTreeMap, BTreeSet};

/// Every file of a packet, by path, with a hash of its content.
pub type PacketState = BTreeMap<String, String>;

/// One path `git status` lists: its status code, and a hash of what is at the path now.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct WorkingTreeEntry {
    pub code: String,
    pub hash: String,
}

/// The working tree as `git status` sees it: only the paths it lists.
pub type WorkingTreeState = BTreeMap<String, WorkingTreeEntry>;

/// The paths of either state, sorted as JavaScript sorts strings, that `same` tells apart.
fn changed_paths<V>(before: &BTreeMap<String, V>, after: &BTreeMap<String, V>, same: impl Fn(Option<&V>, Option<&V>) -> bool) -> Vec<String> {
    let paths: BTreeSet<&String> = before.keys().chain(after.keys()).collect();
    let mut changed: Vec<String> = paths.into_iter().filter(|path| !same(before.get(*path), after.get(*path))).cloned().collect();
    changed.sort_by_key(|path| path.encode_utf16().collect::<Vec<u16>>());
    changed
}

/// Every change between two states of a packet, as `<packet>: <added|changed|removed> <path>`, by path.
pub fn packet_changes(packet: &str, before: &PacketState, after: &PacketState) -> Vec<String> {
    changed_paths(before, after, |a, b| a == b)
        .into_iter()
        .map(|path| {
            let change = if !before.contains_key(&path) {
                "added"
            } else if !after.contains_key(&path) {
                "removed"
            } else {
                "changed"
            };
            format!("{packet}: {change} {path}")
        })
        .collect()
}

/// Every path whose status or content changed, as `working tree: <status> <path>` with the status after the critic ran
/// (`clean` when git no longer lists it).
pub fn working_tree_changes(before: &WorkingTreeState, after: &WorkingTreeState) -> Vec<String> {
    changed_paths(before, after, |a, b| a == b)
        .into_iter()
        .map(|path| format!("working tree: {} {path}", after.get(&path).map_or("clean", |entry| model_gateway::text::js_trim(&entry.code))))
        .collect()
}

struct States {
    packet: PacketState,
    run_packet: PacketState,
    working_tree: WorkingTreeState,
}

/// One critic's tripwire, set before it starts.
pub struct ArmedTripwire {
    packet_dir: String,
    before: States,
}

/// The run's tripwire: the packet as prepare froze it, path by path.
pub struct ReadOnlyTripwire {
    packet_dir: String,
    repo_root: String,
    frozen: Vec<(String, String)>,
}

impl ReadOnlyTripwire {
    /// Reads the run's packet, which must still be what prepare froze.
    pub fn load(runtime: &Runtime, manifest: &RunManifest) -> Result<ReadOnlyTripwire, CritiqueError> {
        let mut frozen = Vec::new();
        for file in &manifest.packet_files {
            let text = runtime.files.read_text(&join(&[&manifest.packet_dir, &file.path])).map_err(CritiqueError::from_fs)?;
            if sha256_hex(text.as_bytes()) != file.sha256 {
                return Err(CritiqueError::new("invalid-run", format!("{} in the packet no longer matches the manifest", file.path)));
            }
            frozen.push((file.path.clone(), text));
        }
        Ok(ReadOnlyTripwire { packet_dir: manifest.packet_dir.clone(), repo_root: manifest.repo_root.clone(), frozen })
    }

    /// Writes the critic's copy of the packet to `packet_dir` and records the state it starts from.
    pub fn arm(&self, runtime: &Runtime, packet_dir: &str) -> Result<ArmedTripwire, CritiqueError> {
        self.restore(runtime, packet_dir)?;
        Ok(ArmedTripwire { packet_dir: packet_dir.to_string(), before: self.states(runtime, packet_dir)? })
    }

    /// Compares everything the critic could have written to with its state when the tripwire was set. On any change
    /// it restores the packet, and returns the changes; it returns none when the critic left everything as it was.
    pub fn check(&self, runtime: &Runtime, armed: &ArmedTripwire) -> Result<Vec<String>, CritiqueError> {
        let after = self.states(runtime, &armed.packet_dir)?;
        let mut changes = packet_changes("packet", &armed.before.packet, &after.packet);
        changes.extend(packet_changes("run packet", &armed.before.run_packet, &after.run_packet));
        changes.extend(working_tree_changes(&armed.before.working_tree, &after.working_tree));
        if !changes.is_empty() {
            self.restore(runtime, &armed.packet_dir)?;
            self.restore(runtime, &self.packet_dir)?;
        }
        Ok(changes)
    }

    fn states(&self, runtime: &Runtime, packet_dir: &str) -> Result<States, CritiqueError> {
        Ok(States { packet: packet_state(runtime, packet_dir), run_packet: packet_state(runtime, &self.packet_dir), working_tree: self.working_tree_state(runtime)? })
    }

    /// The porcelain status, plus a hash of each listed file, so a critic that edits a file the owner had already
    /// changed is caught too.
    fn working_tree_state(&self, runtime: &Runtime) -> Result<WorkingTreeState, CritiqueError> {
        let entries = runtime.git.porcelain_status(&self.repo_root).map_err(CritiqueError::unexpected)?;
        Ok(entries.into_iter().map(|PorcelainEntry { code, path }| (path.clone(), WorkingTreeEntry { code, hash: fingerprint(runtime, &join(&[&self.repo_root, &path])) })).collect())
    }

    /// Puts back every frozen file and removes every other file.
    fn restore(&self, runtime: &Runtime, dir: &str) -> Result<(), CritiqueError> {
        for path in runtime.dirs.list_tree(dir) {
            if !self.frozen.iter().any(|(known, _)| *known == path) {
                runtime.files.remove(&join(&[dir, &path])).map_err(CritiqueError::from_fs)?;
            }
        }
        for (path, text) in &self.frozen {
            runtime.files.write_text(&join(&[dir, path]), text).map_err(CritiqueError::from_fs)?;
        }
        Ok(())
    }
}

fn packet_state(runtime: &Runtime, dir: &str) -> PacketState {
    runtime.dirs.list_tree(dir).into_iter().map(|path| (path.clone(), fingerprint(runtime, &join(&[dir, &path])))).collect()
}

/// A hash of what is at the path, or `unreadable`.
fn fingerprint(runtime: &Runtime, path: &str) -> String {
    runtime.dirs.read_bytes(path).map_or_else(|| "unreadable".to_string(), |bytes| sha256_hex(&bytes))
}
