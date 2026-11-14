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

#[test]
fn the_search_server_is_the_runner_itself_unless_switched_off() {
    use crate::runtime::search_command;
    let exe = Some(PathBuf::from("/opt/kit/bin/critique"));
    let mut env = HashMap::new();
    assert_eq!(search_command(&env, exe.clone()).map(|own| own.command), Some("/opt/kit/bin/critique".to_string()));
    assert!(search_command(&env, None).is_none());
    env.insert("CRITIQUE_SEARCH_OFF".to_string(), "0".to_string());
    assert!(search_command(&env, exe.clone()).is_some());
    env.insert("CRITIQUE_SEARCH_OFF".to_string(), "1".to_string());
    assert!(search_command(&env, exe).is_none());
}
