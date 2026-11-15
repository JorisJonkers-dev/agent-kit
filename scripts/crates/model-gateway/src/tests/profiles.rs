//! Delegation profiles: how the shipped seeds hold against a private, a public and a nedap repository, how a
//! profile is read and refused, and the limits a client cannot enforce. The goldens carry the runs and the
//! `models` listing; these tests carry what no golden reaches.

use crate::allowlist::AllowlistEntry;
use crate::config::{CostClass, GatewayConfig, Tier, ToolList, ToolScope};
use crate::config_parse::parse_defaults;
use crate::json::{self, parse};
use crate::policy::{decide_policy, DataScope, PolicyDecision, PolicySubject};
use crate::profiles::{effective_tools, hold_to_profile, list_profiles, plain_name, profile_models, require_profile, resolve_profile, run_scope, tools_json, unenforceable, Narrowing, ProfileModel};

fn shipped() -> GatewayConfig {
    parse_defaults(&parse(crate::SHIPPED_DEFAULTS).expect("JSON")).expect("the shipped defaults")
}

/// Three clients and a few models of three families: enough to state any profile.
fn custom(profiles: &str) -> Result<GatewayConfig, crate::error::GatewayError> {
    let text = format!(
        r#"{{
  "version": 1,
  "clients": {{
    "claude": {{"binary": "claude", "cost_class": "subsidised", "model_source": "declared"}},
    "pi-vibe": {{"binary": "pi", "cost_class": "subsidised", "model_source": "live"}},
    "vibe": {{"binary": "vibe", "cost_class": "metered-personal", "model_source": "declared"}}
  }},
  "models": [
    {{"client": "claude", "model": "opus", "family": "anthropic", "tier": "frontier", "notes": "n"}},
    {{"client": "claude", "model": "haiku", "family": "anthropic", "tier": "routine", "notes": "n"}},
    {{"client": "pi-vibe", "model": "small", "family": "mistral", "tier": "trivial", "notes": "n"}},
    {{"client": "pi-vibe", "model": "big", "family": "zhipu", "tier": "hard", "notes": "n"}},
    {{"client": "vibe", "model": "paid", "family": "zhipu", "tier": "trivial", "notes": "n"}}
  ],
  "profiles": {profiles},
  "settings": {{
    "availability_window_minutes": 60, "max_batch_delegations": 50, "max_parallel_per_client": 3,
    "metered_personal_caps": {{"per_day_usd": 5, "per_workflow_usd": 2}},
    "output_allowance_tokens": {{"frontier": 16000, "hard": 6000, "routine": 2000, "trivial": 500}},
    "plan_warning_ratio": 0.8, "summary_max_chars": 1500
  }}
}}"#
    );
    parse_defaults(&parse(&text).expect("JSON"))
}

fn refused_config(profiles: &str) -> String {
    custom(profiles).expect_err("a refused profile").message
}

fn decision(config: &GatewayConfig, scope: &DataScope) -> PolicyDecision {
    let subjects: Vec<PolicySubject> =
        config.models.iter().map(|model| PolicySubject { client: model.client.clone(), model: model.model.clone(), family: model.family.clone() }).collect();
    decide_policy(config, scope, &subjects)
}

fn repository(families: &[&str]) -> DataScope {
    let entry = AllowlistEntry { families: families.iter().map(|family| family.to_string()).collect(), code: true, history: false };
    DataScope::Repository { key: "github.com/owner/repo".to_string(), entry: Some(entry) }
}

fn keys(models: &[ProfileModel]) -> Vec<String> {
    models.iter().map(|model| format!("{}:{}", model.client, model.model)).collect()
}

fn pi_models(config: &GatewayConfig, name: &str, scope: &DataScope, narrow: Option<&Narrowing>) -> crate::profiles::Resolution {
    profile_models(config, &decision(config, scope), name, narrow).expect("a known profile")
}

const EVERY_FAMILY: [&str; 6] = ["zhipu", "mistral", "deepseek", "moonshot", "alibaba", "google"];

#[test]
fn the_shipped_defaults_state_the_four_seed_profiles() {
    let config = shipped();
    let names: Vec<&str> = config.profiles.keys().map(String::as_str).collect();
    assert_eq!(names, vec!["cheap-research", "cross-critic", "implement-cheap", "public-free"]);
    for profile in config.profiles.values() {
        assert_eq!(profile.tools, ToolScope { mcp: ToolList::Only(vec![]), plugins: ToolList::Only(vec![]), skills: ToolList::Only(vec![]) });
        assert!(profile.max_spend_usd.is_some());
    }
    assert_eq!(config.profiles["cross-critic"].distinct_families, Some(2.0));
    assert_eq!(config.profiles["cross-critic"].exclude_writer_family, Some(true));
    assert_eq!(config.profiles["implement-cheap"].ladder.as_ref().map(Vec::len), Some(4));
}

#[test]
fn every_seed_profile_has_a_model_with_no_repository_or_a_public_one_and_none_for_a_private_or_nedap_one() {
    let config = shipped();
    let writer = Narrowing { writer_family: Some("mistral".to_string()), ..Narrowing::default() };
    for name in ["cheap-research", "cross-critic", "implement-cheap", "public-free"] {
        let narrow = (name == "cross-critic").then_some(&writer);
        // No repository: only inline content, to subsidised clients.
        assert!(pi_models(&config, name, &DataScope::Outside, narrow).error.is_none(), "{name} outside");
        // A private repository allowed Anthropic alone: no Pi model is a family it allows.
        let private = pi_models(&config, name, &repository(&["anthropic"]), narrow);
        assert!(private.models.is_empty(), "{name} private");
        assert!(private.error.expect("an error").contains(&format!("profile {name} has no allowed model")));
        // A nedap repository reaches Claude Code and Codex alone, and no seed lists either.
        let nedap = pi_models(&config, name, &DataScope::Nedap, narrow);
        assert!(nedap.models.is_empty(), "{name} nedap");
        assert!(nedap.error.expect("an error").contains("a nedap repository"));
        // A public repository allowed every family: the profiles on a private route get it, the public route never does.
        let public = pi_models(&config, name, &repository(&EVERY_FAMILY), narrow);
        assert_eq!(public.models.is_empty(), name == "public-free", "{name} public");
    }
}

#[test]
fn a_seed_profile_lists_its_models_cheapest_first_inside_its_own_models() {
    let config = shipped();
    let listed = pi_models(&config, "implement-cheap", &repository(&EVERY_FAMILY), None);
    assert_eq!(keys(&listed.models), vec!["pi-vibe:deepseek-v4.1-flash", "pi-vibe:glm-5.3-flash", "pi-vibe:mistral-large-4", "pi-vibe:glm-5.3"]);
    let narrow = Narrowing { models: Some(vec!["pi-vibe:glm-5.3".to_string()]), ..Narrowing::default() };
    assert_eq!(keys(&pi_models(&config, "implement-cheap", &repository(&EVERY_FAMILY), Some(&narrow)).models), vec!["pi-vibe:glm-5.3"]);
}

#[test]
fn a_profile_that_leaves_out_the_writers_family_and_spans_two_families() {
    let config = shipped();
    let scope = repository(&EVERY_FAMILY);
    let mistral = Narrowing { writer_family: Some("mistral".to_string()), ..Narrowing::default() };
    let left = pi_models(&config, "cross-critic", &scope, Some(&mistral));
    assert!(left.models.iter().all(|model| model.family != "mistral"));
    assert!(left.error.is_none());
    // Without the writer named, a listing keeps every model; a run is refused.
    assert!(pi_models(&config, "cross-critic", &scope, None).error.expect("a run needs the writer").contains("narrow.writer_family"));
    // Leaving out the one family that remains leaves nothing; two families are needed.
    let only_zhipu = repository(&["zhipu"]);
    let writer = Narrowing { writer_family: Some("zhipu".to_string()), ..Narrowing::default() };
    assert!(pi_models(&config, "cross-critic", &only_zhipu, Some(&writer)).error.expect("none left").contains("once the writer's family zhipu is left out"));
    let other = Narrowing { writer_family: Some("mistral".to_string()), ..Narrowing::default() };
    assert!(pi_models(&config, "cross-critic", &only_zhipu, Some(&other)).error.expect("one family").contains("needs 2 distinct families but only 1 remain (zhipu)"));
}

#[test]
fn a_profile_is_read_with_its_tool_scope_and_every_field() {
    let config = custom(
        r#"{"all": {"description": "d", "models": ["pi-vibe:small", "pi-vibe:big"], "pin": "pi-vibe:big", "ladder": ["pi-vibe:small", "pi-vibe:big"],
            "distinct_families": 2, "exclude_writer_family": false, "max_spend_usd": 1.5,
            "tools": {"mcp": "any", "skills": ["review"]}},
           "bare": {"description": "d", "models": ["pi-vibe:small"]}}"#,
    )
    .expect("two profiles");
    let all = &config.profiles["all"];
    assert_eq!(all.tools, ToolScope { mcp: ToolList::Any, plugins: ToolList::Only(vec![]), skills: ToolList::Only(vec!["review".to_string()]) });
    assert_eq!(all.pin.as_deref(), Some("pi-vibe:big"));
    assert_eq!(config.profiles["bare"].tools.mcp, ToolList::Only(vec![]));
    assert!(custom("{}").expect("none").profiles.is_empty());
}

#[test]
fn a_profile_that_is_malformed_is_refused_with_what_is_wrong() {
    let one = |body: &str| format!(r#"{{"p": {body}}}"#);
    let ok = r#""description": "d", "models": ["pi-vibe:small"]"#;
    assert_eq!(refused_config("[]"), "profiles must be an object keyed by profile name");
    assert!(refused_config(r#"{"Bad_Name": {}}"#).contains("a profile name is lowercase letters, digits and hyphens"));
    assert!(refused_config(r#"{"": {}}"#).contains("a profile name is lowercase"));
    assert_eq!(refused_config(r#"{"p": 1}"#), "profiles.p must be an object");
    assert!(refused_config(&one(&format!("{{{ok}, \"surprise\": 1}}"))).contains("profiles.p.surprise is not a field a profile has"));
    assert_eq!(refused_config(&one(r#"{"models": ["pi-vibe:small"]}"#)), "profiles.p.description must be a non-empty string");
    assert_eq!(refused_config(&one(r#"{"description": "", "models": ["pi-vibe:small"]}"#)), "profiles.p.description must be a non-empty string");
    assert_eq!(refused_config(&one(r#"{"description": "d"}"#)), "profiles.p.models must be a list of non-empty strings");
    assert_eq!(refused_config(&one(r#"{"description": "d", "models": [""]}"#)), "profiles.p.models must be a list of non-empty strings");
    assert_eq!(refused_config(&one(r#"{"description": "d", "models": []}"#)), "profiles.p.models must not be empty");
    assert_eq!(refused_config(&one(r#"{"description": "d", "models": ["pi-vibe:small", "pi-vibe:small"]}"#)), "profiles.p.models must not repeat an entry");
    assert_eq!(refused_config(&one(r#"{"description": "d", "models": ["pi-vibe:nothing"]}"#)), "profiles.p.models names pi-vibe:nothing, which is no model of the catalogue");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"pin\": \"pi-vibe:big\"}}"))), "profiles.p.pin must name one of the profile's models");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"pin\": 3}}"))), "profiles.p.pin must name one of the profile's models");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"ladder\": [\"pi-vibe:big\"]}}"))), "profiles.p.ladder must name one of the profile's models");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"ladder\": \"x\"}}"))), "profiles.p.ladder must be a list of non-empty strings");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"distinct_families\": 0}}"))), "profiles.p.distinct_families must be a whole number of at least 1");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"distinct_families\": \"2\"}}"))), "profiles.p.distinct_families must be a whole number of at least 1");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"exclude_writer_family\": 1}}"))), "profiles.p.exclude_writer_family must be true or false");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"max_spend_usd\": 0}}"))), "profiles.p.max_spend_usd must be a number above 0");
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"tools\": 1}}"))), "profiles.p.tools must be an object");
    assert!(refused_config(&one(&format!("{{{ok}, \"tools\": {{\"shell\": []}}}}"))).contains("profiles.p.tools.shell is not a kind of tool"));
    assert_eq!(refused_config(&one(&format!("{{{ok}, \"tools\": {{\"mcp\": [1]}}}}"))), "profiles.p.tools.mcp must be a list of non-empty strings");
}

#[test]
fn a_profile_pinned_to_one_model_allows_only_it() {
    let config = custom(r#"{"pinned": {"description": "d", "models": ["pi-vibe:small", "pi-vibe:big"], "pin": "pi-vibe:big"}}"#).expect("pinned");
    let scope = repository(&["zhipu", "mistral"]);
    assert_eq!(keys(&pi_models(&config, "pinned", &scope, None).models), vec!["pi-vibe:big"]);
    let elsewhere = pi_models(&config, "pinned", &repository(&["mistral"]), None);
    assert!(elsewhere.error.expect("pinned out").contains("(pinned to pi-vibe:big)"));
    let small = Narrowing { models: Some(vec!["pi-vibe:small".to_string()]), ..Narrowing::default() };
    assert!(pi_models(&config, "pinned", &scope, Some(&small)).models.is_empty());
}

#[test]
fn the_delegate_picks_by_cost_class_then_tier_then_the_profiles_order() {
    let config = custom(r#"{"mixed": {"description": "d", "models": ["vibe:paid", "claude:opus", "pi-vibe:big", "claude:haiku", "pi-vibe:small"]}}"#).expect("mixed");
    let scope = repository(&["zhipu", "mistral", "anthropic"]);
    // vibe is metered-personal, so it sorts after the three subsidised clients, then by tier.
    assert_eq!(keys(&pi_models(&config, "mixed", &scope, None).models), vec!["pi-vibe:small", "claude:haiku", "pi-vibe:big", "claude:opus", "vibe:paid"]);
    let unknown_tier = ProfileModel { client: "pi-vibe".into(), model: "big".into(), family: "zhipu".into(), tier: None, cost_class: CostClass::Subsidised };
    let known_tier = ProfileModel { client: "pi-vibe".into(), model: "small".into(), family: "mistral".into(), tier: Some(Tier::Trivial), cost_class: CostClass::Subsidised };
    let profile = require_profile(&config, "mixed").expect("mixed");
    let resolution = resolve_profile(profile, &[unknown_tier, known_tier], "reason", None, true);
    assert_eq!(keys(&resolution.models), vec!["pi-vibe:small", "pi-vibe:big"]);
}

#[test]
fn a_profile_that_is_not_stated_is_unknown_and_the_error_says_what_is() {
    let config = shipped();
    let error = require_profile(&config, "nowhere").expect_err("unknown");
    assert_eq!(error.code, "unknown-profile");
    assert!(error.message.ends_with("the defaults state cheap-research, cross-critic, implement-cheap, public-free"));
    let none = custom("{}").expect("none");
    assert!(require_profile(&none, "x").expect_err("unknown").message.ends_with("the defaults state none"));
}

#[test]
fn a_client_that_cannot_enforce_a_restricted_scope_is_refused_and_pi_holds_only_the_empty_scope() {
    let config = custom(
        r#"{"deny": {"description": "d", "models": ["claude:haiku", "pi-vibe:small"]},
            "open": {"description": "d", "models": ["claude:haiku"], "tools": {"mcp": "any", "skills": "any", "plugins": "any"}},
            "grants": {"description": "d", "models": ["pi-vibe:small"], "tools": {"mcp": ["github"], "skills": ["a", "b"]}},
            "mixed": {"description": "d", "models": ["pi-vibe:small"], "tools": {"mcp": "any"}},
            "listed": {"description": "d", "models": ["pi-vibe:small"], "tools": {"plugins": ["hello"], "skills": ["a", "b"]}},
            "pathy": {"description": "d", "models": ["pi-vibe:small"], "tools": {"plugins": ["../x"], "skills": ["a/b"]}}}"#,
    )
    .expect("profiles");
    let decision = decision(&config, &repository(&["anthropic", "mistral"]));
    let hold = |name: &str, client: &str, model: &str, is_pi: bool, narrow: Option<&Narrowing>| hold_to_profile(&config, &decision, name, narrow, client, model, is_pi);
    // Deny-all on a client that is not Pi: refused, naming every restricted kind.
    let refused = hold("deny", "claude", "haiku", false, None).expect_err("claude cannot enforce");
    assert_eq!(refused.code, "profile-refused");
    assert_eq!(
        refused.message,
        "profile deny restricts mcp, plugins, skills (a scope is deny-all unless the profile lists it), and claude cannot enforce a tool scope: only Pi does"
    );
    // The empty scope holds on Pi.
    assert!(hold("deny", "pi-vibe", "small", true, None).is_ok());
    // A scope that is `any` everywhere restricts nothing, so any client holds it.
    assert!(hold("open", "claude", "haiku", false, None).is_ok());
    // Pi cannot hold an MCP server, and says why.
    let pi = hold("grants", "pi-vibe", "small", true, None).expect_err("pi cannot attach an MCP server");
    assert_eq!(
        pi.message,
        "profile grants grants mcp github, and pi-vibe cannot enforce it: Pi has a built-in MCP client, but a confined launch reaches only the route's proxy port and starts no other program, so a listed server could never connect; mcp stays deny-all on Pi"
    );
    // Narrowing takes the MCP server away, and the rest of the scope then holds: Pi builds its home from the skills.
    let none = Narrowing { mcp: Some(vec![]), ..Narrowing::default() };
    assert!(hold("grants", "pi-vibe", "small", true, Some(&none)).is_ok());
    // Skills and plugins are held by name.
    assert!(hold("listed", "pi-vibe", "small", true, None).is_ok());
    // `any` is refused on Pi: its home is built only from named items.
    let open = hold("mixed", "pi-vibe", "small", true, None).expect_err("any is refused on Pi");
    assert_eq!(
        open.message,
        "profile mixed leaves mcp open with any, and pi-vibe builds its per-run home only from named items: list the mcp it needs, or leave them out for deny-all"
    );
    // A skill or plugin must be a plain name, never a path.
    assert!(hold("pathy", "pi-vibe", "small", true, None).expect_err("a path").message.contains("lists skills entry a/b, which is not a plain name"));
    let no_skills = Narrowing { skills: Some(vec![]), ..Narrowing::default() };
    assert!(hold("pathy", "pi-vibe", "small", true, Some(&no_skills)).expect_err("a path").message.contains("lists plugins entry ../x, which is not a plain name: a plugin is named by"));
    assert!(plain_name("a.b_c-1") && !plain_name(".hidden") && !plain_name(""));
    // The scope a run holds is the profile's narrowed, in a fixed order, and none without a profile.
    let narrowed = run_scope(&config, Some("listed"), Some(&Narrowing { skills: Some(vec!["a".to_string()]), ..Narrowing::default() })).expect("scope");
    assert_eq!(json::compact(&tools_json(&narrowed)), r#"{"mcp":[],"plugins":["hello"],"skills":["a"]}"#);
    assert_eq!(json::compact(&tools_json(&run_scope(&config, Some("open"), None).expect("open"))), r#"{"mcp":"any","plugins":"any","skills":"any"}"#);
    assert!(run_scope(&config, None, None).is_none() && run_scope(&config, Some("nowhere"), None).is_none());
    // A narrowing may name a subset of what a profile grants, never more.
    let subset = Narrowing { skills: Some(vec!["a".to_string()]), mcp: Some(vec![]), ..Narrowing::default() };
    let tools = effective_tools(require_profile(&config, "grants").expect("grants"), Some(&subset));
    assert_eq!(tools.skills, ToolList::Only(vec!["a".to_string()]));
    let wider = Narrowing { skills: Some(vec!["c".to_string()]), ..Narrowing::default() };
    let widened = hold("grants", "pi-vibe", "small", true, Some(&wider)).expect_err("never widen");
    assert!(widened.message.contains("narrow.skills names c, which profile grants does not grant (its skills scope is a, b)"));
    // `any` in the profile lets a request narrow to a list.
    let to_list = Narrowing { mcp: Some(vec!["github".to_string()]), ..Narrowing::default() };
    assert!(hold("mixed", "pi-vibe", "small", true, Some(&to_list)).is_err());
    assert!(unenforceable("mixed", "pi-vibe", true, &effective_tools(require_profile(&config, "mixed").expect("mixed"), Some(&to_list))).is_some());
    // An unknown profile and a model outside the profile.
    assert_eq!(hold("nowhere", "pi-vibe", "small", true, None).expect_err("unknown").code, "unknown-profile");
    let outside = hold("deny", "pi-vibe", "big", true, None).expect_err("not the profile's model");
    assert!(outside.message.starts_with("pi-vibe:big is not allowed by profile deny here"));
    let empty = hold("deny", "pi-vibe", "small", true, Some(&Narrowing { models: Some(vec!["claude:haiku".to_string()]), ..Narrowing::default() }));
    assert!(empty.expect_err("narrowed away").message.contains("is not allowed by profile deny here"));
}

#[test]
fn the_listing_carries_every_field_a_profile_has_and_why_it_is_empty() {
    let config = custom(
        r#"{"full": {"description": "d", "models": ["pi-vibe:small", "pi-vibe:big"], "pin": "pi-vibe:small", "ladder": ["pi-vibe:small"],
            "distinct_families": 1, "exclude_writer_family": true, "max_spend_usd": 2, "tools": {"mcp": "any", "plugins": ["p"]}}}"#,
    )
    .expect("full");
    let available = vec![ProfileModel { client: "pi-vibe".into(), model: "small".into(), family: "mistral".into(), tier: Some(Tier::Trivial), cost_class: CostClass::Subsidised }];
    let listed = crate::json::pretty(&list_profiles(&config, &available, "the policy"));
    for expected in ["\"pin\": \"pi-vibe:small\"", "\"ladder\": [", "\"distinct_families\": 1", "\"exclude_writer_family\": true", "\"max_spend_usd\": 2", "\"mcp\": \"any\"", "\"plugins\": [\n        \"p\"\n      ]", "\"tier\": \"trivial\""] {
        assert!(listed.contains(expected), "{expected} in {listed}");
    }
    assert!(!listed.contains("\"error\""));
    let empty = crate::json::pretty(&list_profiles(&config, &[], "the policy"));
    assert!(empty.contains("\"models\": []") && empty.contains("\"error\": \"profile full has no allowed model (pinned to pi-vibe:small): none of its 1 models is both usable now and permitted by the data policy (the policy)\""));
}
