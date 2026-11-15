//! Checking a value against its own host, and the passphrase gate.

use crate::http::{send, Req};
use crate::store::{find_tool, Store};
use crate::{field, truthy, Ctx, R};
use serde_json::Value;
use std::io::Write;
use std::process::{Command, Stdio};

const MCP_INIT: &str = r#"{"jsonrpc":"2.0","id":1,"method":"initialize","params":{"protocolVersion":"2025-03-26","capabilities":{},"clientInfo":{"name":"agent-kit","version":"1"}}}"#;

/// ok, invalid, or unchecked (the host did not answer, or the entry says it cannot be asked).
pub fn check(ctx: &Ctx, entry: &Value, value: &str) -> &'static str {
    if !entry.get("check").is_some_and(truthy) {
        return "unchecked";
    }
    let get = field(entry, "check_method") == "GET";
    let req = Req {
        method: if get { "GET" } else { "POST" },
        url: field(entry, "check"),
        headers: vec![
            ("Authorization", format!("Bearer {value}")),
            ("Content-Type", "application/json".to_string()),
            ("Accept", "application/json, text/event-stream".to_string()),
        ],
        body: (!get).then(|| MCP_INIT.to_string()),
        jar: None,
        timeout: 20,
    };
    match send(ctx, &req) {
        Ok(reply) if reply.status == 200 => "ok",
        Ok(_) => "invalid",
        Err(_) => "unchecked",
    }
}

/// A gated value lives behind a passphrase only you know (installer/agent-kit-private).
pub fn gated(entry: &Value) -> bool {
    field(entry, "gate") == "passphrase"
}

/// Seal a value; the gate asks for the passphrase at the terminal itself.
pub fn gate_store(ctx: &Ctx, value: &str) -> bool {
    let tool = find_tool(ctx, "AGENT_KIT_PRIVATE_TOOL", "agent-kit-private");
    let child = Command::new(tool)
        .arg("store")
        .env_clear()
        .envs(&ctx.env)
        .stdin(Stdio::piped())
        .spawn();
    let Ok(mut child) = child else { return false };
    // Dropping the pipe after the write ends the helper's input.
    let _ = child
        .stdin
        .take()
        .map(|mut stdin| stdin.write_all(value.as_bytes()));
    child.wait().is_ok_and(|s| s.success())
}

pub fn state(ctx: &Ctx, entry: &Value, keys: &Store) -> R<&'static str> {
    if gated(entry) {
        let tool = find_tool(ctx, "AGENT_KIT_PRIVATE_TOOL", "agent-kit-private");
        let out = Command::new(tool)
            .arg("status")
            .env_clear()
            .envs(&ctx.env)
            .output();
        let present = out.is_ok_and(|o| String::from_utf8_lossy(&o.stdout).trim() == "present");
        return Ok(if present { "ok" } else { "missing" });
    }
    let value = keys.get(ctx, field(entry, "env"))?;
    Ok(if value.is_empty() {
        "missing"
    } else {
        check(ctx, entry, &value)
    })
}
