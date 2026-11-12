//! Build files: the package names one build file provides and requires.
//!
//! Line-based and format-agnostic: no parser crate and no index. The reader
//! answers one question per file — which package names does this build file
//! declare itself as, and which does it depend on — so a caller can build a
//! "related repositories" manifest from the build files it finds.
//!
//! Every name is normalised ([`normalise`]) and both lists are de-duplicated in
//! first-seen order, so `my_crate` and `my-crate` are one name.

use code_nav_eval::{parse_json, Json};

/// What one build file declares.
#[derive(Debug, Default, Clone, PartialEq)]
pub struct BuildInfo {
    /// The package names the file declares itself as.
    pub provides: Vec<String>,
    /// The package names the file depends on.
    pub requires: Vec<String>,
}

/// The file names [`is_build_file`] accepts.
const BUILD_FILES: [&str; 8] = [
    "Cargo.toml",
    "package.json",
    "pyproject.toml",
    "go.mod",
    "settings.gradle",
    "settings.gradle.kts",
    "build.gradle",
    "build.gradle.kts",
];

/// The Gradle words that make a line declare a dependency.
const GRADLE_KEYWORDS: [&str; 7] = ["implementation", "api", "compileOnly", "runtimeOnly", "testImplementation", "kapt", "classpath"];

/// The npm tables that hold dependencies.
const NPM_TABLES: [&str; 4] = ["dependencies", "devDependencies", "peerDependencies", "optionalDependencies"];

/// True for a file name this module reads: Cargo.toml, package.json,
/// pyproject.toml, go.mod, settings.gradle, settings.gradle.kts, build.gradle,
/// build.gradle.kts.
pub fn is_build_file(file_name: &str) -> bool {
    BUILD_FILES.contains(&file_name)
}

/// Read one build file's text. `file_name` selects the format; an unknown name
/// gives an empty [`BuildInfo`].
pub fn parse(file_name: &str, text: &str) -> BuildInfo {
    match normalise(file_name).as_str() {
        "cargo.toml" => cargo(text),
        "package.json" => npm(text),
        "pyproject.toml" => python(text),
        "go.mod" => go(text),
        "settings.gradle" | "settings.gradle.kts" => gradle_settings(text),
        "build.gradle" | "build.gradle.kts" => gradle_build(text),
        _ => BuildInfo::default(),
    }
}

/// Lowercase, `_` -> `-`, so `my_crate` and `my-crate` are one name.
pub fn normalise(name: &str) -> String {
    name.trim().to_lowercase().replace('_', "-")
}

/// `names` normalised, empty ones dropped, first-seen order kept.
fn clean(names: Vec<String>) -> Vec<String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let name = normalise(&name);
        if !name.is_empty() && !out.contains(&name) {
            out.push(name);
        }
    }
    out
}

/// The text inside the brackets of a table header line, if `line` is one.
fn header_of(line: &str) -> Option<&str> {
    let line = line.split('#').next().unwrap_or(line).trim_end();
    Some(line.strip_prefix('[')?.strip_suffix(']')?.trim())
}

/// The text inside the quotes of `value` when it is one quoted string.
fn quoted(value: &str) -> Option<String> {
    let quote = value.chars().next()?;
    if quote != '"' && quote != '\'' {
        return None;
    }
    let body = &value[1..];
    let end = body.find(quote)?;
    Some(body[..end].to_string())
}

/// The quoted string after `key =` in `text`, if any.
fn quoted_after(text: &str, key: &str) -> Option<String> {
    let at = text.find(key)?;
    let rest = text[at + key.len()..].trim_start();
    quoted(rest.strip_prefix('=')?.trim_start())
}

/// True when `line` holds a `]` outside its quoted strings, which ends an array.
fn closes_array(line: &str) -> bool {
    let mut quote = None;
    for c in line.chars() {
        match (quote, c) {
            (None, '"' | '\'') => quote = Some(c),
            (Some(q), _) if q == c => quote = None,
            (None, ']') => return true,
            _ => {}
        }
    }
    false
}

/// True for a Cargo table header that lists requirements of the package itself;
/// `[workspace.dependencies]` is a catalogue the members draw from, not a requirement.
fn is_dependency_header(header: &str) -> bool {
    header.contains("dependencies") && !header.starts_with("workspace")
}

/// Every quoted string in `line`, either quote style.
fn quoted_strings(line: &str) -> Vec<String> {
    let mut out = Vec::new();
    let mut rest = line;
    while let Some(open) = rest.find(['"', '\'']) {
        let quote = rest.as_bytes()[open] as char;
        let body = &rest[open + 1..];
        let Some(end) = body.find(quote) else { break };
        out.push(body[..end].to_string());
        rest = &body[end + 1..];
    }
    out
}

/// `key` without surrounding quotes.
fn unquote(key: &str) -> String {
    quoted(key).unwrap_or_else(|| key.to_string())
}

/// The last `/`-separated segment of a module path.
fn last_segment(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// `name` without a leading `@scope/`.
fn strip_scope(name: &str) -> &str {
    match name.strip_prefix('@') {
        Some(rest) => rest.split_once('/').map_or(name, |(_, tail)| tail),
        None => name,
    }
}

/// The characters that end the package name of a requirement string.
const PYTHON_DELIMITERS: [char; 11] = ['<', '>', '=', '!', '~', ';', '[', ' ', '(', '@', ','];

/// The leading package name of a requirement string: up to the first of
/// `<>=!~;[ (@,`.
fn leading_name(spec: &str) -> &str {
    let end = spec.find(&PYTHON_DELIMITERS[..]).unwrap_or(spec.len());
    spec[..end].trim()
}

/// Cargo.toml: `[package] name`, and the keys of every `*dependencies*` table.
fn cargo(text: &str) -> BuildInfo {
    let mut provides = Vec::new();
    let mut requires: Vec<String> = Vec::new();
    let mut header = String::new();
    // The index in `requires` of a `[dependencies.foo]` header's entry.
    let mut sub: Option<usize> = None;
    // Braces still open from an inline table that runs over several lines.
    let mut open: i32 = 0;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if open > 0 {
            open += brace_balance(line);
            continue;
        }
        if let Some(inner) = header_of(line) {
            header = inner.to_lowercase();
            sub = None;
            let last = header.rsplit('.').next().unwrap_or("");
            if is_dependency_header(&header) && !last.contains("dependencies") {
                requires.push(last.trim_matches(|c| c == '\'' || c == '"').to_string());
                sub = Some(requires.len() - 1);
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = unquote(key.trim());
        let value = value.trim();
        if header == "package" {
            if key == "name" {
                if let Some(name) = quoted(value) {
                    provides.push(name);
                }
            }
            continue;
        }
        if !is_dependency_header(&header) {
            continue;
        }
        match sub {
            Some(at) => {
                if key == "package" {
                    if let Some(real) = quoted(value) {
                        requires[at] = real;
                    }
                }
            }
            None => {
                // `serde.workspace = true` names `serde`.
                let key = key.split('.').next().unwrap_or(&key).to_string();
                let name = if value.starts_with('{') { quoted_after(value, "package").unwrap_or(key) } else { key };
                requires.push(name);
                open = brace_balance(value).max(0);
            }
        }
    }
    BuildInfo { provides: clean(provides), requires: clean(requires) }
}

/// Opening braces minus closing ones in `text`.
fn brace_balance(text: &str) -> i32 {
    text.chars().map(|c| match c { '{' => 1, '}' => -1, _ => 0 }).sum()
}

/// package.json: the top-level `name` and the keys of the dependency tables.
fn npm(text: &str) -> BuildInfo {
    let Ok(root) = parse_json(text) else { return BuildInfo::default() };
    let mut provides = Vec::new();
    let mut requires = Vec::new();
    if let Some(name) = root.get("name").and_then(Json::as_str) {
        provides.push(strip_scope(name).to_string());
    }
    for table in NPM_TABLES {
        if let Some(Json::Obj(items)) = root.get(table) {
            for (name, _) in items {
                requires.push(strip_scope(name).to_string());
            }
        }
    }
    BuildInfo { provides: clean(provides), requires: clean(requires) }
}

/// pyproject.toml: the project or poetry name, and the requirement strings.
fn python(text: &str) -> BuildInfo {
    let mut provides = Vec::new();
    let mut requires = Vec::new();
    let mut header = String::new();
    // True while a `dependencies = [` array is still open.
    let mut in_array = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some(inner) = header_of(line) {
            header = inner.to_lowercase();
            in_array = false;
            continue;
        }
        if in_array {
            push_python_requirements(line, &mut requires);
            if closes_array(line) {
                in_array = false;
            }
            continue;
        }
        let Some((key, value)) = line.split_once('=') else { continue };
        let key = unquote(key.trim());
        let value = value.trim();
        if (header == "project" || header == "tool.poetry") && key == "name" {
            if let Some(name) = quoted(value) {
                provides.push(name);
            }
            continue;
        }
        if header == "project" && key == "dependencies" && value.starts_with('[') {
            push_python_requirements(value, &mut requires);
            if !closes_array(value) {
                in_array = true;
            }
            continue;
        }
        if header == "project.optional-dependencies" && value.starts_with('[') {
            push_python_requirements(value, &mut requires);
            if !closes_array(value) {
                in_array = true;
            }
            continue;
        }
        if (header == "tool.poetry.dependencies" || header == "tool.poetry.dev-dependencies") && key != "python" {
            requires.push(key);
        }
    }
    BuildInfo { provides: clean(provides), requires: clean(requires) }
}

/// The leading name of every quoted requirement string in `line`.
fn push_python_requirements(line: &str, out: &mut Vec<String>) {
    for spec in quoted_strings(line) {
        out.push(leading_name(&spec).to_string());
    }
}

/// go.mod: the `module` line and every `require` path.
fn go(text: &str) -> BuildInfo {
    let mut provides = Vec::new();
    let mut requires = Vec::new();
    let mut in_block = false;
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if in_block {
            if line.starts_with(')') {
                in_block = false;
                continue;
            }
            if let Some(path) = line.split_whitespace().next() {
                requires.push(last_segment(path).to_string());
            }
            continue;
        }
        let mut words = line.split_whitespace();
        match words.next() {
            Some("module") => {
                if let Some(path) = words.next() {
                    provides.push(last_segment(path).to_string());
                }
            }
            Some("require") => {
                if let Some(path) = words.next() {
                    if path == "(" {
                        in_block = true;
                    } else {
                        requires.push(last_segment(path).to_string());
                    }
                }
            }
            _ => {}
        }
    }
    BuildInfo { provides: clean(provides), requires: clean(requires) }
}

/// settings.gradle(.kts): the `rootProject.name` string.
fn gradle_settings(text: &str) -> BuildInfo {
    let mut provides = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if let Some(rest) = line.strip_prefix("rootProject.name") {
            let rest = rest.trim_start();
            if let Some(rest) = rest.strip_prefix('=') {
                if let Some(name) = quoted(rest.trim_start()) {
                    provides.push(name);
                }
            }
        }
    }
    BuildInfo { provides: clean(provides), requires: Vec::new() }
}

/// build.gradle(.kts): the artifact of every dependency coordinate.
fn gradle_build(text: &str) -> BuildInfo {
    let mut requires = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with("//") {
            continue;
        }
        if !GRADLE_KEYWORDS.iter().any(|k| line.contains(*k)) {
            continue;
        }
        for spec in quoted_strings(line) {
            if let Some(name) = gradle_artifact(&spec) {
                requires.push(name.to_string());
            }
        }
    }
    BuildInfo { provides: Vec::new(), requires: clean(requires) }
}

/// The artifact of a Gradle coordinate, or the name of a `project(":name")`.
fn gradle_artifact(spec: &str) -> Option<&str> {
    if let Some(rest) = spec.strip_prefix(':') {
        return rest.rsplit(':').next().filter(|s| !s.is_empty());
    }
    let mut parts = spec.split(':');
    parts.next()?;
    parts.next().filter(|s| !s.is_empty())
}

#[cfg(test)]
mod tests {
    use super::*;

    /// A [`BuildInfo`] from plain names, for the assertions below.
    fn info(provides: &[&str], requires: &[&str]) -> BuildInfo {
        BuildInfo {
            provides: provides.iter().map(|s| s.to_string()).collect(),
            requires: requires.iter().map(|s| s.to_string()).collect(),
        }
    }

    #[test]
    fn normalise_lowercases_and_joins_underscores() {
        assert_eq!(normalise("My_Crate"), "my-crate");
        assert_eq!(normalise("  serde_json  "), "serde-json");
        assert_eq!(normalise("already-dashed"), "already-dashed");
    }

    #[test]
    fn is_build_file_accepts_the_eight_names() {
        let names = [
            "Cargo.toml",
            "package.json",
            "pyproject.toml",
            "go.mod",
            "settings.gradle",
            "settings.gradle.kts",
            "build.gradle",
            "build.gradle.kts",
        ];
        for name in names {
            assert!(is_build_file(name), "{name}");
        }
        assert!(!is_build_file("README.md"));
        assert!(!is_build_file("Cargo.lock"));
    }

    #[test]
    fn unknown_file_name_gives_nothing() {
        assert_eq!(parse("README.md", "name = \"x\""), BuildInfo::default());
    }

    #[test]
    fn parse_normalises_the_file_name() {
        assert_eq!(parse("CARGO.TOML", "[package]\nname = \"app\"\n"), info(&["app"], &[]));
    }

    #[test]
    fn cargo_reads_the_package_name_and_dependency_keys() {
        let text = r#"
[package]
name = "code_nav"
version = "0.1.0"
description = "a server"

[features]
default = [
  "std",
]

[dependencies]
code-nav-eval = { path = "../code-nav-eval" }
serde = { version = "1", features = ["derive"] }

[dev-dependencies]
tempfile = "3"

[build-dependencies]
cc = "1"

[workspace.dependencies]
anyhow = "1"

[target.'cfg(unix)'.dependencies]
libc = "0.2"
"#;
        assert_eq!(parse("Cargo.toml", text), info(&["code-nav"], &["code-nav-eval", "serde", "tempfile", "cc", "libc"]));
    }

    #[test]
    fn cargo_reads_a_dependency_sub_table_and_its_package_rename() {
        let text = r#"
[package]
name = "app"

[dependencies.serde]
version = "1"
features = ["derive"]

[dependencies.renamed]
package = "real_crate"
version = "2"
"#;
        assert_eq!(parse("Cargo.toml", text), info(&["app"], &["serde", "real-crate"]));
    }

    #[test]
    fn cargo_uses_the_package_rename_of_an_inline_table() {
        let text = r#"
[package]
name = "app"

[dependencies]
alias = { package = "real-name", version = "1" }
plain = "1"
# a comment
"#;
        assert_eq!(parse("Cargo.toml", text), info(&["app"], &["real-name", "plain"]));
    }

    #[test]
    fn package_json_reads_the_name_and_every_dependency_table() {
        let text = r#"{
  "name": "@acme/web_app",
  "version": "1.0.0",
  "dependencies": { "react": "^18.0.0", "@types/node": "^20.0.0" },
  "devDependencies": { "vitest": "^1.0.0" },
  "peerDependencies": { "react": "^18.0.0", "react-dom": "^18.0.0" },
  "optionalDependencies": { "fsevents": "^2.0.0" },
  "scripts": { "build": "vite build" }
}"#;
        assert_eq!(parse("package.json", text), info(&["web-app"], &["react", "node", "vitest", "react-dom", "fsevents"]));
    }

    #[test]
    fn package_json_with_invalid_json_gives_nothing() {
        assert_eq!(parse("package.json", "{ \"name\": "), BuildInfo::default());
    }

    #[test]
    fn package_json_without_a_name_or_dependency_tables() {
        assert_eq!(parse("package.json", "{\"private\": true}"), BuildInfo::default());
    }

    #[test]
    fn package_json_ignores_a_dependency_table_that_is_not_an_object() {
        assert_eq!(parse("package.json", "{\"dependencies\": [\"react\"]}"), BuildInfo::default());
    }

    #[test]
    fn pyproject_reads_the_project_name_and_its_dependencies() {
        let text = r#"
# a comment
[build-system]
requires = ["setuptools>=68"]

[project]
name = "my_pkg"
version = "0.1.0"
dependencies = [
  "requests>=2.31",
  "click",
  "pydantic[email]>=2",
]

[project.optional-dependencies]
dev = [
  "pytest>=7",
  "ruff",
]
"#;
        assert_eq!(parse("pyproject.toml", text), info(&["my-pkg"], &["requests", "click", "pydantic", "pytest", "ruff"]));
    }

    #[test]
    fn pyproject_reads_a_single_line_dependency_array() {
        let text = "[project]\nname = \"app\"\ndependencies = [\"flask>=3\", \"gunicorn\"]\n";
        assert_eq!(parse("pyproject.toml", text), info(&["app"], &["flask", "gunicorn"]));
    }

    #[test]
    fn pyproject_drops_an_empty_requirement() {
        let text = "[project]\nname = \"app\"\ndependencies = [\"\", \"flask\"]\n";
        assert_eq!(parse("pyproject.toml", text), info(&["app"], &["flask"]));
    }

    #[test]
    fn pyproject_reads_the_poetry_tables() {
        let text = r#"
[tool.poetry]
name = "poetry_app"
version = "1.0.0"

[tool.poetry.dependencies]
python = "^3.11"
requests = "^2.31"
numpy = { version = "^1.26", optional = true }

[tool.poetry.dev-dependencies]
pytest = "^7.0"
"#;
        assert_eq!(parse("pyproject.toml", text), info(&["poetry-app"], &["requests", "numpy", "pytest"]));
    }

    #[test]
    fn go_mod_reads_the_module_and_single_line_requires() {
        let text = r#"
// a comment
module github.com/acme/build_tools

go 1.22

require github.com/stretchr/testify v1.9.0
require golang.org/x/sync v0.7.0 // indirect
"#;
        assert_eq!(parse("go.mod", text), info(&["build-tools"], &["testify", "sync"]));
    }

    #[test]
    fn go_mod_reads_a_require_block() {
        let text = r#"
module example.com/estate/tools

require (
    github.com/spf13/cobra v1.8.0
    gopkg.in/yaml.v3 v3.0.1
)
"#;
        assert_eq!(parse("go.mod", text), info(&["tools"], &["cobra", "yaml.v3"]));
    }

    #[test]
    fn settings_gradle_reads_the_root_project_name() {
        let text = "// the root\nrootProject.name = \"estate\"\ninclude(\":app\")\n";
        assert_eq!(parse("settings.gradle", text), info(&["estate"], &[]));
        assert_eq!(parse("settings.gradle.kts", "rootProject.name='estate_kotlin'\n"), info(&["estate-kotlin"], &[]));
    }

    #[test]
    fn build_gradle_reads_the_artifact_of_each_coordinate() {
        let text = r#"
plugins {
    id 'java'
}

dependencies {
    implementation "org.springframework:spring-core:6.1.0"
    api 'com.google.guava:guava'
    compileOnly "org.projectlombok:lombok:1.18.30"
    runtimeOnly "org.postgresql:postgresql:42.7.1"
    testImplementation "org.junit.jupiter:junit-jupiter:5.10.0"
    kapt "com.google.dagger:dagger-compiler:2.50"
    implementation project(":core:data")
    // a comment
}

buildscript {
    dependencies {
        classpath "com.android.tools.build:gradle:8.1.0"
    }
}
"#;
        assert_eq!(
            parse("build.gradle", text),
            info(
                &[],
                &[
                    "spring-core",
                    "guava",
                    "lombok",
                    "postgresql",
                    "junit-jupiter",
                    "dagger-compiler",
                    "data",
                    "gradle",
                ],
            )
        );
    }

    #[test]
    fn build_gradle_kts_reads_the_same_coordinates() {
        let text = r#"
dependencies {
    implementation("io.ktor:ktor-server-core:2.3.0")
    implementation(project(":shared"))
    implementation(files("libs/local.jar"))
}
"#;
        assert_eq!(parse("build.gradle.kts", text), info(&[], &["ktor-server-core", "shared"]));
    }

    #[test]
    fn half_written_lines_declare_nothing() {
        assert_eq!(parse("go.mod", "module\nrequire\n"), info(&[], &[]));
        assert_eq!(parse("settings.gradle", "rootProject.name\nrootProject.name = nameless\n"), info(&[], &[]));
    }

    #[test]
    fn cargo_reads_inheritance_multi_line_tables_and_skips_the_workspace_catalogue() {
        let text = r#"
[workspace.dependencies]
anyhow = "1"
[workspace.dependencies.extra]
version = "1"

[dependencies] # the real ones
serde.workspace = true
tokio = { version = "1",
  features = ["full"],
  optional = true }
alias = { features = ["x"], package = "real" }
"#;
        assert_eq!(parse("Cargo.toml", text), info(&[], &["serde", "tokio", "real"]));
    }

    #[test]
    fn a_bracketed_extra_does_not_end_a_python_array() {
        let text = "[project]\ndependencies = [\n  \"pydantic[email]>=2\",\n  \"click\",\n]\n";
        assert_eq!(parse("pyproject.toml", text), info(&[], &["pydantic", "click"]));
        let single = "[project]\ndependencies = [\"a[x]\",\n \"b\"]\n";
        assert_eq!(parse("pyproject.toml", single), info(&[], &["a", "b"]));
    }
}
