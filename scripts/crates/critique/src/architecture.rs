//! What the architecture-review stage reads of a repository beside its tree and its decision records: the README and
//! the build configuration at the root. The node runner's `domain/architecture.ts`.

use crate::js_regex::Regex;

/// The README the packet holds, the first the repository has.
pub const READMES: [&str; 3] = ["README.md", "readme.md", "README"];

/// Layer-rule configurations: a tool that fails a build when an import points the wrong way.
const LAYER_CONFIGS: [&str; 7] = [
    r"^\.?dependency-cruiser(\.config)?\.(c|m)?js$",
    r"^\.?dependency-cruiser(\.config)?\.json$",
    r"^\.importlinter$",
    r"^tach\.toml$",
    r"^\.?arch-?unit\.(json|ya?ml|properties)$",
    r"^\.go-arch-lint\.ya?ml$",
    r"^deptrac\.ya?ml$",
];

/// Build and project configuration worth a critic's reading, by root file name.
const BUILD_CONFIGS: [&str; 11] = [
    r"^package\.json$",
    r"^tsconfig(\.[\w-]+)?\.json$",
    r"^Cargo\.toml$",
    r"^pyproject\.toml$",
    r"^go\.mod$",
    r"^pom\.xml$",
    r"^build\.gradle(\.kts)?$",
    r"^settings\.gradle(\.kts)?$",
    r"^Makefile$",
    r"^justfile$",
    r"^docker-compose\.ya?ml$",
];

/// How many build configurations a packet holds, and how many bytes of each: a config is evidence, not a second tree.
pub const BUILD_CONFIG_MAX_FILES: usize = 12;
pub const BUILD_CONFIG_MAX_BYTES: usize = 8192;

fn any_matches(patterns: &[&str], name: &str) -> bool {
    let chars: Vec<char> = name.chars().collect();
    patterns.iter().any(|pattern| Regex::new(pattern, false).expect("the pattern compiles").find_at(&chars, 0).is_some())
}

/// Whether a root file is a layer-rule configuration.
pub fn is_layer_config(name: &str) -> bool {
    any_matches(&LAYER_CONFIGS, name)
}

/// The root files to read as the build configuration, layer configs first, then by name, at most `BUILD_CONFIG_MAX_FILES`.
pub fn build_configs(root_files: &[String]) -> Vec<String> {
    let layers = root_files.iter().filter(|name| is_layer_config(name));
    let others = root_files.iter().filter(|name| !is_layer_config(name) && any_matches(&BUILD_CONFIGS, name));
    layers.chain(others).take(BUILD_CONFIG_MAX_FILES).cloned().collect()
}
