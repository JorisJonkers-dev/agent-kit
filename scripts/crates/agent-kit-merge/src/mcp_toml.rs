//! TOML client configs: Vibe's `config.toml` and `hooks.toml` (a rendered
//! `[[mcp_servers]]` or `[[hooks]]` fragment merged in) and Codex's
//! `config.toml` (one URL server added). Both go through `toml_edit`, which
//! keeps the owner's comments and layout the way `tomlkit` did.

use crate::{backup_once, finish, home, make_parent, repr_list, Env, Failure, Io};
use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use toml_edit::{value, ArrayOfTables, DocumentMut, Item, Table, Value};

pub fn vibe_default(env: Env, key: &str) -> PathBuf {
    let base = env("VIBE_HOME").filter(|base| !base.is_empty()).map(PathBuf::from).unwrap_or_else(|| home(env).join(".vibe"));
    base.join(if key == "hooks" { "hooks.toml" } else { "config.toml" })
}

/// A value compared by what it holds, not by how it is written.
#[derive(Debug, PartialEq)]
pub enum Plain {
    Text(String),
    Int(i64),
    Float(String),
    Flag(bool),
    Moment(String),
    List(Vec<Plain>),
    Map(BTreeMap<String, Plain>),
}

fn plain_value(item: &Value) -> Plain {
    match item {
        Value::String(text) => Plain::Text(text.value().clone()),
        Value::Integer(number) => Plain::Int(*number.value()),
        Value::Float(number) => Plain::Float(number.value().to_string()),
        Value::Boolean(flag) => Plain::Flag(*flag.value()),
        Value::Datetime(moment) => Plain::Moment(moment.value().to_string()),
        Value::Array(items) => Plain::List(items.iter().map(plain_value).collect()),
        Value::InlineTable(table) => Plain::Map(table.iter().map(|(k, v)| (k.to_string(), plain_value(v))).collect()),
    }
}

pub fn plain(item: &Item) -> Plain {
    match item {
        Item::Value(inner) => plain_value(inner),
        Item::Table(table) => Plain::Map(table.iter().map(|(k, v)| (k.to_string(), plain(v))).collect()),
        Item::ArrayOfTables(tables) => Plain::List(tables.iter().map(|t| plain(&Item::Table(t.clone()))).collect()),
        Item::None => Plain::Map(BTreeMap::new()),
    }
}

/// An entry's name the way `str(entry["name"])` shows it.
pub fn entry_name(table: &Table) -> String {
    match table.get("name") {
        None => "None".to_string(),
        Some(Item::Value(Value::String(text))) => text.value().clone(),
        Some(other) => other.to_string().trim().to_string(),
    }
}

/// The same content with no whitespace trivia, so a re-run writes byte-identical output
/// instead of growing blank lines. Inline tables become tables, as `tomlkit.item(dict)` has it.
fn rebuilt_value(item: &Value) -> Value {
    match item {
        Value::String(text) => Value::from(text.value().as_str()),
        Value::Integer(number) => Value::from(*number.value()),
        Value::Float(number) => Value::from(*number.value()),
        Value::Boolean(flag) => Value::from(*flag.value()),
        Value::Datetime(moment) => Value::from(*moment.value()),
        Value::Array(items) => Value::Array(items.iter().map(rebuilt_value).collect()),
        Value::InlineTable(table) => {
            let mut rebuilt = toml_edit::InlineTable::new();
            for (name, inner) in table.iter() {
                rebuilt.insert(name, rebuilt_value(inner));
            }
            Value::InlineTable(rebuilt)
        }
    }
}

pub fn rebuilt_item(item: &Item) -> Item {
    match item {
        Item::Value(Value::InlineTable(table)) => Item::Table(rebuilt_table(table.iter().map(|(k, v)| (k, Item::Value(v.clone()))))),
        Item::Value(inner) => value(rebuilt_value(inner)),
        Item::Table(table) => Item::Table(rebuilt_table(table.iter().map(|(k, v)| (k, v.clone())))),
        Item::ArrayOfTables(tables) => {
            let mut rebuilt = ArrayOfTables::new();
            for table in tables.iter() {
                rebuilt.push(rebuilt_table(table.iter().map(|(k, v)| (k, v.clone()))));
            }
            Item::ArrayOfTables(rebuilt)
        }
        Item::None => Item::None,
    }
}

fn rebuilt_table<'a>(members: impl Iterator<Item = (&'a str, Item)>) -> Table {
    let mut table = Table::new();
    for (name, item) in members {
        table.insert(name, rebuilt_item(&item));
    }
    table
}

fn entries(item: Option<&Item>) -> Vec<Table> {
    match item {
        Some(Item::ArrayOfTables(tables)) => tables.iter().cloned().collect(),
        _ => Vec::new(),
    }
}

fn parse(text: &str) -> Result<DocumentMut, Failure> {
    text.parse::<DocumentMut>().map_err(|e| Failure(e.to_string()))
}

/// The entries of `doc` named by name, as plain values.
fn by_name(doc: &DocumentMut, key: &str) -> BTreeMap<String, Plain> {
    entries(doc.get(key)).iter().map(|t| (entry_name(t), plain(&Item::Table(t.clone())))).collect()
}

/// What setup would be told if the file did not come back as written, or None.
pub fn verify(wanted: &[Table], dropped: &[String], back: &BTreeMap<String, Plain>) -> Option<String> {
    let missing: Vec<String> = wanted
        .iter()
        .filter(|s| !dropped.contains(&entry_name(s)) && back.get(&entry_name(s)) != Some(&plain(&Item::Table((*s).clone()))))
        .map(entry_name)
        .collect();
    let mut lingering: Vec<String> = dropped.iter().filter(|name| back.contains_key(*name)).cloned().collect();
    lingering.sort();
    lingering.dedup();
    (!missing.is_empty() || !lingering.is_empty()).then(|| format!("vibe-merge-mcp: FAILED, not as written: {}, still present: {}", repr_list(&missing), repr_list(&lingering)))
}

/// The document with `key` rebuilt as `kept` followed by `added` and nothing else of it kept.
fn replaced(doc: &mut DocumentMut, key: &str, tables: Vec<Table>) {
    doc.remove(key);
    if !tables.is_empty() {
        let mut servers = ArrayOfTables::new();
        for table in tables {
            servers.push(table);
        }
        doc.insert(key, Item::ArrayOfTables(servers));
    }
}

/// `vibe-merge-mcp.py FRAGMENT [--key mcp_servers|hooks] [--config PATH] [--remove NAME ...]`.
pub fn vibe(fragment: &Path, config: &Path, remove: &[String], key: &str, io: &mut Io) -> Result<i32, Failure> {
    let wanted = entries(parse(&fs::read_to_string(fragment)?)?.get(key));
    let managed: Vec<String> = wanted.iter().map(entry_name).collect();
    let mut doc = if config.is_file() { parse(&fs::read_to_string(config)?)? } else { DocumentMut::new() };
    let kept = entries(doc.get(key)).into_iter().filter(|s| !managed.contains(&entry_name(s)) && !remove.contains(&entry_name(s)));

    let tables: Vec<Table> = kept.chain(wanted.iter().filter(|w| !remove.contains(&entry_name(w))).cloned()).map(|t| rebuilt_table(t.iter().map(|(k, v)| (k, v.clone())))).collect();
    replaced(&mut doc, key, tables);

    backup_once(config)?;
    make_parent(config)?;
    fs::write(config, doc.to_string().trim_start_matches('\n'))?;

    let back = by_name(&parse(&fs::read_to_string(config)?)?, key);
    let mut counted: Vec<&String> = managed.iter().filter(|name| !remove.contains(name)).collect();
    counted.sort();
    counted.dedup();
    let success = format!("vibe-merge-mcp: {} managed {key} entries in {}", counted.len(), config.display());
    finish(verify(&wanted, remove, &back), success, io)
}

/// `codex-add-url-mcp.py CONFIG NAME URL`: `[mcp_servers.NAME]` with the URL only.
pub fn codex_add_url(config: &Path, name: &str, url: &str) -> Result<i32, Failure> {
    let mut doc = if config.is_file() { parse(&fs::read_to_string(config)?)? } else { DocumentMut::new() };
    if doc.get("mcp_servers").is_none() {
        let mut servers = Table::new();
        servers.set_implicit(true);
        doc.insert("mcp_servers", Item::Table(servers));
    }
    let mut server = Table::new();
    server.insert("url", value(url));
    match doc.get_mut("mcp_servers") {
        Some(Item::Table(servers)) => {
            servers.insert(name, Item::Table(server));
        }
        _ => return Err(Failure("mcp_servers is not a table".to_string())),
    }
    make_parent(config)?;
    fs::write(config, doc.to_string())?;
    Ok(0)
}
