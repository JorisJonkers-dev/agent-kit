//! auth-api: sign in once (password, then TOTP), grant, mint and revoke service tokens.

use crate::http::{json_object, send, Jar, Req};
use crate::{ask, field, id_text, Ctx, Fail, R};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::process::Command;

pub struct Auth {
    base: String,
    jar: Jar,
}

impl Auth {
    pub fn new(ctx: &Ctx) -> R<Auth> {
        let base = ctx.env.get("AGENT_KIT_AUTH_URL").filter(|u| !u.is_empty());
        let base = base.map_or("https://auth.jorisjonkers.dev", String::as_str);
        let tmp = ctx.env.get("TMPDIR").map(String::as_str);
        let jar = Jar::new(tmp)
            .map_err(|e| Fail::Stop(format!("no private folder for the session: {e}")))?;
        Ok(Auth {
            base: format!("{}/api/v1", base.trim_end_matches('/')),
            jar,
        })
    }

    pub fn call(
        &self,
        ctx: &Ctx,
        method: &str,
        path: &str,
        body: Option<&Value>,
    ) -> R<(u16, Value)> {
        let mut headers = vec![("Content-Type", "application/json".to_string())];
        if let Some(xsrf) = self.jar.cookie("XSRF-TOKEN") {
            headers.push(("X-XSRF-TOKEN", xsrf));
        }
        let url = format!("{}{path}", self.base);
        let req = Req {
            method,
            url: &url,
            headers,
            body: body.map(Value::to_string),
            jar: Some(&self.jar.path()),
            timeout: 20,
        };
        let reply = send(ctx, &req).map_err(Fail::Unreachable)?;
        Ok((reply.status, json_object(&reply.body)))
    }

    pub fn sign_in(&self, ctx: &mut Ctx) -> R<Option<Value>> {
        let default = Command::new("git")
            .args(["config", "user.email"])
            .env_clear()
            .envs(&ctx.env)
            .output()
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_string())
            .unwrap_or_default();
        let hint = if default.is_empty() {
            String::new()
        } else {
            format!(" [{default}]")
        };
        let typed = ask(
            ctx,
            &format!("auth.jorisjonkers.dev username or email{hint}: "),
            false,
        );
        let user = if typed.is_empty() { default } else { typed };
        let password = ask(ctx, "password: ", true);
        let mut body = json!({"username": user, "password": password});
        let (mut code, mut reply) = self.call(ctx, "POST", "/auth/session-login", Some(&body))?;
        if code == 200 && reply.get("totpRequired").is_some_and(crate::truthy) {
            body["totpCode"] = Value::String(ask(ctx, "TOTP code: ", false));
            (code, reply) = self.call(ctx, "POST", "/auth/session-login", Some(&body))?;
        }
        if code != 200 || !reply.get("success").is_some_and(crate::truthy) {
            let why = match reply.get("detail").filter(|d| crate::truthy(d)) {
                Some(detail) => id_text(detail),
                None => format!("HTTP {code}"),
            };
            ctx.say(&format!("sign-in refused: {why}"));
            return Ok(None);
        }
        let (code, me) = self.call(ctx, "GET", "/auth/me", None)?;
        if code != 200 {
            ctx.say(&format!("signed in, but /auth/me answered HTTP {code}"));
            return Ok(None);
        }
        Ok(Some(me))
    }

    fn held(user: &Value) -> BTreeSet<String> {
        let list = user.get("servicePermissions").and_then(Value::as_array);
        list.into_iter().flatten().map(id_text).collect()
    }

    /// The grants held after granting what is missing; only an admin can grant.
    pub fn ensure_grants(
        &self,
        ctx: &mut Ctx,
        me: &Value,
        wanted: &BTreeSet<String>,
    ) -> R<BTreeSet<String>> {
        if field(me, "role") != "ADMIN" {
            // Minting tells us what is missing, with auth-api's own reason.
            return Ok(wanted.clone());
        }
        let id = id_text(me.get("id").unwrap_or(&Value::Null));
        let (code, user) = self.call(ctx, "GET", &format!("/admin/users/{id}"), None)?;
        let mut held = if code == 200 {
            Self::held(&user)
        } else {
            BTreeSet::new()
        };
        let missing: BTreeSet<String> = wanted.difference(&held).cloned().collect();
        if !missing.is_empty() {
            let all: Vec<&String> = held.union(&missing).collect();
            let body = json!({ "services": all });
            let (code, user) = self.call(
                ctx,
                "PUT",
                &format!("/admin/users/{id}/services"),
                Some(&body),
            )?;
            if code == 200 {
                held = Self::held(&user);
            }
            let granted: Vec<String> = missing.intersection(&held).cloned().collect();
            if !granted.is_empty() {
                ctx.say(&format!("granted yourself {}", granted.join(", ")));
            }
        }
        Ok(held)
    }

    /// (token, id, why it failed).
    pub fn mint(&self, ctx: &Ctx, grant: &str) -> R<(String, String, String)> {
        let label = format!("{} agent-kit", ctx.host.split('.').next().unwrap_or(""));
        let body = json!({"service": grant, "label": label});
        let (code, reply) = self.call(ctx, "POST", "/auth/service-tokens", Some(&body))?;
        let token = reply.get("token").filter(|t| crate::truthy(t));
        match token {
            Some(token) if code == 200 || code == 201 => {
                let id = reply
                    .get("id")
                    .filter(|i| crate::truthy(i))
                    .map(id_text)
                    .unwrap_or_default();
                Ok((id_text(token), id, String::new()))
            }
            _ => {
                let why = match reply.get("detail").filter(|d| crate::truthy(d)) {
                    Some(detail) => id_text(detail),
                    None => format!("HTTP {code}"),
                };
                Ok((String::new(), String::new(), why))
            }
        }
    }

    pub fn revoke(&self, ctx: &Ctx, ident: &str) -> R<bool> {
        let (code, _) = self.call(
            ctx,
            "DELETE",
            &format!("/auth/service-tokens/{ident}"),
            None,
        )?;
        Ok(code == 200 || code == 204)
    }

    pub fn sign_out(&self, ctx: &Ctx) -> R<()> {
        self.call(ctx, "GET", "/auth/logout", None).map(|_| ())
    }
}
