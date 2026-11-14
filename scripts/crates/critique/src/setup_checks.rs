//! What `/flow setup` checks on this machine and what a check's last result says: the checklist, each item's due
//! state, and the results file. The node runner's `domain/setup.ts`.

use crate::error::ShapeError;
use crate::fields::Fields;
use crate::routing::CriticClient;
use model_gateway::json::{self, Value};

/// What a check's result depends on, by input name in the order the check builds them: `None` is a `null` (the CLI
/// is missing, the agent is not installed). A result stays good while these are unchanged: the CLI's version, the
/// critic agent file a Vibe critic runs through, the part of routing the check reads, and the local files a hook
/// check reads.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SetupInputs(pub Vec<(&'static str, Option<String>)>);

/// The inputs, in the order a change is looked for.
pub const SETUP_INPUTS: [&str; 4] = ["cli_version", "agent_hash", "routing_hash", "files_hash"];

impl SetupInputs {
    pub fn of(pairs: Vec<(&'static str, Option<String>)>) -> SetupInputs {
        SetupInputs(pairs)
    }

    fn get(&self, key: &str) -> Option<&Option<String>> {
        self.0.iter().find(|(name, _)| *name == key).map(|(_, value)| value)
    }

    pub fn to_json(&self) -> Value {
        json::object(self.0.iter().map(|(key, value)| (*key, value.as_ref().map_or(Value::Null, |text| Value::str(text)))).collect())
    }
}

/// One check's last run on this machine.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SetupCheckResult {
    pub checked_at: String,
    /// `pass`, `fail`, or `unproven`: a proof that cannot be shown without a live prompt this machine does not spend.
    pub result: String,
    pub detail: String,
    pub inputs: SetupInputs,
}

impl SetupCheckResult {
    pub fn to_json(&self) -> Value {
        json::object(vec![
            ("checked_at", Value::str(&self.checked_at)),
            ("detail", Value::str(&self.detail)),
            ("inputs", self.inputs.to_json()),
            ("result", Value::str(&self.result)),
        ])
    }
}

/// Reads the results file as strictly as the node runner's `parseSetupResults`: an entry with a field of the wrong shape
/// makes the whole file unreadable.
pub fn parse_setup_results(text: &str) -> Result<Vec<(String, SetupCheckResult)>, ShapeError> {
    let value = json::parse(text).map_err(ShapeError)?;
    let file = Fields::of(&value, "")?;
    let checks = file.entries("checks", |entry, _| {
        let inputs = entry.object("inputs")?;
        let mut read = Vec::new();
        let checked_at = entry.string("checked_at")?;
        let detail = entry.string("detail")?;
        for key in ["cli_version", "agent_hash"] {
            if inputs.has(key) {
                read.push((key, inputs.nullable_string(key)?));
            }
        }
        for key in ["routing_hash", "files_hash"] {
            if inputs.has(key) {
                read.push((key, Some(inputs.string(key)?)));
            }
        }
        let result = entry.one_of("result", &["pass", "fail", "unproven"])?;
        Ok(SetupCheckResult { checked_at, detail, inputs: SetupInputs(read), result })
    })?;
    file.literal("version", 1.0)?;
    Ok(checks)
}

/// What running a check spends: nothing (`free`), Vibe money on the personal account, one Antigravity invocation, or
/// one trivial prompt on a LiteLLM route's budget through Pi (`pi`).
pub type SetupCost = &'static str;

/// One item of the checklist, before anything is checked.
#[derive(Clone, Debug, PartialEq)]
pub struct SetupCheck {
    pub id: String,
    /// `vibe-signed-in`, `agy-signed-in`, `pi-signed-in`, `critic`, `agy-quota`, `read-only`, `hooks` or `kill-switch`.
    pub kind: &'static str,
    pub title: String,
    /// One sentence on why the item matters.
    pub why: String,
    /// What to do when the check fails.
    pub fix: String,
    pub cost: SetupCost,
    /// The routing client the item is about; absent for a sign-in, the quota and the hooks.
    pub client: Option<String>,
    /// The CLI the check reads: the item's client, or for a sign-in or the quota, the first routing client of that CLI.
    /// Absent for a hook check and for the read-only proof of a client without a critic.
    pub client_config: Option<CriticClient>,
    /// Why a client without a critic is not proven read-only, in one sentence.
    pub reason: Option<String>,
}

/// The clients without a critic configuration, in the order setup lists their proofs.
const UNPROVEN_READ_ONLY_CLIENTS: [(&str, &str); 4] = [
    ("claude", "Claude Code's read-only mode is a permission mode, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof"),
    ("codex", "Codex's read-only mode is a sandbox setting, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof"),
    ("hermes", "Hermes has no read-only flag, so only its toolsets limit a run, and showing that they refuse writes needs a live prompt, which setup does not spend on a proof"),
    ("opencode", "OpenCode's read-only use is an agent whose edit permissions are denied in its own configuration, and showing that it refuses writes needs a live prompt, which setup does not spend on a proof"),
];

/// agy's read-only proof is #178's live smoke check, which does not exist yet.
pub const READ_ONLY_PENDING: &str = "not available until agent-kit#178 lands: the read-only proof reuses its live smoke check";

/// Pi has no read-only mode to prove: a Pi critic is launched with the read tools alone, and its stream is checked after
/// every run.
pub const PI_READ_ONLY_PENDING: &str = "not available: Pi has no sandbox of its own to prove, so every Pi critic is launched with read, grep, find and ls alone, and a run that used another tool or reached past its packet is voided";

/// What a passed Vibe read-only proof leaves to #178.
pub const WRITE_PROOF_PENDING: &str = "agent-kit#178's live smoke check adds the rest of the read-only proof";

fn base(id: &str, kind: &'static str, title: &str, why: &str, fix: &str) -> SetupCheck {
    SetupCheck {
        id: id.to_string(),
        kind,
        title: title.to_string(),
        why: why.to_string(),
        fix: fix.to_string(),
        cost: "free",
        client: None,
        client_config: None,
        reason: None,
    }
}

fn hooks_installed() -> SetupCheck {
    base(
        "hooks-installed",
        "hooks",
        "The critique hooks are installed in Claude Code and Codex",
        "Without the commit and PR gate hooks every gate is advisory, and Codex skips a hook the owner has not trusted in /hooks.",
        "Re-run agent-kit setup to install the hooks; in Codex, open /hooks and trust critique-commit-gate, critique-pr-gate and critique-session-start; then check again.",
    )
}

fn kill_switch() -> SetupCheck {
    base(
        "kill-switch",
        "kill-switch",
        "The kill switch turns the hooks off",
        "CRITIQUE_OFF=1 or the disabled flag in the per-machine critique configuration must make every hook a no-op at once.",
        "Turn the kill switch off or on here, or fix the critique configuration file the detail names.",
    )
}

fn with_config(mut check: SetupCheck, config: &CriticClient) -> SetupCheck {
    check.client_config = Some(config.clone());
    check
}

fn critic_check(client: &str, config: &CriticClient) -> SetupCheck {
    let through = match config {
        CriticClient::Vibe(vibe) => format!("the {} agent", vibe.agent),
        CriticClient::Pi(pi) => format!("pi on {}", pi.provider),
        _ => "agy".to_string(),
    };
    let fix = match config {
        CriticClient::Vibe(vibe) => format!("Re-run agent-kit setup to reinstall {}, then check again; if it still fails, read the detail.", vibe.agent),
        CriticClient::Pi(pi) => format!(
            "Check that {} offers {} in ~/.pi/agent/models.json and its route key works, then check again; if it still fails, read the detail.",
            pi.provider, pi.model
        ),
        _ => "Check that agy runs and offers the model, then check again; if it still fails, read the detail.".to_string(),
    };
    let mut check = base(
        &format!("critic:{client}"),
        "critic",
        &format!("{client} answers a trivial prompt"),
        &format!("{} runs through {through}; a critic that cannot answer a trivial prompt fails every review it is routed to.", config.model()),
        &fix,
    );
    check.client = Some(client.to_string());
    check.cost = config.kind();
    with_config(check, config)
}

fn read_only_check(client: &str, config: &CriticClient) -> SetupCheck {
    let vibe = config.kind() == "vibe";
    let mut check = base(
        &format!("read-only:{client}"),
        "read-only",
        &format!("{client} is proven read-only"),
        if vibe {
            "A critic may only read and search its packet; this proves the search server cannot reach past the packet, and that run_typescript, the only way into the search tools, refuses shell commands, file writes and process starts."
        } else {
            "A critic may only read and search its packet; this proves the client refuses to change files and its search tools cannot reach past the packet on this machine."
        },
        match config {
            CriticClient::Vibe(_) => "Re-run agent-kit setup so the critique runner is current, then check again; until it passes, this client reads its packet without searching.",
            CriticClient::Pi(_) => "Nothing to fix: every Pi critic run is checked against its packet instead.",
            _ => "Wait for agent-kit#178.",
        },
    );
    check.client = Some(client.to_string());
    // A Vibe client's proof is one trivial prompt, after a search-reach check that calls no model; a Pi client's runs nothing.
    check.cost = if config.kind() == "pi" { "free" } else { config.kind() };
    with_config(check, config)
}

fn unproven_read_only_check(client: &str, reason: &str) -> SetupCheck {
    let mut check = base(
        &format!("read-only:{client}"),
        "read-only",
        &format!("{client} has no read-only proof yet"),
        &format!("{reason}."),
        "Nothing to fix: this client has no critic, and its read-only delegations run in a throwaway worktree until a live proof exists.",
    );
    check.client = Some(client.to_string());
    check.reason = Some(reason.to_string());
    check
}

/// The machine checklist, in the order setup walks it: Vibe's sign-in and each Vibe critic, Antigravity's sign-in, its
/// critic and its quota, Pi's provider and each Pi critic, the read-only proof per critic client, then the hooks (installed, and the kill switch).
/// Clients come from routing, in its order. The work clients have no setup check yet: only a nedap repository's roster
/// names them.
pub fn setup_checklist(clients: &[(String, CriticClient)]) -> Vec<SetupCheck> {
    let entries: Vec<&(String, CriticClient)> = clients.iter().filter(|(_, config)| !matches!(config, CriticClient::Codex(_) | CriticClient::Claude(_))).collect();
    let of = |kind: &str| entries.iter().filter(|(_, config)| config.kind() == kind).copied().collect::<Vec<_>>();
    let (vibe, agy, pi) = (of("vibe"), of("agy"), of("pi"));
    let mut checks = Vec::new();
    if let Some((_, first)) = vibe.first() {
        let mut sign_in = base(
            "vibe-signed-in",
            "vibe-signed-in",
            "Vibe is signed in on the personal account",
            "Every Vibe critic runs on the personal Mistral account, so while it is signed out each one drops out of a run.",
            "Run vibe and sign in with your personal Mistral account, then check again.",
        );
        sign_in = with_config(sign_in, first);
        checks.push(sign_in);
        checks.extend(vibe.iter().map(|(client, config)| critic_check(client, config)));
    }
    if let Some((_, first)) = agy.first() {
        let sign_in = base(
            "agy-signed-in",
            "agy-signed-in",
            "Antigravity is signed in",
            "The Gemini critic runs through agy on your Google account, so while it is signed out its slots fall back to Vibe.",
            "Run agy and sign in with your Google account, then check again.",
        );
        checks.push(with_config(sign_in, first));
        checks.extend(agy.iter().map(|(client, config)| critic_check(client, config)));
        let quota = base(
            "agy-quota",
            "agy-quota",
            "The Antigravity daily cap and reserve are set",
            "agy reports no quota, so the runner counts its own launches against a daily cap, which should be your plan\u{2019}s real limit.",
            "Give your Antigravity plan's daily request limit and the reserve to keep for the spec and feature reviews.",
        );
        checks.push(with_config(quota, first));
    }
    if let Some((_, first)) = pi.first() {
        let sign_in = base(
            "pi-signed-in",
            "pi-signed-in",
            "Pi is installed and its LiteLLM provider resolves a key",
            "Every Pi critic runs through a LiteLLM route, so while pi is missing or its provider has no key each one drops out of a run.",
            "Install pi, then re-run agent-kit setup so it writes the LiteLLM providers and their keys, then check again.",
        );
        checks.push(with_config(sign_in, first));
        checks.extend(pi.iter().map(|(client, config)| critic_check(client, config)));
    }
    checks.extend(entries.iter().map(|(client, config)| read_only_check(client, config)));
    checks.extend(UNPROVEN_READ_ONLY_CLIENTS.iter().map(|(client, reason)| unproven_read_only_check(client, reason)));
    checks.push(hooks_installed());
    checks.push(kill_switch());
    checks
}

/// One item of the checklist as the skill sees it, with where it stands.
#[derive(Clone, Debug, PartialEq)]
pub struct SetupItem {
    check: SetupCheck,
    /// `current`: it last passed and nothing it depends on changed. `due`: it should run. `not-available`: no check exists yet.
    pub status: &'static str,
    /// Why a due item is due: `never-checked`, `failed` or `inputs-changed`.
    pub due_because: Option<&'static str>,
    pub changed: Option<Vec<&'static str>>,
    pub detail: Option<&'static str>,
    pub inputs: Option<SetupInputs>,
    pub last: Option<SetupCheckResult>,
}

/// Whether an item runs: a check that never ran here, last failed, or whose inputs changed is due; one that passed with
/// the same inputs is current.
pub fn setup_item(check: &SetupCheck, inputs: SetupInputs, last: Option<&SetupCheckResult>) -> SetupItem {
    let item = |status, due_because, changed, detail, inputs, last| SetupItem { check: check.clone(), status, due_because, changed, detail, inputs, last };
    if check.kind == "read-only" && matches!(check.client_config, Some(CriticClient::Agy(_))) {
        return item("not-available", None, None, Some(READ_ONLY_PENDING), None, None);
    }
    if check.kind == "read-only" && matches!(check.client_config, Some(CriticClient::Pi(_))) {
        return item("not-available", None, None, Some(PI_READ_ONLY_PENDING), None, None);
    }
    let shown = last.cloned();
    let Some(last) = last else {
        return item("due", Some("never-checked"), None, None, Some(inputs), shown);
    };
    let changed: Vec<&'static str> = SETUP_INPUTS.iter().copied().filter(|key| last.inputs.get(key) != inputs.get(key)).collect();
    if !changed.is_empty() {
        return item("due", Some("inputs-changed"), Some(changed), None, Some(inputs), shown);
    }
    if last.result == "fail" {
        return item("due", Some("failed"), None, None, Some(inputs), shown);
    }
    item("current", None, None, None, Some(inputs), shown)
}

impl SetupItem {
    pub fn to_json(&self) -> Value {
        let check = &self.check;
        let mut pairs = vec![
            ("cost", Value::str(check.cost)),
            ("fix", Value::str(&check.fix)),
            ("id", Value::str(&check.id)),
            ("kind", Value::str(check.kind)),
            ("title", Value::str(&check.title)),
            ("why", Value::str(&check.why)),
        ];
        if let Some(client) = &check.client {
            pairs.push(("client", Value::str(client)));
        }
        if let Some(inputs) = &self.inputs {
            pairs.push(("inputs", inputs.to_json()));
        }
        if let Some(last) = &self.last {
            pairs.push(("last", last.to_json()));
        }
        if let Some(changed) = &self.changed {
            pairs.push(("changed", Value::Arr(changed.iter().map(|key| Value::str(key)).collect())));
        }
        if let Some(due) = self.due_because {
            pairs.push(("due_because", Value::str(due)));
        }
        if let Some(detail) = self.detail {
            pairs.push(("detail", Value::str(detail)));
        }
        pairs.push(("status", Value::str(self.status)));
        json::object(pairs)
    }
}
