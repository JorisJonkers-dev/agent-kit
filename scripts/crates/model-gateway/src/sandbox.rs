//! Which sandbox a delegated client runs in. Every launch runs through `agent-launcher confine` (ADR-0016) until
//! the Docker Sandboxes backend lands (`sbx`: one microVM per sandbox, with a private Docker engine and a
//! host-side network proxy). Then sbx is required: a machine without it refuses the launch, and `confine` retires.
//! Until it is built, `AGENT_KIT_SANDBOX=sbx-dry-run` turns a launch into its sbx
//! plan: the gateway prepares the launch exactly as for `confine`, translates the grants the launcher would have
//! been given into what one sandbox would hold, removes what it made for the launch, and refuses it with the plan
//! as the reason. Nothing is started, and a machine that does not set the variable runs as before.
//!
//! The grants are the policy, and each backend renders them: `confine` as its own flags, `sbx` as the mounts of
//! one sandbox (`PATH` or `PATH:ro`, at the same path inside it), the variables passed in by name, and the network
//! reach. What the sbx side cannot hold the way `confine` does is listed as a gap, never dropped silently.

use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::ports::ProcessCommand;
use std::collections::HashMap;

/// The variable that picks the backend on this machine; unset or empty is `confine`.
pub const SANDBOX_VARIABLE: &str = "AGENT_KIT_SANDBOX";

/// The sandbox a launch runs in.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SandboxBackend {
    /// `agent-launcher confine`: Seatbelt on macOS, Landlock on Linux.
    Confine,
    /// Plan the launch for Docker Sandboxes and refuse it with the plan; nothing starts.
    SbxDryRun,
    /// Run the launch in its own Docker Sandbox (`sbx`), for the read-only Pi and OpenCode clients.
    Sbx,
}

impl SandboxBackend {
    pub fn as_str(self) -> &'static str {
        match self {
            SandboxBackend::Confine => "confine",
            SandboxBackend::SbxDryRun => "sbx-dry-run",
            SandboxBackend::Sbx => "sbx",
        }
    }

    /// The backend `AGENT_KIT_SANDBOX` names, or why the value names none. A value the gateway does not know is
    /// refused rather than read as `confine`, so a typo never decides how a delegate is held.
    pub fn select(env: &HashMap<String, String>) -> Result<SandboxBackend, GatewayError> {
        match env.get(SANDBOX_VARIABLE).map_or("", String::as_str) {
            "" | "confine" => Ok(SandboxBackend::Confine),
            "sbx-dry-run" => Ok(SandboxBackend::SbxDryRun),
            "sbx" => Ok(SandboxBackend::Sbx),
            other => Err(GatewayError::new("sandbox-unknown", format!("{SANDBOX_VARIABLE}={other} names no sandbox backend; use confine, sbx-dry-run or sbx"))),
        }
    }
}

/// One host path the sandbox sees, at the same path inside it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Mount {
    pub path: String,
    pub read_only: bool,
}

impl Mount {
    /// The mount as `sbx run` and `sbx create` name it: `PATH`, or `PATH:ro`.
    pub fn arg(&self) -> String {
        if self.read_only {
            format!("{}:ro", self.path)
        } else {
            self.path.clone()
        }
    }
}

/// What one sandbox would hold for one launch.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct SbxPlan {
    pub client: String,
    /// The Docker Hub repository of the image the sandbox starts from: the client's own install lives there, not on a mount of the host's.
    pub template: String,
    /// The directory the client starts in.
    pub workdir: String,
    pub mounts: Vec<Mount>,
    /// The host programs `confine` would have let run; in a sandbox they come from the template instead.
    pub programs: Vec<String>,
    /// The variables passed into the sandbox, by name; their values are never part of a plan.
    pub env: Vec<String>,
    pub ports: Vec<u16>,
    pub loopback: bool,
    /// What `confine` holds that this plan does not hold the same way.
    pub gaps: Vec<String>,
}

impl SbxPlan {
    /// The plan of a command behind `agent-launcher confine`, read from the grants between `confine` and `--`;
    /// `None` for a command that is not behind the launcher.
    pub fn of_command(client: &str, command: &ProcessCommand) -> Option<SbxPlan> {
        if command.args.first().map(String::as_str) != Some("confine") {
            return None;
        }
        let end = command.args.iter().position(|word| word == "--")?;
        let template = crate::sbx_images::image_of(client).map_or_else(String::new, |image| format!("docker.io/sbx/{}", image.repo));
        let mut plan = SbxPlan { client: client.to_string(), template, workdir: command.cwd.clone().unwrap_or_default(), ..SbxPlan::default() };
        let mut words = command.args[1..end].iter();
        while let Some(flag) = words.next() {
            match flag.as_str() {
                "--net-loopback" => plan.loopback = true,
                "--keychain" => plan.gaps.push("keychain: the client signs in through the macOS login keychain, which no sandbox reaches; its credential has to be handed in (a proxy-injected sbx secret, or a variable the gateway reads outside)".to_string()),
                // The managed preferences a macOS client asks for before it starts; a Linux sandbox has none to ask.
                "--preferences" => {}
                _ => {
                    let value = words.next().cloned().unwrap_or_default();
                    plan.grant(flag, value);
                }
            }
        }
        if !plan.ports.is_empty() {
            let ports = plan.ports.iter().map(u16::to_string).collect::<Vec<_>>().join(", ");
            plan.gaps.push(format!("network: confine allows TCP ports {ports} to any host; an sbx policy allows host names, which the plan does not derive yet"));
        }
        Some(plan)
    }

    fn grant(&mut self, flag: &str, value: String) {
        match flag {
            "--read" => self.mount(value, true),
            "--write" => self.mount(value, false),
            "--run" | "--run-from" => self.programs.push(value),
            "--env" => self.env.push(value),
            "--net-connect" => match value.parse() {
                Ok(port) => self.ports.push(port),
                Err(_) => self.gaps.push(format!("--net-connect {value} is not a port")),
            },
            other => self.gaps.push(format!("{other} {value}: a grant the plan does not know")),
        }
    }

    /// A path joins the mounts once; a path granted both ways is written.
    fn mount(&mut self, path: String, read_only: bool) {
        match self.mounts.iter_mut().find(|mount| mount.path == path) {
            Some(mount) => mount.read_only &= read_only,
            None => self.mounts.push(Mount { path, read_only }),
        }
    }

    pub fn to_json(&self) -> Value {
        let texts = |items: &[String]| Value::Arr(items.iter().map(|item| Value::str(item)).collect());
        json::object(vec![
            ("backend", Value::str(SandboxBackend::SbxDryRun.as_str())),
            ("client", Value::str(&self.client)),
            ("template", Value::str(&self.template)),
            ("workdir", Value::str(&self.workdir)),
            ("mounts", Value::Arr(self.mounts.iter().map(|mount| Value::str(&mount.arg())).collect())),
            ("programs", texts(&self.programs)),
            ("env", texts(&self.env)),
            ("ports", Value::Arr(self.ports.iter().map(|port| Value::Num(f64::from(*port))).collect())),
            ("loopback", Value::Bool(self.loopback)),
            ("gaps", texts(&self.gaps)),
        ])
    }
}

/// The refusal a dry run ends a launch with: the plan as JSON, or why there is none.
pub fn dry_run(client: &str, command: &ProcessCommand) -> GatewayError {
    match SbxPlan::of_command(client, command) {
        Some(plan) => GatewayError::new("sandbox-dry-run", format!("{client}: not started ({SANDBOX_VARIABLE}=sbx-dry-run); the sbx plan: {}", json::compact(&plan.to_json()))),
        None => GatewayError::new("sandbox-dry-run", format!("{client}: not started ({SANDBOX_VARIABLE}=sbx-dry-run); the launch is not behind agent-launcher confine, so there are no grants to plan from")),
    }
}
