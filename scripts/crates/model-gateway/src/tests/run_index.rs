//! Tests for the global run index: concurrent writers, rotation, retention of rotated files, and the fold.

use super::scenarios::{FixedClock, Sandbox};
use crate::adapters::fs::SystemFiles;
use crate::ports::Files;
use crate::run_index::{fold, IndexEntry, IndexKind, RunIndex, KEEP_ROTATED};

fn entry(kind: IndexKind, run_id: &str, ts_ms: i64) -> IndexEntry {
    IndexEntry {
        kind,
        ts_ms,
        run_id: run_id.to_string(),
        run_dir: format!("/runs/{run_id}"),
        workflow: "w".into(),
        client: "pi".into(),
        model: "m".into(),
        family: "f".into(),
        mode: "read-only".into(),
        gateway_pid: (kind == IndexKind::Started).then_some(7),
        status: (kind == IndexKind::Finished).then(|| "ok".to_string()),
        reason: None,
    }
}

#[test]
fn many_writers_at_once_lose_no_line_and_tear_none() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let path = format!("{}/run-index.jsonl", sandbox.text());
    let (writers, each) = (8, 250);
    std::thread::scope(|scope| {
        for writer in 0..writers {
            let (files, clock, path) = (&files, &clock, &path);
            scope.spawn(move || {
                let index = RunIndex::new(files, clock, path);
                for number in 0..each {
                    index.record(&entry(IndexKind::Started, &format!("w{writer}-{number}"), 1_795_000_000_000));
                }
            });
        }
    });
    let text = std::fs::read_to_string(&path).expect("the index");
    assert_eq!(text.lines().count(), writers * each);
    let read = RunIndex::new(&files, &clock, &path).read();
    assert_eq!(read.len(), writers * each, "a line was torn or lost");
    assert_eq!(fold(&read).len(), writers * each);
}

#[test]
fn writers_that_rotate_the_index_at_once_still_lose_no_line() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let path = format!("{}/run-index.jsonl", sandbox.text());
    let (writers, each) = (6, 120);
    std::thread::scope(|scope| {
        for writer in 0..writers {
            let (files, clock, path) = (&files, &clock, &path);
            scope.spawn(move || {
                let mut index = RunIndex::new(files, clock, path);
                index.rotate_bytes = 4_000;
                index.keep = 10_000;
                for number in 0..each {
                    index.record(&entry(IndexKind::Started, &format!("w{writer}-{number}"), 1_795_000_000_000));
                }
            });
        }
    });
    let index = RunIndex::new(&files, &clock, &path);
    assert!(index.rotated_files().len() > 1, "the index never rotated");
    assert_eq!(index.read().len(), writers * each);
}

#[test]
fn a_full_index_is_rotated_and_only_the_newest_rotated_files_stay() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let path = format!("{}/run-index.jsonl", sandbox.text());
    let mut kept = Vec::new();
    for round in 0..(KEEP_ROTATED + 3) {
        // A clock per round, so every rotated file has a name of its own and they sort by age.
        let clock = FixedClock(1_795_000_000_000 + round as i64);
        let mut index = RunIndex::new(&files, &clock, &path);
        index.rotate_bytes = 1;
        index.record(&entry(IndexKind::Started, &format!("r{round}"), 1_795_000_000_000));
        kept.push(format!("r{round}"));
    }
    let clock = FixedClock(1_795_000_000_100);
    let index = RunIndex::new(&files, &clock, &path);
    let rotated = index.rotated_files();
    assert_eq!(rotated.len(), KEEP_ROTATED);
    assert!(rotated.iter().all(|(_, modified)| modified.is_some()));
    // The oldest are gone, the newest all read, oldest first.
    let ids: Vec<String> = index.read().into_iter().map(|entry| entry.run_id).collect();
    assert_eq!(ids, kept[kept.len() - KEEP_ROTATED - 1..].to_vec());
    // The index itself is not counted among its rotated files.
    assert!(rotated.iter().all(|(name, _)| !name.ends_with("/run-index.jsonl")));
}

#[test]
fn a_line_that_is_not_an_entry_is_skipped_and_the_fold_joins_a_runs_lines() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    let path = format!("{}/run-index.jsonl", sandbox.text());
    files.append(&path, "not json\n{\"kind\":\"started\"}\n{\"kind\":\"nope\",\"ts\":\"2026-11-18T11:06:40.000Z\",\"run_id\":\"x\",\"run_dir\":\"d\"}\n[1]\n").expect("garbage");
    let index = RunIndex::new(&files, &clock, &path);
    for line in [
        entry(IndexKind::Finished, "early", 1_000),
        entry(IndexKind::Started, "a", 1_795_000_000_000),
        entry(IndexKind::Started, "a", 1_795_000_000_001),
        entry(IndexKind::Started, "b", 1_795_000_000_002),
        entry(IndexKind::Started, "c", 1_795_000_000_003),
        entry(IndexKind::Finished, "a", 1_795_000_000_010),
        entry(IndexKind::Finished, "a", 1_795_000_000_020),
        entry(IndexKind::Removed, "c", 1_795_000_000_030),
    ] {
        index.record(&line);
    }
    let runs = fold(&index.read());
    assert_eq!(runs.iter().map(|run| run.run_id.as_str()).collect::<Vec<_>>(), ["a", "b"]);
    assert_eq!(runs[0].started_ms, 1_795_000_000_000);
    assert_eq!(runs[0].finished, Some(("ok".to_string(), None, 1_795_000_000_010)));
    assert_eq!((runs[1].finished.clone(), runs[1].gateway_pid), (None, Some(7)));
}

#[test]
fn an_index_that_cannot_be_written_or_listed_is_an_empty_one() {
    let sandbox = Sandbox::new();
    let files = SystemFiles { cwd: sandbox.text() };
    let clock = FixedClock(1_795_000_000_000);
    // A directory where the file should be: nothing can be appended, and nothing is read.
    let path = format!("{}/run-index.jsonl", sandbox.text());
    std::fs::create_dir(&path).expect("a directory");
    let index = RunIndex::new(&files, &clock, &path);
    index.record(&entry(IndexKind::Started, "a", 1));
    assert!(index.read().is_empty());
    // A file system that cannot rename or list leaves the index as it is.
    let bare = super::run_events::Bare;
    let mut index = RunIndex::new(&bare, &clock, "/nowhere/run-index.jsonl");
    index.rotate_bytes = 0;
    index.record(&entry(IndexKind::Started, "a", 1));
    assert!(index.rotated_files().is_empty());
}
