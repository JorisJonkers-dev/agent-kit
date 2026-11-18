//! The gateway's launch path, for a caller inside this process: it builds the
//! client's command with the caller's extras, runs it, and reads what the
//! client exported. A caller that parses its own output, as critique does,
//! reads `result` and `exported`; a delegation reads `output` too.

use crate::config::ToolScope;
use crate::client_confine::{no_launcher, Confinement};
use crate::error::GatewayError;
use crate::extras::{RunExtras, EXTRA_NAMES};
use crate::home_mcp::home_mcp_servers;
use crate::json::{self, Value};
use crate::launchers::{launcher, LaunchExtras, LaunchPaths, LaunchRequest, PiConfinement};
use crate::outputs::ClientOutput;
use crate::pi_launch::{launcher_of, prepare_pi};
use crate::ports::{Files, Process, ProcessResult};
use crate::text::join;
use crate::run::RunClient;
use crate::run_home::{sync_login, prepare_codex, prepare_hermes, prepare_opencode, prepare_tmp, prepare_vibe, run_home_dir, RunHome};
use std::collections::HashMap;

/// The longest a keychain read may take: it may be locked, and an unattended run must not wait on a dialog.
const KEYCHAIN_TIMEOUT_MS: u64 = 15_000;

/// The Vibe tools that load on demand through `search_tool_functions`, which `--enabled-tools` filters out along with them.
const VIBE_ON_DEMAND_TOOLS: [&str; 2] = ["web_search", "web_fetch"];

/// One client launch, as a caller inside this process asks for it: its own prompt, its own directory, its own extras.
#[derive(Clone, Debug)]
pub struct ClientRunRequest {
    pub launch: LaunchRequest,
    pub binary: String,
    pub client: RunClient,
    /// The prompt text, which the client reads from `paths.prompt_path` or from its arguments.
    pub prompt: String,
    pub paths: LaunchPaths,
    /// What the caller adds to the client's command; none when it adds nothing.
    pub extras: RunExtras,
}

/// What one client launch came to: the process's own result, what it exported, and what its output says.
#[derive(Clone, Debug, PartialEq)]
pub struct ClientRunOutcome {
    pub result: ProcessResult,
    /// Parsed `export.json`, or `None` when the client wrote none or it does not parse.
    pub exported: Option<Value>,
    pub output: ClientOutput,
    /// What the launch could not clean up: a run home that was not removed still holds what the run needed.
    pub warnings: Vec<String>,
}

/// Why a launch did not come to an outcome: the gateway refused it, or the
/// client's program could not be started.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ClientRunError {
    Gateway(GatewayError),
    /// The platform's words for a program that would not start.
    Spawn(String),
}

impl From<GatewayError> for ClientRunError {
    fn from(error: GatewayError) -> Self {
        ClientRunError::Gateway(error)
    }
}

/// Whether the caller named the extra.
fn names(extras: &RunExtras, name: &str) -> bool {
    match name {
        "agent" => extras.agent.is_some(),
        "disable_home_mcp" => extras.disable_home_mcp.is_some(),
        "env" => extras.env.is_some(),
        "mcp_servers" => extras.mcp_servers.is_some(),
        "sandbox" => extras.sandbox.is_some(),
        "schema" => extras.schema.is_some(),
        _ => extras.tools.is_some(),
    }
}

/// Refuses an extra the client cannot apply, naming the extra and the client,
/// and an `extras.env` variable the launcher sets itself, naming the variable.
/// A caller that needs a server attached, a home switched off or a sandbox
/// profile gets a refusal rather than a run that quietly drops it, and one that
/// tries to lift the launcher's own isolation is refused rather than obeyed.
pub fn require_supported_extras(client: RunClient, extras: &RunExtras) -> Result<(), GatewayError> {
    let launcher = launcher(client);
    let name = client.as_str();
    if let Some(unsupported) = EXTRA_NAMES.iter().find(|extra| names(extras, extra) && !launcher.extras.contains(extra)) {
        return Err(GatewayError::new("extras-not-supported", format!("{name} cannot apply the {unsupported} extra")));
    }
    let web_tool = if client == RunClient::Vibe {
        extras.tools.as_deref().unwrap_or_default().iter().find(|tool| VIBE_ON_DEMAND_TOOLS.contains(&tool.as_str()))
    } else {
        None
    };
    if let Some(web_tool) = web_tool {
        return Err(GatewayError::new(
            "extras-not-supported",
            format!(
                "vibe cannot enable the {web_tool} tool: it loads on demand, and --enabled-tools hides it, so the run would have no web tool; keep the default tool set and disable the unwanted tools instead"
            ),
        ));
    }
    if let Some(sandboxed) = launcher.sandboxed_extras.iter().find(|extra| names(extras, extra) && extras.sandbox.is_none()) {
        return Err(GatewayError::new(
            "extras-not-supported",
            format!("{name} applies the {sandboxed} extra only in the read-only sandbox, so it needs the sandbox extra too"),
        ));
    }
    let protected = extras.env.iter().flat_map(|env| env.keys()).find(|variable| launcher.protected_env.contains(&variable.as_str()));
    if let Some(protected) = protected {
        return Err(GatewayError::new("extras-not-supported", format!("{name} sets {protected} itself, so extras.env may not name it")));
    }
    Ok(())
}

/// What one client launch reaches the system through.
pub struct ClientRun<'a> {
    pub process: &'a dyn Process,
    pub files: &'a dyn Files,
    /// The environment the client's home is named in.
    pub env: &'a HashMap<String, String>,
}

impl ClientRun<'_> {
    pub fn launch(&self, request: &ClientRunRequest) -> Result<ClientRunOutcome, ClientRunError> {
        self.launch_scoped(request, None)
    }

    /// The launch, with the tool scope a delegation profile gives a Pi run: its home holds exactly the skills
    /// and plugins the scope lists.
    pub fn launch_scoped(&self, request: &ClientRunRequest, scope: Option<&ToolScope>) -> Result<ClientRunOutcome, ClientRunError> {
        require_supported_extras(request.client, &request.extras)?;
        let launcher = launcher(request.client);
        let confine = if request.client == RunClient::Pi { Some(self.confinement_of(request, scope)?) } else { None };
        let home = self.home_of(request)?;
        // A client that can sign in from an environment variable gets its credential that way and no keychain: the
        // gateway asks the keychain, outside the confinement, for the one item the client needs.
        let credential = self.credential_of(request);
        let confinement = match self.confinement_of_client(request, home.as_ref(), credential.is_some()) {
            Ok(confinement) => confinement,
            Err(error) => {
                // A home made for a launch that never starts goes at once: it may hold a link to the owner's login.
                let _ = home.iter().try_for_each(|home| self.files.remove(&home.dir));
                return Err(error.into());
            }
        };
        let mut extras = LaunchExtras { confine: confine.clone(), home: home.clone(), confinement, ..self.extras_of(request)? };
        extras.env.extend(credential);
        let command = (launcher.command)(&request.binary, &request.launch, &request.paths, &request.prompt, Some(&extras));
        let ran = self.process.run(&command);
        // What the launch refreshed in its copy of a login goes back to the owner's, before the home goes.
        let unsynced: Vec<String> = home.iter().filter_map(|home| home.login.as_ref()).filter_map(|(owner, copy)| sync_login(self.files, owner, copy).err()).collect();
        // The agent directory holds the route's key, so it goes the moment the launch ends, and so does a run home; a
        // directory that stays is reported, never left unsaid. `remove` does not follow a symlink.
        let leftovers: Vec<String> = confine
            .iter()
            .map(|confine| confine.agent_dir.clone())
            .chain(home.iter().map(|home| home.dir.clone()))
            .filter_map(|dir| self.files.remove(&dir).err().map(|error| format!("the launch's directory {dir} was not removed: {}", error.message)))
            .chain(unsynced)
            .collect();
        let result = ran.map_err(|message| ClientRunError::Spawn(if leftovers.is_empty() { message } else { format!("{message}; {}", leftovers.join("; ")) }))?;
        let exported = match launcher.report_path {
            Some(path) => self.read_report(&path(&request.paths))?,
            None => None,
        };
        let output = (launcher.read)(&result.stdout, exported.as_ref());
        Ok(ClientRunOutcome { result, exported, output, warnings: leftovers })
    }

    /// What a Pi launch runs through, or the refusal that says why it cannot: Pi has no sandbox of its own,
    /// so a launch that cannot be confined never starts.
    fn confinement_of(&self, request: &ClientRunRequest, scope: Option<&ToolScope>) -> Result<PiConfinement, GatewayError> {
        let Some(provider) = request.launch.provider.as_deref() else {
            return Err(GatewayError::new("unconfined", "a pi launch must name its provider"));
        };
        let agent_dir = join(&[&request.paths.export_dir, "pi-agent"]);
        prepare_pi(self.files, self.process, self.env, provider, &request.launch.model, &agent_dir, scope).map_err(|message| GatewayError::new("unconfined", message))
    }

    /// The credential of a client that signs in from an environment variable, read from the macOS keychain with
    /// `security` outside the confinement: Claude Code's `CLAUDE_CODE_OAUTH_TOKEN` (the access token of its
    /// subscription login) and Vibe's `MISTRAL_API_KEY`. `None` where the keychain has none (Linux keeps these in
    /// files) or the client has no such variable: the launch then gets `--keychain` and the client asks for itself.
    fn credential_of(&self, request: &ClientRunRequest) -> Option<(String, String)> {
        let (service, account, variable) = match request.client {
            RunClient::Claude => ("Claude Code-credentials", self.env.get("USER")?.as_str(), "CLAUDE_CODE_OAUTH_TOKEN"),
            RunClient::Vibe => ("ai.mistral.vibe", "MISTRAL_API_KEY", "MISTRAL_API_KEY"),
            _ => return None,
        };
        let read = self.process.exec("/usr/bin/security", &["find-generic-password", "-s", service, "-a", account, "-w"], Some(KEYCHAIN_TIMEOUT_MS)).ok()?;
        let text = crate::text::js_trim(&read.stdout).to_string();
        if read.exit_code != 0 || text.is_empty() {
            return None;
        }
        let value = if request.client == RunClient::Claude {
            let login = json::parse(&text).ok()?;
            login.as_object()?.get("claudeAiOauth")?.as_object()?.get("accessToken")?.as_str()?.to_string()
        } else {
            text
        };
        Some((variable.to_string(), value))
    }

    /// What a client but Pi is started through: the launcher and the grants of its recipe (ADR-0016), or why it
    /// cannot be confined. A client that cannot is refused, never started bare.
    fn confinement_of_client(&self, request: &ClientRunRequest, home: Option<&RunHome>, has_credential: bool) -> Result<Option<Confinement>, GatewayError> {
        let Some(home) = home else {
            return Ok(None);
        };
        let client = request.client;
        let launcher = launcher_of(self.files, self.env).ok_or_else(|| GatewayError::new("unconfined", no_launcher(client.as_str())))?;
        let owner_home = self.env.get("HOME").map_or("", String::as_str);
        let gemini = |name: &str| join(&[owner_home, ".gemini", name]);
        let (keychain, preferences, own, env) = match client {
            // Its subscription login is in the keychain, which a `security` call under `USER` finds.
            RunClient::Claude => (true, false, Vec::new(), vec!["USER".to_string(), "LOGNAME".to_string()]),
            // Its login is in the keychain, and its state in two directories of the owner's `~/.gemini`.
            RunClient::Agy => (true, false, vec![gemini("antigravity-cli"), gemini("config")], Vec::new()),
            RunClient::Codex => (true, true, Vec::new(), Vec::new()),
            RunClient::Vibe => (true, false, Vec::new(), Vec::new()),
            _ => (false, false, Vec::new(), Vec::new()),
        };
        let keychain = keychain && !has_credential;
        let mut reads = home.reads.clone();
        // What the caller hands the launch: the server programs it attaches (run from their install, and reading the
        // files they are given by path) and the schema file it names, which the client reads.
        let mut run_from = Vec::new();
        for server in request.extras.mcp_servers.iter().flatten() {
            run_from.push(server.command.clone());
            reads.extend(server.args.iter().filter(|arg| arg.starts_with('/') && self.files.exists(arg).unwrap_or(false)).cloned());
        }
        reads.extend(request.extras.schema.iter().filter(|schema| self.files.exists(schema).unwrap_or(false)).cloned());
        // Vibe walks `~/.agents/skills` at start and fails on a directory it may not even stat; the delegate agent
        // enables no skill, so the listing is all it takes.
        let agents_skills = join(&[owner_home, ".agents", "skills"]);
        if client == RunClient::Vibe && self.files.exists(&agents_skills).unwrap_or(false) {
            reads.push(agents_skills);
        }
        let mut writes = home.writes.clone();
        // A home beside the export directory (no run directory) is writable too; one under it already is.
        if !std::path::Path::new(&home.dir).starts_with(crate::launchers::report_dir(&request.paths)) {
            writes.push(home.dir.clone());
        }
        writes.extend(own);
        // Antigravity serves itself on a loopback port chosen at run time, like OpenCode: loopback at any port, the rest
        // of the network at 443 only.
        let (ports, loopback) = if client == RunClient::Agy { (vec![443], true) } else { (home.ports.clone(), home.loopback) };
        Ok(Some(Confinement { launcher, reads, run_from, writes, ports, loopback, keychain, preferences, env }))
    }

    /// The home of a launch of a client but Pi (#903): what the client runs in, and what the confinement grants for
    /// it. A launch whose provider the owner's home does not declare, or whose login is not there, is refused rather
    /// than started in the owner's own home.
    fn home_of(&self, request: &ClientRunRequest) -> Result<Option<RunHome>, GatewayError> {
        if request.client == RunClient::Pi {
            return Ok(None);
        }
        let dir = run_home_dir(&request.paths);
        // A run's directory is its own (run ids are unique), so a home that is already there is not this launch's: it is never reused.
        if self.files.exists(&dir).unwrap_or(false) {
            return Err(GatewayError::new("unconfined", format!("the launch's home {dir} already exists; a run home is made for one launch and never reused")));
        }
        let made = match request.client {
            RunClient::Opencode => prepare_opencode(self.files, self.env, &request.launch.model, request.launch.mode, &dir),
            RunClient::Hermes => prepare_hermes(self.files, self.env, request.launch.provider.as_deref().unwrap_or("undefined"), &dir),
            RunClient::Codex => prepare_codex(self.files, self.env, &dir),
            RunClient::Vibe => prepare_vibe(self.files, self.env, &dir),
            RunClient::Claude => prepare_tmp(self.files, "claude", &dir, Some("CLAUDE_CODE_TMPDIR")),
            _ => prepare_tmp(self.files, request.client.as_str(), &dir, None),
        };
        made.map(Some).map_err(|message| GatewayError::new("unconfined", message))
    }

    /// The caller's extras as the launcher applies them, with the client's home servers resolved.
    fn extras_of(&self, request: &ClientRunRequest) -> Result<LaunchExtras, GatewayError> {
        let extras = &request.extras;
        let home_servers = if extras.disable_home_mcp == Some(true) { home_mcp_servers(self.files, self.env)? } else { Vec::new() };
        Ok(LaunchExtras {
            confine: None,
            agent: extras.agent.clone(),
            env: extras.env.clone().unwrap_or_default(),
            home_servers,
            mcp_servers: extras.mcp_servers.clone().unwrap_or_default(),
            sandbox: extras.sandbox,
            schema: extras.schema.clone(),
            tools: extras.tools.clone().unwrap_or_default(),
            home: None,
            confinement: None,
        })
    }

    /// The report the client wrote beside its output (Vibe's export, Hermes' usage), or `None` when it wrote none or it does not parse.
    fn read_report(&self, path: &str) -> Result<Option<Value>, GatewayError> {
        if !self.files.exists(path).map_err(GatewayError::from_fs)? {
            return Ok(None);
        }
        Ok(self.files.read_text(path).ok().and_then(|text| json::parse(&text).ok()))
    }
}
