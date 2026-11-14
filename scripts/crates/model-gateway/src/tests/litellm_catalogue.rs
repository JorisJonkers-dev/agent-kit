//! What the goldens cannot reach in the LiteLLM catalogue reader: a machine with no home to find the
//! catalogue in, and a file system that cannot even look at the path.

use super::config::{runtime, StubFiles, StubGit};
use crate::litellm_catalogue::{read_catalogue, require_usable, CatalogueReading};
use crate::ports::FsError;
use std::collections::HashMap;

#[test]
fn no_home_and_no_absolute_cache_home_reads_no_catalogue_and_says_nothing() {
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "relative".to_string())]);
    let reading = read_catalogue(&runtime(StubFiles::new(), StubGit::new(None, &[]), env));
    assert_eq!(reading, CatalogueReading::default());
}

#[test]
fn a_path_the_files_cannot_look_at_is_a_warning() {
    let mut files = StubFiles::new();
    files.exists_error = Some(FsError { code: "EACCES".into(), message: "EACCES: permission denied".into() });
    let env = HashMap::from([("XDG_CACHE_HOME".to_string(), "/cache".to_string())]);
    let reading = read_catalogue(&runtime(files, StubGit::new(None, &[]), env));
    assert_eq!(reading.catalogue, None);
    assert_eq!(
        reading.warning.as_deref(),
        Some("the LiteLLM catalogue at /cache/agent-kit/litellm-catalogue.json cannot be read, so the models are listed unchecked")
    );
}

#[test]
fn a_client_with_no_route_is_never_refused_on_the_catalogue() {
    let runtime = runtime(StubFiles::new(), StubGit::new(None, &[]), super::config::home_env("/home/someone"));
    let config = crate::config_parse::load_config(&runtime);
    let config = config.expect("the shipped defaults load");
    assert!(require_usable(&runtime, &config, "claude", "claude-opus-5-5").is_ok());
    assert!(require_usable(&runtime, &config, "nobody", "x").is_ok());
}
