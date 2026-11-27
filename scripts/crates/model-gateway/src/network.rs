//! The network reach of one sandboxed run, and how the package proxy reaches it.
//!
//! A delegation profile names a reach (`open-web`, `packages` or `proxy-only`); this module turns it, with the
//! hosts the route and the task name, into the per-sandbox rules `sbx` can enforce. What sbx enforces was measured
//! on 0.54.1 (ADR-0019): a per-sandbox allow adds a host, a per-sandbox deny beats every allow in any scope and a
//! pattern deny (`**`) beats the sandbox's own allows, so a reach narrower than the machine's global allowlist is
//! made by denying, one by one, the global allows it does not need. Every sandbox also gets the estate package
//! proxy, with the package managers pointed at it; its key reaches the sandbox only as an sbx custom secret that
//! the host proxy injects into requests to that host, and the sandbox holds only the placeholder.

use crate::config::DelegationProfile;
use crate::error::GatewayError;
use crate::json::{self, Value};
use crate::ports::Files;
use crate::sha256::sha256_hex;
use std::collections::HashMap;

/// The estate package proxy every sandbox reaches, whatever its reach.
pub const PROXY_HOST: &str = "packages.jorisjonkers.dev";

/// The rule that allows the package proxy.
pub const PROXY_RULE: &str = "packages.jorisjonkers.dev:443";

/// The environment variable the workstation's proxy key is read from first, as setup reads it.
pub const KEY_VARIABLE: &str = "PACKAGES_PROXY_TOKEN";

/// The key's file under `XDG_CONFIG_HOME` (or `~/.config`) that setup stores a working key in.
pub const KEY_FILE: &str = "jorisjonkers/packages-token";

/// What a `packages` run reaches besides the route and the package proxy: the registries a build resolves from
/// (npm, PyPI, crates.io, Go), Docker Hub and ghcr for Docker inside the sandbox, and GitHub for reading.
pub const PACKAGE_HOSTS: [&str; 17] = [
    "registry.npmjs.org:443",
    "pypi.org:443",
    "files.pythonhosted.org:443",
    "crates.io:443",
    "index.crates.io:443",
    "static.crates.io:443",
    "proxy.golang.org:443",
    "sum.golang.org:443",
    "registry-1.docker.io:443",
    "auth.docker.io:443",
    "production.cloudflare.docker.com:443",
    "ghcr.io:443",
    "github.com:443",
    "api.github.com:443",
    "codeload.github.com:443",
    "raw.githubusercontent.com:443",
    "objects.githubusercontent.com:443",
];

/// How far a run's network reaches, from the widest to the narrowest.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub enum Reach {
    /// Any public host (research over public material).
    OpenWeb,
    /// The route, the package registries, GitHub read and the hosts the task names.
    Packages,
    /// The route alone: what a run gets unless its profile says otherwise.
    #[default]
    ProxyOnly,
}

impl Reach {
    /// The names a profile writes, widest first.
    pub const NAMES: [&'static str; 3] = ["open-web", "packages", "proxy-only"];

    pub fn parse(text: &str) -> Option<Reach> {
        match text {
            "open-web" => Some(Reach::OpenWeb),
            "packages" => Some(Reach::Packages),
            "proxy-only" => Some(Reach::ProxyOnly),
            _ => None,
        }
    }

    pub fn as_str(self) -> &'static str {
        match self {
            Reach::OpenWeb => "open-web",
            Reach::Packages => "packages",
            Reach::ProxyOnly => "proxy-only",
        }
    }
}

/// What a run's profile gives it for the network: its reach (cut when the data policy does not allow the open
/// web), and the sbx service secrets it names.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct NetworkScope {
    pub reach: Reach,
    pub secrets: Vec<String>,
    /// The reach the profile asked for when the data policy cut it.
    pub cut_from: Option<Reach>,
}

impl NetworkScope {
    /// The scope of a run of `profile` (none: the narrowest). `open_web_allowed` is false for a run whose data is
    /// not public, which keeps its repository's policy: open web reach is cut to `packages` for it.
    pub fn of(profile: Option<&DelegationProfile>, open_web_allowed: bool) -> NetworkScope {
        let asked = profile.and_then(|profile| profile.network).unwrap_or_default();
        let secrets = profile.and_then(|profile| profile.sbx_secrets.clone()).unwrap_or_default();
        if asked == Reach::OpenWeb && !open_web_allowed {
            return NetworkScope { reach: Reach::Packages, secrets, cut_from: Some(asked) };
        }
        NetworkScope { reach: asked, secrets, cut_from: None }
    }
}

/// The rules one sandbox gets: what to allow for it, what to deny of the machine's global allows, and the wider
/// global allows kept because a host the run needs sits under them.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Egress {
    pub allow: Vec<String>,
    pub deny: Vec<String>,
    pub kept: Vec<String>,
}

/// A `host` or `host:port` the task names, as a rule (port 443 when none). A pattern that opens a host family is
/// refused: only a profile's `open-web` reach does that.
pub fn extra_rule(entry: &str) -> Result<String, GatewayError> {
    let (host, port) = split_rule(entry);
    let plain = !host.is_empty() && host.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c == '.' || c == '-');
    let port_ok = port.is_none_or(|port| !port.is_empty() && port.chars().all(|c| c.is_ascii_digit()));
    if !plain || !port_ok || host.starts_with('.') || host.ends_with('.') {
        return Err(GatewayError::new("invalid-extras", format!("network_allow entry {entry:?} is not a host name (host or host:port, lowercase, no wildcard)")));
    }
    Ok(format!("{host}:{}", port.unwrap_or("443")))
}

/// The elements of a JSON array; `None` for anything else.
fn items(value: Option<&Value>) -> Option<&[Value]> {
    match value {
        Some(Value::Arr(items)) => Some(items),
        _ => None,
    }
}

/// The global network allows of a machine, from `sbx policy ls --json`.
pub fn global_allows(listing: &str) -> Result<Vec<String>, GatewayError> {
    let unreadable = || GatewayError::new("sandbox-unsafe", "sbx policy ls --json gave no readable rule list, so the run's network reach cannot be narrowed against the global allows");
    let value = json::parse(listing).map_err(|_| unreadable())?;
    let rules = value.as_object().and_then(|object| items(object.get("rules"))).ok_or_else(unreadable)?;
    let mut hosts = Vec::new();
    for rule in rules {
        let Some(rule) = rule.as_object() else { continue };
        let text = |key: &str| rule.get(key).and_then(Value::as_str).unwrap_or_default();
        if text("scope") != "global" || text("decision") != "allow" || text("resource_type") != "network" {
            continue;
        }
        hosts.extend(items(rule.get("resources")).into_iter().flatten().filter_map(Value::as_str).map(str::to_string));
    }
    Ok(hosts)
}

/// The sandbox's rules for a reach: the route, the package proxy, the reach's own hosts and the task's extras are
/// allowed; every global allow that none of them needs is denied. A narrower reach cannot be made when the global
/// policy allows every host (a deny of everything would take the sandbox's own allows with it), and that refuses.
pub fn plan(reach: Reach, route: &[String], extras: &[String], global: &[String]) -> Result<Egress, GatewayError> {
    let mut allow: Vec<String> = Vec::new();
    let mut add = |rule: &str| {
        if !allow.iter().any(|have| have == rule) {
            allow.push(rule.to_string());
        }
    };
    route.iter().for_each(|rule| add(rule));
    add(PROXY_RULE);
    if reach == Reach::OpenWeb {
        add("**");
    }
    if reach != Reach::ProxyOnly {
        PACKAGE_HOSTS.iter().for_each(|rule| add(rule));
    }
    extras.iter().for_each(|rule| add(rule));
    if reach == Reach::OpenWeb {
        return Ok(Egress { allow, deny: Vec::new(), kept: Vec::new() });
    }
    let (mut deny, mut kept) = (Vec::new(), Vec::new());
    for rule in global {
        if split_rule(rule).0.chars().all(|c| c == '*') {
            return Err(GatewayError::new(
                "sandbox-unsafe",
                format!("the global sbx policy allows every host ({rule}), so a {} reach cannot be enforced; remove that rule or give the run's profile network open-web", reach.as_str()),
            ));
        }
        match allow.iter().find(|wanted| rule_covers(rule, wanted)) {
            Some(wanted) if wanted != rule => kept.push(rule.clone()),
            Some(_) => {}
            None => deny.push(rule.clone()),
        }
    }
    Ok(Egress { allow, deny, kept })
}

/// Whether a global allow `pattern` also allows the `wanted` rule: the same port (or none named) and a host the
/// pattern matches, where `**` matches any characters and `*` and `?` match within one label.
pub fn rule_covers(pattern: &str, wanted: &str) -> bool {
    let (pattern_host, pattern_port) = split_rule(pattern);
    let (host, port) = split_rule(wanted);
    (pattern_port.is_none() || pattern_port == port) && glob(pattern_host.as_bytes(), host.as_bytes())
}

fn split_rule(rule: &str) -> (&str, Option<&str>) {
    match rule.rsplit_once(':') {
        Some((host, port)) if !host.contains(']') && port.bytes().all(|byte| byte.is_ascii_digit()) => (host, Some(port)),
        _ => (rule, None),
    }
}

fn glob(pattern: &[u8], text: &[u8]) -> bool {
    match pattern.split_first() {
        None => text.is_empty(),
        Some((b'*', rest)) if rest.first() == Some(&b'*') => (0..=text.len()).any(|at| glob(&rest[1..], &text[at..])),
        Some((b'*', rest)) => (0..=text.len()).take_while(|at| text[..*at].iter().all(|byte| *byte != b'.')).any(|at| glob(rest, &text[at..])),
        Some((b'?', rest)) => text.split_first().is_some_and(|(byte, tail)| *byte != b'.' && glob(rest, tail)),
        Some((byte, rest)) => text.split_first().is_some_and(|(head, tail)| head == byte && glob(rest, tail)),
    }
}

/// Refuses while sbx holds a global service secret the profile does not name: sbx gives every sandbox a proxy-managed
/// placeholder for each service secret it stores (`GH_TOKEN` for `github`), so the credential would reach this run.
/// A secret scoped to one sandbox, a registry credential (never injected) and a custom secret are not service secrets.
pub fn require_no_service_secrets(listing: &str, named: &[String]) -> Result<(), GatewayError> {
    let value = json::parse(listing).map_err(|_| GatewayError::new("sandbox-unsafe", "sbx secret ls --json gave no readable secret list, so the run cannot be checked for stored service secrets"))?;
    let secrets = value.as_object().and_then(|object| items(object.get("secrets"))).ok_or_else(|| GatewayError::new("sandbox-unsafe", "sbx secret ls --json has no secrets list, so the run cannot be checked for stored service secrets"))?;
    let stored: Vec<String> = secrets
        .iter()
        .filter_map(Value::as_object)
        .filter(|secret| secret.get("type").and_then(Value::as_str).is_none_or(|kind| kind == "service"))
        .filter_map(|secret| secret.get("name").and_then(Value::as_str))
        .filter(|name| !named.iter().any(|allowed| allowed == name))
        .map(str::to_string)
        .collect();
    if stored.is_empty() {
        return Ok(());
    }
    Err(GatewayError::new(
        "sandbox-unsafe",
        format!(
            "sbx stores a global service secret ({}), and sbx gives every sandbox a proxy-managed placeholder for it, so the credential would reach this run; remove it with sbx secret rm {} or name it in the profile's sbx_secrets",
            stored.join(", "),
            stored.first().map_or("", String::as_str)
        ),
    ))
}

/// The placeholder one sandbox carries for the proxy key: derived from the sandbox's name, so it is useless beyond
/// that sandbox's own secret.
pub fn placeholder(sandbox: &str) -> String {
    format!("mgpk-{}", &sha256_hex(sandbox.as_bytes())[..24])
}

/// The workstation's proxy key, as setup reads it: the environment variable, else the key file setup stores.
/// `None` where this machine has none: the run then gets no credential, and anonymous paths may still work.
pub fn proxy_key(files: &dyn Files, env: &HashMap<String, String>) -> Option<String> {
    let from_env = env.get(KEY_VARIABLE).map(|value| value.trim().to_string()).filter(|value| !value.is_empty());
    from_env.or_else(|| {
        let config = env.get("XDG_CONFIG_HOME").filter(|dir| !dir.is_empty()).cloned().or_else(|| env.get("HOME").map(|home| format!("{home}/.config")))?;
        files.read_text(&format!("{config}/{KEY_FILE}")).ok().map(|text| text.trim().to_string()).filter(|key| !key.is_empty())
    })
}

/// The package managers' settings that point them at the proxy, as `NAME=value` for the sandbox's env file. With a
/// placeholder the managers send it as their credential and the host proxy swaps in the key on the way out; without
/// one they go anonymously. `npmrc` is the path of the run's own npm user config (see [`npmrc_text`]).
pub fn package_env(placeholder: Option<&str>, npmrc: &str) -> Vec<String> {
    let auth = placeholder.map(|token| format!("packages:{token}@")).unwrap_or_default();
    let mut env = vec![
        format!("GOPROXY=https://{auth}{PROXY_HOST}/go"),
        "GOPRIVATE=github.com/JorisJonkers-dev/*".to_string(),
        format!("PIP_INDEX_URL=https://{auth}{PROXY_HOST}/pypi/simple/"),
        format!("UV_DEFAULT_INDEX=https://{auth}{PROXY_HOST}/pypi/simple/"),
        format!("NPM_CONFIG_REGISTRY=https://{PROXY_HOST}/npm/"),
        format!("NPM_CONFIG_USERCONFIG={npmrc}"),
        format!("CARGO_REGISTRIES_JORISJONKERS_PACKAGES_INDEX=sparse+https://{PROXY_HOST}/cargo/"),
        "CARGO_SOURCE_CRATES_IO_REPLACE_WITH=jorisjonkers-packages".to_string(),
    ];
    if let Some(token) = placeholder {
        env.push(format!("CARGO_REGISTRIES_JORISJONKERS_PACKAGES_TOKEN={token}"));
        env.push("CARGO_REGISTRY_GLOBAL_CREDENTIAL_PROVIDERS=cargo:token".to_string());
    }
    env
}

/// The run's own npm user config: the proxy registry, and the placeholder as its token. Never the owner's `.npmrc`.
pub fn npmrc_text(placeholder: Option<&str>) -> String {
    let mut text = format!("registry=https://{PROXY_HOST}/npm/\n");
    if let Some(token) = placeholder {
        text.push_str(&format!("//{PROXY_HOST}/npm/:_authToken={token}\n"));
    }
    text
}

/// What the run records of the reach it got, in words that never claim more than the rules enforce.
pub fn describe(scope: &NetworkScope, egress: &Egress, credential: &str) -> String {
    let mut text = format!("network reach {}: allow {}", scope.reach.as_str(), egress.allow.join(","));
    if let Some(asked) = scope.cut_from {
        text.push_str(&format!("; cut from {} because the repository is not public", asked.as_str()));
    }
    if scope.reach == Reach::OpenWeb {
        text.push_str("; nothing of the machine's global policy is denied");
    } else {
        text.push_str(&format!("; denied {} global allows", egress.deny.len()));
        if !egress.kept.is_empty() {
            text.push_str(&format!("; kept wider global allows {} because a host the run needs sits under them", egress.kept.join(",")));
        }
    }
    text.push_str(&format!("; package proxy {PROXY_HOST}: {credential}"));
    text
}
