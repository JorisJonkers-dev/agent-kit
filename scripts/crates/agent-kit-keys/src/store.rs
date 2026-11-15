//! The one secret store, through the `agent-kit-secret` helper: the macOS
//! Keychain, or on Linux libsecret. The value goes over stdin, never argv. A
//! missing store stops the run; there is no plain-file fallback.

use crate::{config_dir, Ctx, Fail, R};
use std::io::Write;
use std::path::PathBuf;
use std::process::{Command, Output, Stdio};

/// A helper beside the kit this binary runs from, else the installed copy.
pub fn find_tool(ctx: &Ctx, env_name: &str, name: &str) -> PathBuf {
    if let Some(path) = ctx.env.get(env_name).filter(|p| !p.is_empty()) {
        return PathBuf::from(path);
    }
    let in_kit = ctx
        .exe
        .ancestors()
        .map(|dir| dir.join("installer").join(name))
        .find(|path| path.is_file());
    in_kit.unwrap_or_else(|| ctx.home.join(".local").join("bin").join(name))
}

pub struct Store {
    tool: PathBuf,
    pub name: &'static str,
}

impl Store {
    pub fn new(ctx: &Ctx) -> Store {
        let forced = ctx
            .env
            .get("AGENT_KIT_SECRETS_BACKEND")
            .cloned()
            .unwrap_or_default();
        let has_security = ctx
            .env
            .get("PATH")
            .is_some_and(|p| std::env::split_paths(p).any(|d| d.join("security").is_file()));
        let keychain = if forced.is_empty() {
            std::env::consts::OS == "macos" && has_security
        } else {
            forced == "keychain"
        };
        Store {
            tool: find_tool(ctx, "AGENT_KIT_SECRET_TOOL", "agent-kit-secret"),
            name: if keychain {
                "keychain"
            } else {
                "Secret Service"
            },
        }
    }

    fn run(&self, ctx: &Ctx, command: &str, var: &str, value: Option<&str>) -> R<Output> {
        let stop =
            |e: std::io::Error| Fail::Stop(format!("cannot run {}: {e}", self.tool.display()));
        let mut child = Command::new(&self.tool)
            .args([command, var])
            .env_clear()
            .envs(&ctx.env)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(stop)?;
        let _ = child
            .stdin
            .take()
            .map(|mut stdin| stdin.write_all(value.unwrap_or("").as_bytes()));
        let done = child.wait_with_output().map_err(stop)?;
        if done.status.code() == Some(2) {
            let said = String::from_utf8_lossy(&done.stderr);
            let said = said.trim();
            let said = said.strip_prefix("agent-kit-secret: ").unwrap_or(said);
            return Err(Fail::Stop(if said.is_empty() {
                "the secret store is not usable".to_string()
            } else {
                said.to_string()
            }));
        }
        Ok(done)
    }

    pub fn get(&self, ctx: &Ctx, var: &str) -> R<String> {
        let done = self.run(ctx, "get", var, None)?;
        Ok(if done.status.success() {
            String::from_utf8_lossy(&done.stdout).trim().to_string()
        } else {
            String::new()
        })
    }

    pub fn put(&self, ctx: &Ctx, var: &str, value: &str) -> R<()> {
        self.run(ctx, "put", var, Some(value))?;
        if self.get(ctx, var)? != value {
            return Err(Fail::Stop(format!(
                "{var} did not read back from the {}",
                self.name
            )));
        }
        Ok(())
    }

    pub fn delete(&self, ctx: &Ctx, var: &str) -> R<()> {
        self.run(ctx, "delete", var, None).map(|_| ())
    }

    /// Quiet on a missing key: env.sh exports only what is there.
    pub fn shell_reader(&self) -> &'static str {
        "_agent_kit_secret() { \"$HOME/.local/bin/agent-kit-secret\" get \"$1\" 2>/dev/null; }"
    }

    /// Move a secrets.env an earlier setup wrote into the store, then remove the plain file.
    pub fn migrate_legacy_file(&self, ctx: &mut Ctx) -> R<()> {
        let path = config_dir(ctx).join("secrets.env");
        if !path.is_file() {
            return Ok(());
        }
        let text = std::fs::read_to_string(&path).map_err(|e| Fail::Stop(e.to_string()))?;
        for (var, value) in text.lines().filter_map(|l| l.split_once('=')) {
            if !value.is_empty() && self.get(ctx, var)?.is_empty() {
                self.put(ctx, var, value)?;
            }
        }
        std::fs::remove_file(&path).map_err(|e| Fail::Stop(e.to_string()))?;
        ctx.say(&format!(
            "moved the keys in {} into the {} and removed the plain file",
            path.display(),
            self.name
        ));
        Ok(())
    }
}
