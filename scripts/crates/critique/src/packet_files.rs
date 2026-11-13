//! What a packet reader hands back: the files it froze, before their hashes are taken. The node runner's
//! `UnfrozenFile` of `application/code-packet.ts`.

use crate::context_slices::ContextSlice;
use crate::tree_sample::{LineRange, SampledWindow};
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
    pub window: Option<FrozenWindow>,
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

/// A run of lines a tree file holds, as the manifest writes it. A range that is a context slice the sample took as it
/// was keeps the slice's own fields after the two numbers, in the node runner's key order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenRange {
    pub first: usize,
    pub last: usize,
    pub extra: Vec<(&'static str, String)>,
}

/// The lines of a file a packet sends, and how many the file has.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FrozenWindow {
    pub last: usize,
    pub lines: usize,
    /// Only when the file holds other lines than 1 to `last`.
    pub ranges: Option<Vec<FrozenRange>>,
}

fn frozen_range(range: &LineRange, slices: &[ContextSlice]) -> FrozenRange {
    FrozenRange { first: range.first, last: range.last, extra: range.origin.map(|at| slices[at].extra_fields()).unwrap_or_default() }
}

/// The window a sampled tree file records, as the manifest holds it; `slices` are the context slices the sample took.
pub fn frozen_window(window: &SampledWindow, slices: &[ContextSlice]) -> FrozenWindow {
    FrozenWindow { last: window.last, lines: window.lines, ranges: window.ranges.as_ref().map(|ranges| ranges.iter().map(|range| frozen_range(range, slices)).collect()) }
}
