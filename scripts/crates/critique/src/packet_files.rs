//! What a packet reader hands back: the files it froze, before their hashes are taken. The node runner's
//! `UnfrozenFile` of `application/code-packet.ts`.

use crate::manifest::{LineRange, PacketWindow};
use crate::tree_sample::{LineRange as SampledRange, SampledWindow};
use model_gateway::json::Value;

/// The packet folder of the tree at the commit under review.
pub const TREE_DIR: &str = "tree";

/// One file to freeze into a packet, with its text.
#[derive(Clone, Debug, PartialEq)]
pub struct UnfrozenFile {
    pub kind: &'static str,
    pub path: String,
    pub source: String,
    pub text: String,
    /// Only for a tree file the packet cap cut short.
    pub window: Option<PacketWindow>,
}

impl UnfrozenFile {
    pub fn new(kind: &'static str, path: &str, source: &str, text: &str) -> UnfrozenFile {
        UnfrozenFile { kind, path: path.to_string(), source: source.to_string(), text: text.to_string(), window: None }
    }
}

/// What a reader came to: the packet's contents, or the refusal for a packet that cannot be frozen.
pub enum Packet<T> {
    Read(T),
    Refused(Value),
}

/// What prepare reports about a repository packet's tree.
#[derive(Clone, Debug, PartialEq)]
pub struct PreparedRepository {
    pub head_commit: String,
    /// How many sampled files of the tree were sent, in whole or in part.
    pub tree_files: usize,
    /// How many sampled files the cap left out, in whole or in part.
    pub dropped_files: usize,
    /// The bytes of every packet file.
    pub packet_bytes: usize,
    pub cap_bytes: f64,
}

/// What a repository packet (decision recovery, architecture review or a plan) holds beyond its files: the measures
/// its stage's depth signals read, each present only for the stages that measure it.
#[derive(Clone, Debug, PartialEq)]
pub struct RepositoryPacket {
    /// The reference of the artifact, which is no tracker item.
    pub reference: String,
    /// What depth is chosen from: the documents the packet holds, as one text.
    pub artifact_text: String,
    pub files: Vec<UnfrozenFile>,
    pub repository: PreparedRepository,
    /// How many lines of source the repository tracks, and how many context maps it holds.
    pub source_lines: Option<usize>,
    pub context_maps: Option<usize>,
    /// How many ADRs in force went in as references.
    pub adrs: Option<usize>,
    /// Only for a plan: how many steps it has, and how many of its migrate and contract steps touch stored data.
    pub steps: Option<usize>,
    pub stored_data_steps: Option<usize>,
}

fn frozen_range(range: &SampledRange) -> LineRange {
    LineRange { first: range.first as f64, last: range.last as f64 }
}

/// The window a sampled tree file records, as the manifest holds it.
pub fn frozen_window(window: &SampledWindow) -> PacketWindow {
    PacketWindow { last: window.last as f64, lines: window.lines as f64, ranges: window.ranges.as_ref().map(|ranges| ranges.iter().map(frozen_range).collect()) }
}
