use super::*;

fn overlay(name: &str, body: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("agent-launcher-er-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    let file = dir.join("overlay.yaml");
    fs::write(&file, body).unwrap();
    file
}

fn roots(file: &Path) -> Result<Vec<PathBuf>, String> {
    employer_roots(Some(file), Some(Path::new("/home/me")))
}

#[test]
fn no_overlay_or_no_work_instance_means_no_roots() {
    assert_eq!(employer_roots(None, None), Ok(vec![]));
    let missing = std::env::temp_dir().join("agent-launcher-er-missing/overlay.yaml");
    assert_eq!(roots(&missing), Ok(vec![]));
    for body in ["", "instances: {}\n", "instances:\n  gold-band-personal:\n    agents: [vibe]\n"] {
        assert_eq!(roots(&overlay("none", body)), Ok(vec![]), "{body:?}");
    }
}

#[test]
fn the_work_instance_declares_its_roots() {
    let file = overlay(
        "ok",
        "instances:\n  gold-band:\n    employer_roots: [~/Projects/nedap, /srv/employer]\n",
    );
    assert_eq!(
        roots(&file).unwrap(),
        [PathBuf::from("/home/me/Projects/nedap"), PathBuf::from("/srv/employer")]
    );
}

#[test]
fn a_work_instance_without_roots_is_refused() {
    for body in [
        "instances:\n  gold-band: {}\n",
        "instances:\n  gold-band:\n    employer_roots: []\n",
        "instances:\n  gold-band:\n    employer_roots: nedap\n",
    ] {
        let message = roots(&overlay("empty", body)).unwrap_err();
        assert!(message.contains("declares no employer_roots"), "{message}");
    }
}

#[test]
fn an_overlay_that_cannot_be_read_is_refused_not_read_as_empty() {
    let message = roots(&overlay("bad", "instances: [")).unwrap_err();
    assert!(message.contains("cannot parse the overlay"), "{message}");
    let dir = overlay("dir", "").parent().unwrap().to_path_buf();
    let message = roots(&dir).unwrap_err();
    assert!(message.contains("cannot read the overlay"), "{message}");
}

#[test]
fn a_root_must_be_a_plain_absolute_path() {
    for bad in ["nedap", "~", "/", "/a/../b", "", "~/x/..", "7"] {
        let file = overlay("path", &format!("instances:\n  gold-band:\n    employer_roots: ['{bad}']\n"));
        let message = roots(&file).unwrap_err();
        assert!(message.contains("must be an absolute path"), "{bad}: {message}");
    }
    let file = overlay("home", "instances:\n  gold-band:\n    employer_roots: [~/x]\n");
    let message = employer_roots(Some(&file), None).unwrap_err();
    assert!(message.contains("needs HOME"), "{message}");
}

#[test]
fn the_overlay_lives_beside_the_keep_file() {
    assert_eq!(
        overlay_path(Path::new("/home/me")),
        PathBuf::from("/home/me/.config/agent-kit/overlay.yaml")
    );
}
