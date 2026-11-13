//! The critic prompt over generated sources, packets, stages and toolsets, compared with what the node runner's
//! `domain/prompt.ts` assembles.

use crate::critic_tools::{CriticToolset, SearchServerLaunch, READ_ONLY_TOOLSET};
use crate::manifest::{CodeSlice, PacketFile};
use crate::packet_search::{SearchLimits, SearchToolName};
use crate::prompt::{assemble_prompt, PromptSources};
use model_gateway::json::{self, Value};

fn field<'a>(value: &'a Value, key: &str) -> &'a Value {
    value.as_object().and_then(|object| object.get(key)).unwrap_or_else(|| panic!("no {key}"))
}

fn text(value: &Value, key: &str) -> String {
    field(value, key).as_str().unwrap_or_else(|| panic!("{key} is not text")).to_string()
}

fn number(value: &Value, key: &str) -> f64 {
    match field(value, key) {
        Value::Num(number) => *number,
        _ => panic!("{key} is not a number"),
    }
}

fn items<'a>(value: &'a Value, key: &str) -> &'a [Value] {
    match field(value, key) {
        Value::Arr(items) => items,
        _ => panic!("{key} is not a list"),
    }
}

fn toolset(value: &Value) -> CriticToolset {
    match text(value, "kind").as_str() {
        "vibe-read" => READ_ONLY_TOOLSET,
        "agy" => CriticToolset::Agy { search: matches!(field(value, "search"), Value::Bool(true)) },
        _ => {
            let limits = field(value, "limits");
            let search = items(value, "search")
                .iter()
                .map(|name| match name.as_str() {
                    Some("grep") => SearchToolName::Grep,
                    Some("list") => SearchToolName::List,
                    _ => SearchToolName::Symbol,
                })
                .collect();
            let server = field(value, "server");
            CriticToolset::VibeSearch {
                search,
                limits: SearchLimits { max_matches: number(limits, "max_matches"), max_bytes: number(limits, "max_bytes") },
                server: SearchServerLaunch { command: text(server, "command"), args: items(server, "args").iter().map(|arg| arg.as_str().expect("an argument").to_string()).collect() },
            }
        }
    }
}

fn replay(index: usize, case: &Value) {
    let sources_value = field(case, "sources");
    let sources = PromptSources {
        preamble: text(sources_value, "preamble"),
        role: text(sources_value, "role"),
        lenses: items(sources_value, "lenses").iter().map(|lens| (text(lens, "name"), text(lens, "text"))).collect(),
        contract: text(sources_value, "contract"),
        schema: text(sources_value, "schema"),
    };
    let files: Vec<PacketFile> = items(case, "files")
        .iter()
        .map(|file| PacketFile { path: text(file, "path"), source: String::new(), kind: text(file, "kind"), sha256: String::new(), window: None })
        .collect();
    let slices: Vec<CodeSlice> = items(case, "slices")
        .iter()
        .map(|slice| CodeSlice { path: text(slice, "path"), first: number(slice, "first"), last: number(slice, "last"), reason: text(slice, "reason") })
        .collect();
    let prompt = assemble_prompt(&sources, &files, &text(case, "stage"), &slices, &toolset(field(case, "toolset")));
    let expected = field(case, "expected");
    assert_eq!(prompt.text, text(expected, "text"), "case {index}: the text differs\n  case: {}", json::compact(case));
    let hashes = field(expected, "prompt_hashes");
    let mine = &prompt.prompt_hashes;
    assert_eq!(
        [&mine.contract, &mine.preamble, &mine.role, &mine.schema].map(String::as_str),
        [text(hashes, "contract"), text(hashes, "preamble"), text(hashes, "role"), text(hashes, "schema")].each_ref().map(String::as_str),
        "case {index}: the hashes differ"
    );
    let lens_hashes = field(expected, "lens_hashes").as_object().expect("lens hashes").entries().into_iter().map(|(name, hash)| (name.to_string(), hash.as_str().expect("a hash").to_string())).collect::<Vec<_>>();
    let mut mine: Vec<(String, String)> = prompt.lens_hashes.clone();
    // A JavaScript object keeps a name that reads as an integer first, in ascending order; the writer of the run applies it.
    let mut keyed = json::Object::new();
    for (name, hash) in mine.drain(..) {
        keyed.set(&name, Value::str(&hash));
    }
    let mine: Vec<(String, String)> = keyed.entries().into_iter().map(|(name, hash)| (name.to_string(), hash.as_str().expect("a hash").to_string())).collect();
    assert_eq!(mine, lens_hashes, "case {index}: the lens hashes differ");
}

/// Replays the corpus; `CRITIQUE_PROMPT_CORPUS` names a bigger one.
#[test]
fn prompt_corpus() {
    let source = match std::env::var("CRITIQUE_PROMPT_CORPUS") {
        Ok(path) => std::fs::read_to_string(path).expect("the corpus file"),
        Err(_) => include_str!("../../tests/fixtures/prompt-corpus.json").to_string(),
    };
    let Ok(Value::Arr(cases)) = json::parse(&source) else { panic!("the corpus is not a JSON array") };
    assert!(cases.len() >= 100);
    for (index, case) in cases.iter().enumerate() {
        replay(index, case);
    }
}
