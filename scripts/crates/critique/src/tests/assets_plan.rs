//! The shipped files read from the assets folder, and a client's plan limit through the gateway's code.

use super::cli::runtime;
use crate::assets::read_asset;
use crate::plan::plan_limit;

fn temp(name: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("critique-assets-{}-{name}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    assert!(std::fs::create_dir_all(&dir).is_ok());
    dir
}

#[test]
fn an_asset_is_read_from_the_assets_folder_and_a_missing_one_is_unexpected() {
    let dir = temp("asset");
    assert!(std::fs::create_dir_all(dir.join("roles")).is_ok());
    assert!(std::fs::write(dir.join("roles/cold-reader.md"), "# Cold reader\n").is_ok());
    let mut runtime = runtime("");
    runtime.assets_root = dir.to_string_lossy().into_owned();
    assert_eq!(read_asset(&runtime, "roles/cold-reader.md"), Ok("# Cold reader\n".to_string()));
    let error = read_asset(&runtime, "roles/none.md").err().map(|error| error.code);
    assert_eq!(error.as_deref(), Some("unexpected"));
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn a_plan_limit_is_read_for_a_client_and_its_failure_is_unexpected() {
    let dir = temp("plan");
    let mut runtime = runtime("");
    runtime.gateway.env.insert("HOME".to_string(), dir.to_string_lossy().into_owned());
    let reading = plan_limit(&runtime, "vibe").map_err(|error| error.message);
    assert_eq!(reading.map(|reading| reading.client), Ok("vibe".to_string()));
    runtime.gateway.env.clear();
    assert_eq!(plan_limit(&runtime, "vibe").err().map(|error| error.code), Some("unexpected".to_string()));
    let _ = std::fs::remove_dir_all(&dir);
}
