//! The gateway's launch path, for a caller inside this process: it builds the
//! client's command with the caller's extras, runs it, and reads what the
//! client exported. A caller that parses its own output, as critique does,
//! reads `result` and `exported`; a delegation reads `output` too.

use crate::error::GatewayError;
use crate::extras::{RunExtras, EXTRA_NAMES};
use crate::home_mcp::home_mcp_servers;
use crate::json::{self, Value};
use crate::launchers::{launcher, LaunchExtras, LaunchPaths, LaunchRequest};
use crate::outputs::ClientOutput;
use crate::ports::{Files, Process, ProcessResult};
use crate::run::RunClient;
use std::collections::HashMap;

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
        require_supported_extras(request.client, &request.extras)?;
        let launcher = launcher(request.client);
        let extras = self.extras_of(request)?;
        let command = (launcher.command)(&request.binary, &request.launch, &request.paths, &request.prompt, Some(&extras));
        let result = self.process.run(&command).map_err(ClientRunError::Spawn)?;
        let exported = match launcher.report_path {
            Some(path) => self.read_report(&path(&request.paths))?,
            None => None,
        };
        let output = (launcher.read)(&result.stdout, exported.as_ref());
        Ok(ClientRunOutcome { result, exported, output })
    }

    /// The caller's extras as the launcher applies them, with the client's home servers resolved.
    fn extras_of(&self, request: &ClientRunRequest) -> Result<LaunchExtras, GatewayError> {
        let extras = &request.extras;
        let home_servers = if extras.disable_home_mcp == Some(true) { home_mcp_servers(self.files, self.env)? } else { Vec::new() };
        Ok(LaunchExtras {
            agent: extras.agent.clone(),
            env: extras.env.clone().unwrap_or_default(),
            home_servers,
            mcp_servers: extras.mcp_servers.clone().unwrap_or_default(),
            sandbox: extras.sandbox,
            schema: extras.schema.clone(),
            tools: extras.tools.clone().unwrap_or_default(),
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
