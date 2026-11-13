//! The pieces the swap stands on: where the shipped assets are, and which commands go to the node runner.

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
fn only_a_command_not_ported_yet_goes_to_the_node_runner_beside_the_assets() {
    let dir = std::env::temp_dir().join(format!("critique-swap-{}", std::process::id()));
    std::fs::create_dir_all(&dir).expect("dir");
    let mut runtime = crate::runtime::default_runtime();
    runtime.assets_root = dir.to_string_lossy().into_owned();
    let argv = |words: &[&str]| words.iter().map(|word| word.to_string()).collect::<Vec<String>>();
    let unported = crate::cli::COMMANDS.iter().find(|(_, port)| port.is_none()).map(|(name, _)| *name);
    if let Some(name) = unported {
        assert_eq!(crate::node_handoff(&argv(&[name, "--json", "{}"]), &runtime), None, "no bundle, no handoff");
        std::fs::write(dir.join("critique.mjs"), "// bundle\n").expect("bundle");
        let bundle = format!("{}/critique.mjs", runtime.assets_root);
        assert_eq!(crate::node_handoff(&argv(&[name, "--json", "{}"]), &runtime), Some(vec![bundle, name.to_string(), "--json".to_string(), "{}".to_string()]));
    }
    std::fs::write(dir.join("critique.mjs"), "// bundle\n").expect("bundle");
    assert_eq!(crate::node_handoff(&argv(&["stats", "--json", "{}"]), &runtime), None);
    assert_eq!(crate::node_handoff(&argv(&["hook", "critique-commit-gate"]), &runtime), None);
    assert_eq!(crate::node_handoff(&argv(&["bogus"]), &runtime), None);
    assert_eq!(crate::node_handoff(&[], &runtime), None);
    let _ = std::fs::remove_dir_all(&dir);
}
