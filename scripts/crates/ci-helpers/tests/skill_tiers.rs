//! The skills that delegate name a LiteLLM tier and never a model (#909). The text a skill
//! installs is its `SKILL.md`, so the snapshot is that text: the section of each skill that
//! names its tier, in `tests/skill-tiers.snap`. `UPDATE_SNAPSHOTS=1 cargo test -p ci-helpers
//! --test skill_tiers` rewrites it after an intended change.

use std::fs;
use std::path::PathBuf;

use serde_json::Value;

/// Each skill, the heading of the section that names its tier, and the tiers it must name.
const SKILLS: [(&str, &str, &[&str]); 11] = [
    (
        "delegate",
        "Choose by tier",
        &[
            "tier-free",
            "tier-go-cheap",
            "tier-vibe",
            "tier-review",
            "tier-strong-open",
        ],
    ),
    ("research", "Model tier", &["tier-free", "tier-vibe"]),
    ("critique", "Model tier", &["tier-review"]),
    ("critique-adrs", "Model tier", &["tier-review"]),
    ("critique-plan", "Model tier", &["tier-review"]),
    ("review-architecture", "Model tier", &["tier-review"]),
    ("review-grill", "Model tier", &["tier-review"]),
    ("review-implementation", "Model tier", &["tier-review"]),
    ("review-pr", "Model tier", &["tier-review"]),
    ("review-spec", "Model tier", &["tier-review"]),
    ("review-tickets", "Model tier", &["tier-review"]),
];

fn root() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../..")
}

fn section(skill: &str, heading: &str) -> String {
    let path = root()
        .join("skills/workstation")
        .join(skill)
        .join("SKILL.md");
    let text = fs::read_to_string(&path).unwrap_or_else(|e| panic!("{}: {e}", path.display()));
    let start = text
        .find(&format!("\n## {heading}\n"))
        .unwrap_or_else(|| panic!("{skill} has no `## {heading}` section"));
    let rest = &text[start + 1..];
    let end = rest[3..].find("\n## ").map_or(rest.len(), |i| i + 3);
    rest[..end].trim_end().to_string()
}

fn known_tiers() -> Vec<String> {
    let text = fs::read_to_string(root().join("registry/litellm-tiers.json")).unwrap();
    let root: Value = serde_json::from_str(&text).unwrap();
    root["tier_groups"]
        .as_object()
        .unwrap()
        .keys()
        .cloned()
        .collect()
}

/// Every `tier-...` name a section writes.
fn tiers_named(text: &str) -> Vec<String> {
    let mut out = Vec::new();
    for word in text.split(|c: char| !(c.is_ascii_alphanumeric() || c == '-')) {
        if word.starts_with("tier-")
            && word != "tier-key-missing"
            && !out.contains(&word.to_string())
        {
            out.push(word.to_string());
        }
    }
    out
}

#[test]
fn every_delegating_skill_names_its_tier_and_it_is_a_real_one() {
    let known = known_tiers();
    for (skill, heading, expected) in SKILLS {
        let text = section(skill, heading);
        let named = tiers_named(&text);
        for tier in expected {
            assert!(
                named.iter().any(|n| n == tier),
                "{skill} does not name {tier}"
            );
        }
        for tier in &named {
            assert!(
                known.contains(tier),
                "{skill} names {tier}, which is no LiteLLM tier"
            );
        }
    }
}

#[test]
fn every_tier_section_reads_the_routing_record_and_keeps_claude_off_litellm() {
    for (skill, heading, _) in SKILLS {
        let text = section(skill, heading);
        for needle in [
            "ledger.jsonl",
            "served_model",
            "fallback_fired",
            "routing_capture",
            "Never send Claude traffic through LiteLLM",
        ] {
            assert!(
                text.contains(needle),
                "{skill}: the tier section lacks `{needle}`"
            );
        }
    }
}

#[test]
fn the_rendered_tier_sections_match_the_snapshot() {
    let mut rendered = String::new();
    for (skill, heading, _) in SKILLS {
        rendered.push_str(&format!(
            "=== skills/workstation/{skill}/SKILL.md ## {heading}\n"
        ));
        rendered.push_str(&section(skill, heading));
        rendered.push_str("\n\n");
    }
    let path = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("tests/skill-tiers.snap");
    if std::env::var_os("UPDATE_SNAPSHOTS").is_some() {
        fs::write(&path, &rendered).unwrap();
    }
    let snapshot =
        fs::read_to_string(&path).expect("tests/skill-tiers.snap (UPDATE_SNAPSHOTS=1 writes it)");
    assert_eq!(
        rendered, snapshot,
        "a tier section changed: UPDATE_SNAPSHOTS=1 rewrites the snapshot"
    );
}
