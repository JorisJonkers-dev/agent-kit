//! What a caller inside this process adds to one run's client command. The
//! gateway holds no caller's rules: it applies these to the command, and
//! refuses the ones a client cannot apply. A caller that starts its own MCP
//! server, switches a client's home servers off, or needs a sandboxed client
//! says so here. Only an in-process caller sends them: the command line's
//! `run` refuses the field.

use std::collections::BTreeMap;

/// One MCP server a run attaches to its client.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct AttachedServer {
    pub name: String,
    pub command: String,
    pub args: Vec<String>,
}

/// The sandbox profiles a launcher maps to its client's own flags.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxProfile {
    ReadOnly,
}

/// Every extra a run may name, in the order a refusal lists them.
pub const EXTRA_NAMES: [&str; 7] = ["agent", "disable_home_mcp", "env", "mcp_servers", "sandbox", "schema", "tools"];

/// What a caller adds to one run, every part of it optional.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct RunExtras {
    /// The agent the client runs, where it names one (Vibe's `--agent`); the model is named beside it.
    pub agent: Option<String>,
    /// Switch off every MCP server the client's home declares, so only the attached ones can start.
    pub disable_home_mcp: Option<bool>,
    /// Extra environment for the client's process, under the launcher's own isolation, which wins.
    pub env: Option<BTreeMap<String, String>>,
    /// The MCP servers to attach to the client.
    pub mcp_servers: Option<Vec<AttachedServer>>,
    /// The sandbox profile the launcher maps to the client's own flags.
    pub sandbox: Option<SandboxProfile>,
    /// A JSON schema the client enforces on its structured output, where it can.
    pub schema: Option<String>,
    /// The tools the run may use; a client enables these and denies the rest.
    pub tools: Option<Vec<String>>,
}
