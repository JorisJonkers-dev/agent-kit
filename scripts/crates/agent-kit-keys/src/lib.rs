//! Get the workstation's tokens and keys, store them, and wire them into the shell.
//!
//! `setup-workstation.sh` runs this. `ensure` asks only for what is missing or
//! no longer works: it signs in to auth-api once (password, then TOTP), grants
//! the missing services when you are an admin, mints each service token, checks
//! every value against its own host, and stores it in the one secret store,
//! through `agent-kit-secret`: the macOS Keychain, or the Secret Service on
//! Linux. There is no plain-file store: a missing store stops the run.
//! `write-env` renders `~/.config/agent-kit/env.sh`, which reads them back in
//! every shell; the Private Memory token is opened only by `vibe-private`, and
//! the Study account key is read only by `vibe-study`.
//!
//! The LiteLLM agent keys (`mint: litellm`) are minted on the proxy itself with
//! a one-hour management key that is never stored: `LITELLM_ADMIN_TOKEN` when
//! set, else minted in process from `LITELLM_MASTER_KEY` (the environment, or
//! the secret store under that name), else asked for once. It makes or updates
//! each route's team, which holds the budget, deletes the old shared key of
//! the route, and mints this host's own key as `<agent>@<host>` into the team.
//!
//! `status` reports without asking anything. `revoke NAME` signs in and revokes
//! this host's token for NAME (its recorded id), so the host refuses it from
//! then on; the next `ensure` sees the stored value refused and mints a fresh one.
//!
//! Secrets never reach argv or output: the store gets them on stdin and curl
//! gets its request on stdin.

mod auth;
mod check;
mod http;
mod litellm;
mod store;

use auth::Auth;
use check::{check, gate_store, gated, state};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::ffi::OsString;
use std::io::{BufRead, Write};
use std::path::{Path, PathBuf};
use std::process::Command;
use store::Store;

pub const NAME: &str = env!("CARGO_PKG_NAME");

const SOURCE_LINE: &str =
    "[ -f \"$HOME/.config/agent-kit/env.sh\" ] && . \"$HOME/.config/agent-kit/env.sh\"  # agent-kit";

/// Why a step did not finish.
#[derive(Debug)]
pub enum Fail {
    /// A host did not answer at all.
    Unreachable(String),
    /// The proxy refused the management key itself: no route can be minted with it.
    Refused(String),
    /// The secret store or the table is unusable: the run stops.
    Stop(String),
}

impl Fail {
    /// The message of an unreachable host; any other failure passes through.
    pub fn unreachable(self) -> R<String> {
        match self {
            Fail::Unreachable(message) => Ok(message),
            other => Err(other),
        }
    }

    /// The message of a refusal; any other failure passes through.
    pub fn refused(self) -> R<String> {
        match self {
            Fail::Refused(message) => Ok(message),
            other => Err(other),
        }
    }
}

pub type R<T> = Result<T, Fail>;

/// Everything outside the process the run touches, so the tests can supply it.
pub struct Ctx {
    pub env: HashMap<String, String>,
    pub home: PathBuf,
    pub input: Box<dyn BufRead>,
    pub tty: bool,
    pub out: Box<dyn Write>,
    pub err: Box<dyn Write>,
    pub host: String,
    pub exe: PathBuf,
}

impl Ctx {
    pub fn say(&mut self, message: &str) {
        let _ = writeln!(self.err, "keys: {message}");
    }

    pub fn host_name(&self) -> String {
        let short = self.host.split('.').next().unwrap_or("").to_lowercase();
        if short.is_empty() {
            "host".to_string()
        } else {
            short
        }
    }
}

pub fn config_dir(ctx: &Ctx) -> PathBuf {
    ctx.home.join(".config").join("agent-kit")
}

/// The value of a string field, or "".
pub fn field<'a>(value: &'a Value, key: &str) -> &'a str {
    value.get(key).and_then(Value::as_str).unwrap_or("")
}

/// Python truthiness of a JSON value.
pub fn truthy(value: &Value) -> bool {
    match value {
        Value::Null => false,
        Value::Bool(b) => *b,
        Value::Number(n) => n.as_f64() != Some(0.0),
        Value::String(s) => !s.is_empty(),
        Value::Array(a) => !a.is_empty(),
        Value::Object(o) => !o.is_empty(),
    }
}

/// A field that is present and truthy.
pub fn has(value: &Value, key: &str) -> bool {
    value.get(key).is_some_and(truthy)
}

/// A scalar as text, the way Python's `str` shows it.
pub fn id_text(value: &Value) -> String {
    match value {
        Value::String(s) => s.clone(),
        Value::Null => String::new(),
        other => other.to_string(),
    }
}

/// `[A-Za-z0-9._~+/=-]+`: what a header can carry safely.
pub fn is_safe(value: &str) -> bool {
    !value.is_empty()
        && value
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._~+/=-".contains(c))
}

/// One answer from the terminal; EOF counts as no answer.
pub fn ask(ctx: &mut Ctx, prompt: &str, secret: bool) -> String {
    let _ = write!(ctx.err, "{prompt}");
    let _ = ctx.err.flush();
    let hide = secret && ctx.tty;
    if hide {
        let _ = Command::new("stty").arg("-echo").status();
    }
    let mut line = String::new();
    let _ = ctx.input.read_line(&mut line);
    if hide {
        let _ = Command::new("stty").arg("echo").status();
        let _ = writeln!(ctx.err);
    }
    line.trim().to_string()
}

fn secrets(table: &[Value]) -> Vec<&Value> {
    table
        .iter()
        .filter(|e| e.get("value").is_none() && !has(e, "retired"))
        .collect()
}

fn env_of(entry: &Value) -> R<&str> {
    match field(entry, "env") {
        "" => Err(Fail::Stop("a table entry has no env".to_string())),
        name => Ok(name),
    }
}

/// Drop values the registry no longer manages from the store.
fn retire(ctx: &mut Ctx, table: &[Value], keys: &Store) -> R<()> {
    for entry in table.iter().filter(|e| has(e, "retired")) {
        let env = env_of(entry)?;
        if !keys.get(ctx, env)?.is_empty() {
            keys.delete(ctx, env)?;
            ctx.say(&format!("{env}: retired, removed from the {}", keys.name));
        }
    }
    Ok(())
}

// --- token ids: not secrets, an id only lets a signed-in owner revoke the token it names.

fn ids_path(ctx: &Ctx) -> PathBuf {
    config_dir(ctx).join("token-ids.json")
}

fn load_ids(ctx: &Ctx) -> BTreeMap<String, String> {
    let text = std::fs::read_to_string(ids_path(ctx)).unwrap_or_default();
    serde_json::from_str(&text).unwrap_or_default()
}

fn write_file(path: &Path, text: &str) -> R<()> {
    let parent = path.parent().unwrap_or(path);
    let done = std::fs::create_dir_all(parent).and_then(|()| std::fs::write(path, text));
    done.map_err(|e| Fail::Stop(format!("cannot write {}: {e}", path.display())))
}

fn save_ids(ctx: &Ctx, ids: &BTreeMap<String, String>) -> R<()> {
    let text = serde_json::to_string_pretty(ids).unwrap_or_default() + "\n";
    write_file(&ids_path(ctx), &text)
}

/// Record the new token's id and revoke the one it replaces.
fn replaced(
    ctx: &mut Ctx,
    auth: &Auth,
    ids: &mut BTreeMap<String, String>,
    var: &str,
    new_id: &str,
) -> R<()> {
    let old = ids.get(var).cloned();
    if !new_id.is_empty() {
        ids.insert(var.to_string(), new_id.to_string());
        save_ids(ctx, ids)?;
    }
    if let Some(old) = old.filter(|old| old != new_id) {
        if auth.revoke(ctx, &old)? {
            ctx.say(&format!("{var}: revoked the token it replaces"));
        } else {
            ctx.say(&format!(
                "{var}: could not revoke the token it replaces (id {old}); revoke it on the admin page"
            ));
        }
    }
    Ok(())
}

// --- env.sh

fn write_env(ctx: &mut Ctx, table: &[Value], keys: &Store) -> R<PathBuf> {
    let mut lines: Vec<String> = vec![
        "# managed by agent-kit setup-workstation.sh; rewritten on every run.".into(),
        format!(
            "# Reads the workstation's tokens and keys from the {}. A value already",
            keys.name
        ),
        "# in the environment wins. An `export: false` value is never exported: the".into(),
        "# Private Memory token is opened only by vibe-private (through agent-kit-private,".into(),
        "# which asks for its passphrase), and vibe-study reads the Study account key".into(),
        "# with _agent_kit_secret itself.".into(),
        keys.shell_reader().into(),
    ];
    for entry in table.iter().filter(|e| !has(e, "retired")) {
        let var = env_of(entry)?;
        if let Some(value) = entry.get("value") {
            let value = id_text(value);
            lines.push(format!(
                "[ -n \"${{{var}:-}}\" ] || export {var}=\"{value}\""
            ));
        } else if entry.get("export").is_none_or(truthy) {
            lines.push(format!(
                "if [ -z \"${{{var}:-}}\" ]; then _agent_kit_v=\"$(_agent_kit_secret {var})\"; \
                 if [ -n \"$_agent_kit_v\" ]; then export {var}=\"$_agent_kit_v\"; fi; fi"
            ));
        }
    }
    lines.push("unset _agent_kit_v".into());
    // A shell that sourced an older env.sh still has its vibe wrapper; drop that
    // one only, never a vibe function of your own.
    lines.push(
        "case \"$(typeset -f vibe 2>/dev/null)\" in *\"agent-kit-private exec vibe\"*) unset -f vibe ;; esac".into(),
    );
    let path = config_dir(ctx).join("env.sh");
    write_file(&path, &(lines.join("\n") + "\n"))?;
    Ok(path)
}

fn shell_rc(ctx: &Ctx) -> Option<PathBuf> {
    let shell = ctx.env.get("SHELL").cloned().unwrap_or_default();
    match shell.rsplit('/').next().unwrap_or("") {
        "zsh" => {
            let dir = ctx
                .env
                .get("ZDOTDIR")
                .filter(|d| !d.is_empty())
                .map(PathBuf::from);
            Some(dir.unwrap_or_else(|| ctx.home.clone()).join(".zshrc"))
        }
        "bash" => Some(ctx.home.join(".bashrc")),
        _ => None,
    }
}

fn wired(rc: &Option<PathBuf>) -> bool {
    rc.as_ref()
        .and_then(|rc| std::fs::read_to_string(rc).ok())
        .is_some_and(|text| text.contains(".config/agent-kit/env.sh"))
}

fn wire_shell(ctx: &mut Ctx) -> R<()> {
    let rc = shell_rc(ctx);
    if wired(&rc) {
        return Ok(());
    }
    let Some(rc) = rc else {
        ctx.say(&format!(
            "add this line to your shell's startup file: {SOURCE_LINE}"
        ));
        return Ok(());
    };
    let prompt = format!(
        "Load the tokens in every new shell by adding one line to {}? [Y/n] ",
        rc.display()
    );
    let answer = ask(ctx, &prompt, false);
    if matches!(answer.to_lowercase().as_str(), "" | "y" | "yes") {
        let mut text = std::fs::read_to_string(&rc).unwrap_or_default();
        text.push_str(&format!("\n{SOURCE_LINE}\n"));
        write_file(&rc, &text)?;
        ctx.say(&format!(
            "added the agent-kit line to {}; open a new shell to pick it up",
            rc.display()
        ));
    } else {
        ctx.say(&format!(
            "left {} alone; to load the tokens, add: {SOURCE_LINE}",
            rc.display()
        ));
    }
    Ok(())
}

// --- LiteLLM

/// Mint the missing LiteLLM keys, and converge the teams whenever an admin token is at hand.
fn ensure_litellm(ctx: &mut Ctx, table: &[Value], todo: &BTreeSet<String>, keys: &Store) -> R<()> {
    let entries: Vec<&Value> = secrets(table)
        .into_iter()
        .filter(|e| field(e, "mint") == "litellm")
        .collect();
    let mut wanted: Vec<String> = Vec::new();
    for entry in &entries {
        let env = env_of(entry)?;
        if todo.contains(env) {
            wanted.push(env.to_string());
            continue;
        }
        // A stored key that names its own access groups misses every team change: mint it again.
        let stored = keys.get(ctx, env)?;
        if stored.is_empty() || !is_safe(&stored) {
            continue;
        }
        let proxy = field(entry.get("litellm").unwrap_or(&Value::Null), "proxy");
        if let Ok(Some(scope)) = litellm::LiteLLM::new(proxy, &stored).narrower_than_team(ctx) {
            ctx.say(&format!("{env}: the key is scoped to {scope}, not to its team's models; minting it again"));
            wanted.push(env.to_string());
        }
    }
    wanted.sort();
    let mut token = ctx
        .env
        .get("LITELLM_ADMIN_TOKEN")
        .map(|t| t.trim().to_string())
        .unwrap_or_default();
    if !wanted.is_empty() && token.is_empty() {
        token = master_minted_token(ctx, &entries, keys)?;
    }
    if !wanted.is_empty() && token.is_empty() {
        let prompt = format!(
            "Paste a LiteLLM admin token to mint {} (a one-hour management key: \
             registry/generated/litellm/make-admin-token.sh makes one; it is not stored), or Enter to skip: ",
            wanted.join(", ")
        );
        token = ask(ctx, &prompt, true);
    }
    if token.is_empty() {
        for env in &wanted {
            ctx.say(&format!("{env}: skipped (no LiteLLM admin token)"));
        }
        return Ok(());
    }
    if !is_safe(&token) {
        ctx.say("the LiteLLM admin token has unexpected characters; nothing minted");
        return Ok(());
    }
    litellm::mint_litellm(ctx, &entries, &wanted, &token, keys)
}

/// The one-hour management key minted in process from the master key (the
/// environment, else the secret store under `LITELLM_MASTER_KEY`); "" when
/// there is no master key or the proxy would not give one.
fn master_minted_token(ctx: &mut Ctx, entries: &[&Value], keys: &Store) -> R<String> {
    let mut master = ctx
        .env
        .get("LITELLM_MASTER_KEY")
        .map(|k| k.trim().to_string())
        .unwrap_or_default();
    if master.is_empty() {
        master = keys.get(ctx, "LITELLM_MASTER_KEY")?;
    }
    let proxy = entries.first().map_or("", |e| {
        field(e.get("litellm").unwrap_or(&Value::Null), "proxy")
    });
    if master.is_empty() || proxy.is_empty() {
        return Ok(String::new());
    }
    if !is_safe(&master) {
        ctx.say("LITELLM_MASTER_KEY has unexpected characters; not used");
        return Ok(String::new());
    }
    match litellm::LiteLLM::new(proxy, &master).admin_token(ctx) {
        Ok(token) => {
            ctx.say("minted a one-hour LiteLLM management key from the master key (not stored)");
            Ok(token)
        }
        Err(Fail::Unreachable(e)) => {
            ctx.say(&format!(
                "could not reach the LiteLLM proxy ({e}); no management key minted"
            ));
            Ok(String::new())
        }
        Err(other) => {
            let e = other.refused()?;
            ctx.say(&format!(
                "litellm: {e}; the master key did not mint a management key"
            ));
            Ok(String::new())
        }
    }
}

// --- commands

fn mint_tokens(ctx: &mut Ctx, keys: &Store, auth: &Auth, me: &Value, tokens: &[&Value]) -> R<()> {
    let mut ids = load_ids(ctx);
    let wanted: BTreeSet<String> = tokens
        .iter()
        .map(|e| field(e, "grant").to_string())
        .collect();
    let held = auth.ensure_grants(ctx, me, &wanted)?;
    for entry in tokens {
        let env = env_of(entry)?;
        let grant = field(entry, "grant");
        if !held.contains(grant) {
            ctx.say(&format!(
                "{env}: you do not hold {grant}; ask an admin to grant it"
            ));
            continue;
        }
        let (token, ident, why) = auth.mint(ctx, grant)?;
        if token.is_empty() {
            let hint = if why.contains("grant") {
                "; ask an admin to grant it"
            } else {
                ""
            };
            ctx.say(&format!("{env}: mint refused: {why}{hint}"));
            continue;
        }
        if !is_safe(&token) {
            ctx.say(&format!(
                "{env}: the minted token has unexpected characters; not stored"
            ));
            auth.revoke(ctx, &ident)?;
            continue;
        }
        if check(ctx, entry, &token) == "invalid" {
            ctx.say(&format!(
                "{env}: the new token is refused by its host; not stored"
            ));
            auth.revoke(ctx, &ident)?;
            continue;
        }
        if gated(entry) {
            if !gate_store(ctx, &token) {
                ctx.say(&format!(
                    "{env}: minted but not sealed; revoking it, run setup again to retry"
                ));
                auth.revoke(ctx, &ident)?;
                continue;
            }
            ctx.say(&format!(
                "{env}: minted, checked and sealed behind your passphrase"
            ));
        } else {
            keys.put(ctx, env, &token)?;
            ctx.say(&format!(
                "{env}: minted, checked and stored in the {}",
                keys.name
            ));
        }
        replaced(ctx, auth, &mut ids, env, &ident)?;
    }
    Ok(())
}

/// Sign in and mint the tokens that are missing, then sign out.
fn sign_in_and_mint(ctx: &mut Ctx, keys: &Store, tokens: &[&Value]) -> R<()> {
    let names: Vec<&str> = tokens.iter().map(|e| field(e, "env")).collect();
    ctx.say(&format!(
        "signing in to auth.jorisjonkers.dev to mint {}",
        names.join(", ")
    ));
    let auth = Auth::new(ctx)?;
    let me = match auth.sign_in(ctx) {
        Ok(me) => me,
        Err(other) => {
            let e = other.unreachable()?;
            ctx.say(&format!(
                "could not reach auth.jorisjonkers.dev ({e}); no tokens minted this run"
            ));
            None
        }
    };
    let Some(me) = me.filter(truthy) else {
        return Ok(());
    };
    let done = mint_tokens(ctx, keys, &auth, &me, tokens);
    let _ = auth.sign_out(ctx);
    match done {
        Ok(()) => Ok(()),
        Err(other) => {
            let e = other.unreachable()?;
            ctx.say(&format!(
                "could not reach auth.jorisjonkers.dev ({e}); the remaining tokens were not minted"
            ));
            Ok(())
        }
    }
}

/// A value this tool cannot mint is asked for, until its host accepts it or the answer is empty.
fn paste(ctx: &mut Ctx, keys: &Store, entry: &Value, env: &str) -> R<()> {
    loop {
        let prompt = format!(
            "Paste {env} ({}), or Enter to skip: ",
            field(entry, "paste")
        );
        let value = ask(ctx, &prompt, true);
        if value.is_empty() {
            ctx.say(&format!("{env}: skipped"));
            return Ok(());
        }
        if !is_safe(&value) || check(ctx, entry, &value) == "invalid" {
            ctx.say(&format!("{env}: refused by its host; try again"));
            continue;
        }
        keys.put(ctx, env, &value)?;
        ctx.say(&format!("{env}: checked and stored in the {}", keys.name));
        return Ok(());
    }
}

fn ensure(ctx: &mut Ctx, table: &[Value]) -> R<i32> {
    let keys = Store::new(ctx);
    keys.migrate_legacy_file(ctx)?;
    retire(ctx, table, &keys)?;
    for entry in secrets(table) {
        let env = env_of(entry)?;
        // A copy outside the gate was readable without the passphrase: drop it and mint anew.
        if gated(entry) && !keys.get(ctx, env)?.is_empty() {
            keys.delete(ctx, env)?;
            ctx.say(&format!(
                "{env}: removed from the {}; a new one goes behind your passphrase",
                keys.name
            ));
        }
    }
    let mut todo = BTreeSet::new();
    for entry in secrets(table) {
        if matches!(state(ctx, entry, &keys)?, "missing" | "invalid") {
            todo.insert(env_of(entry)?.to_string());
        }
    }
    let tokens: Vec<&Value> = secrets(table)
        .into_iter()
        .filter(|e| todo.contains(field(e, "env")) && has(e, "grant"))
        .collect();
    if !tokens.is_empty() {
        sign_in_and_mint(ctx, &keys, &tokens)?;
    }
    ensure_litellm(ctx, table, &todo, &keys)?;
    for entry in secrets(table) {
        let env = env_of(entry)?;
        if !todo.contains(env) || has(entry, "grant") || has(entry, "mint") {
            continue;
        }
        if has(entry, "login") {
            // A sign-in this tool cannot do for you (a browser login): say which command does.
            ctx.say(&format!(
                "{env}: sign in with  {}  (in a terminal)",
                field(entry, "login")
            ));
            continue;
        }
        paste(ctx, &keys, entry, env)?;
    }
    write_env(ctx, table, &keys)?;
    wire_shell(ctx)?;
    Ok(0)
}

fn status(ctx: &mut Ctx, table: &[Value]) -> R<i32> {
    let keys = Store::new(ctx);
    for entry in secrets(table) {
        let line = format!("{} {}", env_of(entry)?, state(ctx, entry, &keys)?);
        let _ = writeln!(ctx.out, "{line}");
    }
    let ok = if wired(&shell_rc(ctx)) {
        "ok"
    } else {
        "missing"
    };
    let _ = writeln!(ctx.out, "shell {ok}");
    Ok(0)
}

/// None when the sign-in was refused.
fn revoke_by_id(ctx: &mut Ctx, auth: &Auth, ident: &str) -> R<Option<bool>> {
    if auth.sign_in(ctx)?.is_none() {
        return Ok(None);
    }
    auth.revoke(ctx, ident).map(Some)
}

/// Revoke this host's minted token for NAME by its recorded id; the stored value is left to fail its check.
fn revoke(ctx: &mut Ctx, table: &[Value], name: &str) -> R<i32> {
    let mints = secrets(table)
        .into_iter()
        .any(|e| field(e, "env") == name && has(e, "grant"));
    if !mints {
        ctx.say(&format!("{name}: not a token this tool mints"));
        return Ok(2);
    }
    let mut ids = load_ids(ctx);
    let Some(ident) = ids.get(name).cloned() else {
        ctx.say(&format!(
            "{name}: no token id recorded on this host; revoke it on the admin page"
        ));
        return Ok(1);
    };
    let auth = Auth::new(ctx)?;
    let done = revoke_by_id(ctx, &auth, &ident);
    let _ = auth.sign_out(ctx);
    let revoked = match done {
        Ok(Some(revoked)) => revoked,
        Ok(None) => return Ok(1),
        Err(other) => {
            let e = other.unreachable()?;
            ctx.say(&format!(
                "could not reach auth.jorisjonkers.dev ({e}); nothing revoked"
            ));
            return Ok(1);
        }
    };
    if !revoked {
        ctx.say(&format!(
            "{name}: auth-api refused to revoke token id {ident}"
        ));
        return Ok(1);
    }
    ids.remove(name);
    save_ids(ctx, &ids)?;
    ctx.say(&format!(
        "{name}: revoked; `status` now reports it invalid, and the next setup run mints a new one"
    ));
    Ok(0)
}

const USAGE: &str =
    "usage: agent-kit-keys --table <workstation-secrets.json> ensure|status|write-env\n       \
                     agent-kit-keys --table <workstation-secrets.json> revoke NAME";

enum Cmd {
    Ensure,
    Status,
    WriteEnv,
    Revoke(String),
}

fn parse(args: &[String]) -> Result<(PathBuf, Cmd), String> {
    let mut table = None;
    let mut words: Vec<&str> = Vec::new();
    let mut it = args.iter();
    while let Some(arg) = it.next() {
        if arg == "--table" {
            table = Some(it.next().ok_or("--table needs a path")?.clone());
        } else if let Some(path) = arg.strip_prefix("--table=") {
            table = Some(path.to_string());
        } else {
            words.push(arg);
        }
    }
    let table = table.ok_or("--table is required")?;
    let command = match words.as_slice() {
        ["ensure"] => Cmd::Ensure,
        ["status"] => Cmd::Status,
        ["write-env"] => Cmd::WriteEnv,
        ["revoke"] => {
            return Err("revoke needs the variable name, for example HINDSIGHT_API_TOKEN".into())
        }
        ["revoke", name] => Cmd::Revoke((*name).to_string()),
        _ => return Err("expected one of ensure, status, write-env, revoke NAME".into()),
    };
    Ok((PathBuf::from(table), command))
}

fn load_table(path: &Path) -> R<Vec<Value>> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| Fail::Stop(format!("cannot read {}: {e}", path.display())))?;
    match serde_json::from_str::<Value>(&text) {
        Ok(Value::Array(entries)) => Ok(entries),
        Ok(_) => Err(Fail::Stop(format!(
            "{} is not a list of entries",
            path.display()
        ))),
        Err(e) => Err(Fail::Stop(format!("{}: {e}", path.display()))),
    }
}

fn write_env_only(ctx: &mut Ctx, table: &[Value]) -> R<i32> {
    let keys = Store::new(ctx);
    keys.migrate_legacy_file(ctx)?;
    retire(ctx, table, &keys)?;
    write_env(ctx, table, &keys).map(|_| 0)
}

/// The whole run against a supplied environment; the exit status.
pub fn run_in(ctx: &mut Ctx, args: &[String]) -> i32 {
    let (path, command) = match parse(args) {
        Ok(parsed) => parsed,
        Err(message) => {
            let _ = writeln!(ctx.err, "agent-kit-keys: {message}\n{USAGE}");
            return 2;
        }
    };
    let result = load_table(&path).and_then(|table| match command {
        Cmd::Ensure => ensure(ctx, &table),
        Cmd::Status => status(ctx, &table),
        Cmd::Revoke(name) => revoke(ctx, &table, &name),
        Cmd::WriteEnv => write_env_only(ctx, &table),
    });
    match result {
        Ok(code) => code,
        Err(Fail::Stop(message) | Fail::Unreachable(message) | Fail::Refused(message)) => {
            ctx.say(&format!("stopped: {message}"));
            1
        }
    }
}

fn hostname() -> String {
    let named = Command::new("uname").arg("-n").output();
    named
        .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
        .unwrap_or_default()
}

/// The environment as this crate reads it. A variable whose name or value is
/// not UTF-8 is skipped: `std::env::vars()` panics on one, and the environment
/// is whatever the shell that started setup passed on.
fn utf8_env(vars: impl Iterator<Item = (OsString, OsString)>) -> HashMap<String, String> {
    vars.filter_map(|(name, value)| Some((name.into_string().ok()?, value.into_string().ok()?)))
        .collect()
}

/// The process environment, read through `utf8_env`.
fn process_env() -> HashMap<String, String> {
    utf8_env(std::env::vars_os())
}

/// The command line: ensure, status, write-env, revoke NAME, all with `--table`.
pub fn run(args: impl Iterator<Item = OsString>) -> i32 {
    use std::io::IsTerminal;
    let args: Vec<String> = args.map(|a| a.to_string_lossy().into_owned()).collect();
    let mut ctx = Ctx {
        env: process_env(),
        home: std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default(),
        input: Box::new(std::io::BufReader::new(std::io::stdin())),
        tty: std::io::stdin().is_terminal(),
        out: Box::new(std::io::stdout()),
        err: Box::new(std::io::stderr()),
        host: hostname(),
        exe: std::env::current_exe().unwrap_or_default(),
    };
    run_in(&mut ctx, &args)
}

#[cfg(test)]
mod tests;
