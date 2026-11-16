//! The LiteLLM proxy's management API, called with a short-lived management
//! key that is never stored, and the minting of each route's team and keys.

use crate::check::check;
use crate::http::{json_object, send, Req};
use crate::store::Store;
use crate::{field, id_text, is_safe, truthy, Ctx, Fail, R};
use serde_json::{json, Map, Value};

/// LiteLLM's sentinel for "whatever the key's team may use".
const TEAM_MODELS: &str = "all-team-models";
/// The proxy user whose one-hour management key setup mints agent keys with.
const SETUP_USER: &str = "agent-kit-setup";

/// LiteLLM's own reason for a refusal, short, for the message.
pub fn why(reply: &Value) -> String {
    let error = reply.get("error").filter(|e| truthy(e));
    let message = match error {
        Some(Value::Object(map)) => map.get("message").filter(|m| truthy(m)).cloned(),
        Some(other) => Some(other.clone()),
        None => reply.get("detail").filter(|d| truthy(d)).cloned(),
    };
    match message {
        Some(m) => format!(": {}", id_text(&m).chars().take(200).collect::<String>()),
        None => String::new(),
    }
}

pub struct LiteLLM {
    proxy: String,
    token: String,
}

/// Two JSON values are the same; 255 and 255.0 are one number.
pub fn same(a: &Value, b: &Value) -> bool {
    match (a, b) {
        (Value::Number(x), Value::Number(y)) => x.as_f64() == y.as_f64(),
        _ => a == b,
    }
}

impl LiteLLM {
    pub fn new(proxy: &str, token: &str) -> LiteLLM {
        LiteLLM {
            proxy: proxy.trim_end_matches('/').to_string(),
            token: token.to_string(),
        }
    }

    pub fn call(
        &self,
        ctx: &Ctx,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> R<(u16, Value)> {
        let url = format!("{}{path}", self.proxy);
        let req = Req {
            method,
            url: &url,
            headers: vec![
                ("Authorization", format!("Bearer {}", self.token)),
                ("Content-Type", "application/json".to_string()),
            ],
            body: body.map(Value::to_string),
            jar: None,
            timeout: 30,
        };
        let reply = send(ctx, &req).map_err(Fail::Unreachable)?;
        if (200..300).contains(&reply.status) {
            return Ok(match serde_json::from_slice::<Value>(&reply.body) {
                Ok(value) => (reply.status, value),
                Err(_) if reply.body.is_empty() => (reply.status, Value::Null),
                Err(_) => (0, Value::Null),
            });
        }
        Ok((reply.status, json_object(&reply.body)))
    }

    /// The id of the team `want` names, made or brought to `want`'s budget and access groups.
    pub fn team(&self, ctx: &mut Ctx, want: &Value) -> R<Option<String>> {
        let alias = field(want, "team_alias");
        let (code, reply) = self.call(ctx, "GET", "/team/list", None)?;
        let teams = reply
            .get("teams")
            .unwrap_or(&reply)
            .as_array()
            .filter(|_| code == 200);
        let Some(teams) = teams else {
            return Err(Fail::Refused(format!(
                "could not list the teams (HTTP {code}{})",
                why(&reply)
            )));
        };
        let found = teams
            .iter()
            .find(|t| t.is_object() && field(t, "team_alias") == alias);
        let Some(found) = found else {
            let (code, made) = self.call(ctx, "POST", "/team/new", Some(want))?;
            let id = made.get("team_id").filter(|i| truthy(i));
            return Ok(match id {
                Some(id) if code == 200 => {
                    ctx.say(&format!("litellm: made team {alias}"));
                    Some(id_text(id))
                }
                _ => {
                    ctx.say(&format!(
                        "litellm: team {alias} could not be made (HTTP {code})"
                    ));
                    None
                }
            });
        };
        // A route without a cap is a team without one: an old cap is cleared, not kept.
        let mut wanted = Map::new();
        wanted.insert("max_budget".into(), Value::Null);
        wanted.insert("budget_duration".into(), Value::Null);
        wanted.extend(want.as_object().cloned().unwrap_or_default());
        let mut drift = Map::new();
        for (key, value) in &wanted {
            let have = found.get(key).unwrap_or(&Value::Null);
            if key != "team_alias" && !same(have, value) {
                drift.insert(key.clone(), value.clone());
            }
        }
        let team_id = found.get("team_id").cloned().unwrap_or(Value::Null);
        if !drift.is_empty() {
            let names: Vec<&String> = drift.keys().collect();
            let names = names
                .iter()
                .map(|n| n.as_str())
                .collect::<Vec<_>>()
                .join(", ");
            let mut body = Map::new();
            body.insert("team_id".into(), team_id.clone());
            body.extend(drift);
            let (code, _) = self.call(ctx, "POST", "/team/update", Some(&Value::Object(body)))?;
            if code == 200 {
                ctx.say(&format!("litellm: set {names} on team {alias}"));
            } else {
                ctx.say(&format!("litellm: team {alias} could not be updated (HTTP {code}); it keeps its old settings"));
            }
        }
        Ok(Some(id_text(&team_id)))
    }

    pub fn holds(&self, ctx: &Ctx, alias: &str) -> R<bool> {
        let path = format!("/key/list?key_alias={}", encode(alias));
        let (code, reply) = self.call(ctx, "GET", &path, None)?;
        Ok(code == 200 && reply.get("keys").is_some_and(truthy))
    }

    pub fn delete(&self, ctx: &Ctx, alias: &str) -> R<bool> {
        let (code, _) = self.call(
            ctx,
            "POST",
            "/key/delete",
            Some(&json!({"key_aliases": [alias]})),
        )?;
        Ok(code == 200)
    }

    /// Whether this client's own key is scoped narrower than its team: a key minted before keys took
    /// `all-team-models` names its access groups itself, so a team change never reaches it. `false` when the
    /// proxy does not say.
    pub fn narrower_than_team(&self, ctx: &Ctx) -> R<Option<String>> {
        let (code, reply) = self.call(ctx, "GET", "/key/info", None)?;
        let models = reply.get("info").and_then(|info| info.get("models")).and_then(Value::as_array);
        Ok(match models {
            Some(models) if code == 200 && models.iter().any(|m| m.as_str() != Some(TEAM_MODELS)) => {
                Some(models.iter().filter_map(Value::as_str).collect::<Vec<_>>().join(", "))
            }
            _ => None,
        })
    }

    pub fn mint(&self, ctx: &Ctx, team_id: &str, alias: &str) -> R<String> {
        // The key takes its team's access groups (LiteLLM's all-team-models), so a team change needs no new key.
        let body = json!({
            "key_alias": alias,
            "team_id": team_id,
            "models": [TEAM_MODELS],
            "metadata": {"minted_by": "agent-kit setup"},
        });
        let (code, reply) = self.call(ctx, "POST", "/key/generate", Some(&body))?;
        Ok(match reply.get("key").and_then(Value::as_str) {
            Some(key) if code == 200 => key.to_string(),
            _ => String::new(),
        })
    }

    /// With the master key: the one-hour management key of the proxy user
    /// `agent-kit-setup` (role proxy_admin, no key of its own), made the first
    /// time. The same two steps as registry/generated/litellm/make-admin-token.sh.
    pub fn admin_token(&self, ctx: &Ctx) -> R<String> {
        let (code, _) = self.call(
            ctx,
            "GET",
            &format!("/user/info?user_id={SETUP_USER}"),
            None,
        )?;
        if code != 200 {
            let body = json!({"user_id": SETUP_USER, "user_role": "proxy_admin", "auto_create_key": false});
            let (code, reply) = self.call(ctx, "POST", "/user/new", Some(&body))?;
            if code != 200 {
                return Err(Fail::Refused(format!(
                    "could not make the {SETUP_USER} user (HTTP {code}{})",
                    why(&reply)
                )));
            }
        }
        let body = json!({
            "user_id": SETUP_USER,
            "key_type": "management",
            "duration": "1h",
            "metadata": {"purpose": "agent-kit setup mints agent keys"},
        });
        let (code, reply) = self.call(ctx, "POST", "/key/generate", Some(&body))?;
        match reply.get("key").and_then(Value::as_str) {
            Some(key) if code == 200 => Ok(key.to_string()),
            _ => Err(Fail::Refused(format!(
                "could not mint a management key (HTTP {code}{})",
                why(&reply)
            ))),
        }
    }
}

/// Percent-encodes a query value.
pub fn encode(value: &str) -> String {
    let mut out = String::new();
    for b in value.bytes() {
        if b.is_ascii_alphanumeric() || b"-._~".contains(&b) {
            out.push(b as char);
        } else {
            out += &format!("%{b:02X}");
        }
    }
    out
}

/// Converge every route's team; delete each route's old shared key; mint the keys in `wanted`.
pub fn mint_litellm(
    ctx: &mut Ctx,
    entries: &[&Value],
    wanted: &[String],
    token: &str,
    keys: &Store,
) -> R<()> {
    let host = ctx.host_name();
    for entry in entries {
        let spec = entry.get("litellm").unwrap_or(&Value::Null);
        let api = LiteLLM::new(field(spec, "proxy"), token);
        let team = spec.get("team").unwrap_or(&Value::Null);
        let agent = field(team, "team_alias").to_string();
        let env = field(entry, "env").to_string();
        match mint_one(ctx, &api, entry, team, &agent, &env, wanted, &host, keys) {
            Ok(()) => {}
            Err(Fail::Unreachable(e)) => {
                ctx.say(&format!(
                    "could not reach the LiteLLM proxy ({e}); the remaining keys were not minted"
                ));
                return Ok(());
            }
            Err(Fail::Refused(e)) => {
                ctx.say(&format!(
                    "litellm: {e}; nothing minted (registry/generated/litellm/make-admin-token.sh makes the token)"
                ));
                return Ok(());
            }
            Err(stop) => return Err(stop),
        }
    }
    Ok(())
}

#[allow(clippy::too_many_arguments)]
fn mint_one(
    ctx: &mut Ctx,
    api: &LiteLLM,
    entry: &Value,
    team: &Value,
    agent: &str,
    env: &str,
    wanted: &[String],
    host: &str,
    keys: &Store,
) -> R<()> {
    let Some(team_id) = api.team(ctx, team)? else {
        return Ok(());
    };
    // The key both machines once shared, by the route's own name: each host has its own now.
    if api.holds(ctx, agent)? {
        if api.delete(ctx, agent)? {
            ctx.say(&format!(
                "litellm: deleted the shared key {agent}; each host has its own key now"
            ));
        } else {
            ctx.say(&format!(
                "litellm: the shared key {agent} could not be deleted"
            ));
        }
    }
    if !wanted.iter().any(|w| w == env) {
        return Ok(());
    }
    let alias = format!("{agent}@{host}");
    // A key under this host's name that this store lost is useless: replace it.
    if api.holds(ctx, &alias)? && !api.delete(ctx, &alias)? {
        ctx.say(&format!(
            "{env}: the old key {alias} could not be deleted; nothing minted"
        ));
        return Ok(());
    }
    let key = api.mint(ctx, &team_id, &alias)?;
    if key.is_empty() || !is_safe(&key) {
        ctx.say(&format!(
            "{env}: the proxy did not mint a usable key for {alias}"
        ));
        return Ok(());
    }
    if check(ctx, entry, &key) == "invalid" {
        ctx.say(&format!(
            "{env}: the new key {alias} is refused by the proxy; not stored"
        ));
        api.delete(ctx, &alias)?;
        return Ok(());
    }
    keys.put(ctx, env, &key)?;
    ctx.say(&format!(
        "{env}: minted {alias} in team {agent}, checked and stored in the {}",
        keys.name
    ));
    Ok(())
}
