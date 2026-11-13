//! What the swap stands on: where the shipped assets are.

use crate::runtime::assets_root;
use std::collections::HashMap;
use std::path::PathBuf;

#[test]
fn the_assets_are_where_the_launcher_says_else_beside_the_binary() {
    let exe = Some(PathBuf::from("/opt/kit/bin/critique"));
    let mut env = HashMap::new();
    assert_eq!(assets_root(&env, exe.clone()), "/opt/kit/bin");
    assert_eq!(assets_root(&env, None), "");
    env.insert("AGENT_KIT_CRITIQUE_ASSETS".to_string(), String::new());
    assert_eq!(assets_root(&env, exe.clone()), "/opt/kit/bin");
    env.insert("AGENT_KIT_CRITIQUE_ASSETS".to_string(), "/h/.claude/skills/critique".to_string());
    assert_eq!(assets_root(&env, exe), "/h/.claude/skills/critique");
}
