//! code-nav's map as a ranking of the repository's files. The node runner's `domain/ranking.ts`.

use std::collections::HashMap;

/// A file of code-nav's map: a repository path and the references to the symbols it declares.
#[derive(Clone, Debug, PartialEq)]
pub struct RankedFile {
    pub path: String,
    pub refs: f64,
}

/// A symbol of code-nav's map, most referenced first.
#[derive(Clone, Debug, PartialEq)]
pub struct RankedSymbol {
    pub path: String,
    pub line: f64,
    pub kind: String,
    pub name: String,
    pub refs: f64,
}

/// The repository's files ranked by code-nav's map, most referenced first; or why there is no ranking, which a
/// packet records instead of failing.
#[derive(Clone, Debug, PartialEq)]
pub enum RepoRanking {
    Ranked { files: Vec<RankedFile>, symbols: Vec<RankedSymbol> },
    Unavailable { detail: String },
}

/// What a packet's facts index says about how code-nav's map shaped the tree sample.
pub fn ranking_note(ranking: &RepoRanking) -> String {
    match ranking {
        RepoRanking::Ranked { files, .. } => format!(
            "code-nav's repository map ranked {} files by how often their symbols are referenced; the tree sample takes those files first within their tier, in that order.",
            files.len()
        ),
        RepoRanking::Unavailable { detail } => format!("code-nav's repository map was not used ({detail}), so the tree sample keeps its path order."),
    }
}

/// The ranked paths, or none when there is no ranking.
pub fn ranked_paths(ranking: Option<&RepoRanking>) -> Vec<String> {
    match ranking {
        Some(RepoRanking::Ranked { files, .. }) => files.iter().map(|file| file.path.clone()).collect(),
        _ => Vec::new(),
    }
}

/// A path's place in the ranking, or past every ranked path (`None`).
pub fn rank_index(ranked: &[String]) -> impl Fn(&str) -> Option<usize> {
    let mut places: HashMap<String, usize> = HashMap::new();
    for (place, path) in ranked.iter().enumerate() {
        places.insert(path.clone(), place);
    }
    move |path| places.get(path).copied()
}
