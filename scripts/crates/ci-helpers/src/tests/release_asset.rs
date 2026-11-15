use super::support::{run, Tmp};
use crate::release_asset::{
    check_checksum_file, check_version, extract_version, parse_sha256sum_line, sha256_hex,
    stamp_version, RELEASE_TAG_PLACEHOLDER,
};

fn script() -> String {
    format!("#!/usr/bin/env bash\n# GENERATED FROM registry/estate-tooling.yaml -- DO NOT EDIT.\nset -uo pipefail\nAGENT_KIT_RELEASE_TAG='{RELEASE_TAG_PLACEHOLDER}'\n")
}

#[test]
fn stamp_inserts_the_token_after_the_shebang_and_sets_the_release_tag() {
    let stamped = stamp_version(&script(), "v4.1.0").unwrap();
    let lines: Vec<&str> = stamped.lines().collect();
    assert_eq!(lines[0], "#!/usr/bin/env bash");
    assert_eq!(lines[1], "# agent-kit v4.1.0");
    assert_eq!(
        lines[2..lines.len() - 1],
        script().lines().collect::<Vec<_>>()[1..3]
    );
    assert_eq!(lines[lines.len() - 1], "AGENT_KIT_RELEASE_TAG='v4.1.0'");
    assert!(!stamped.contains(RELEASE_TAG_PLACEHOLDER));
    assert_eq!(extract_version(&stamped), Some("v4.1.0"));
    assert_eq!(extract_version(&script()), None);
    assert_eq!(extract_version("one line"), None);
}

#[test]
fn stamp_refuses_what_it_cannot_stamp_safely() {
    assert!(stamp_version("#!/usr/bin/env bash\necho hi\n", "v4.1.0")
        .unwrap_err()
        .contains("placeholder"));
    assert!(stamp_version(&script(), "v4.1.0'; rm -rf /; '")
        .unwrap_err()
        .contains("tag"));
    assert!(stamp_version(&script(), "").unwrap_err().contains("tag"));
    assert!(stamp_version("echo hi\n", "v4.1.0")
        .unwrap_err()
        .contains("shebang"));
    assert!(stamp_version("", "v4.1.0").unwrap_err().contains("shebang"));
}

#[test]
fn check_version_compares_the_token_and_the_release_tag() {
    let stamped = stamp_version(&script(), "v4.1.0").unwrap();
    assert_eq!(check_version(&stamped, "v4.1.0"), Ok(()));
    assert!(check_version(&stamped, "v4.2.0")
        .unwrap_err()
        .contains("version token mismatch"));
    assert!(check_version(&script(), "v4.1.0")
        .unwrap_err()
        .contains("got None"));
    let drifted = stamped.replace("TAG='v4.1.0'", "TAG='v4.0.0'");
    assert!(check_version(&drifted, "v4.1.0")
        .unwrap_err()
        .contains("release tag mismatch"));
}

#[test]
fn sha256_matches_known_digests() {
    assert_eq!(
        sha256_hex(b""),
        "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"
    );
    assert_eq!(
        sha256_hex(b"abc"),
        "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
    );
    let long = vec![b'a'; 1000];
    assert_eq!(
        sha256_hex(&long),
        "41edece42d63e8d9bf515a9ba6932e1c20cbc9f5a5d134645adb5db1b9737ea3"
    );
}

#[test]
fn sha256sum_lines() {
    assert_eq!(
        parse_sha256sum_line("abc123  agent-kit-skills.tar.gz\n"),
        Ok(("abc123".into(), "agent-kit-skills.tar.gz".into()))
    );
    assert_eq!(
        parse_sha256sum_line("abc123  *agent-kit-skills.tar.gz"),
        Ok(("abc123".into(), "agent-kit-skills.tar.gz".into()))
    );
    assert!(parse_sha256sum_line("not-a-checksum-line")
        .unwrap_err()
        .contains("not a sha256sum line"));
}

#[test]
fn checksum_files_must_name_and_match_the_tarball() {
    let tmp = Tmp::new();
    let tarball = tmp.write("agent-kit-skills.tar.gz", "fake tarball contents");
    let digest = sha256_hex(b"fake tarball contents");
    assert_eq!(
        check_checksum_file(&format!("{digest}  agent-kit-skills.tar.gz\n"), &tarball),
        Ok(())
    );
    assert!(
        check_checksum_file(&format!("{digest}  wrong-name.tar.gz\n"), &tarball)
            .unwrap_err()
            .contains("checksum file names")
    );
    let zeros = "0".repeat(64);
    assert!(
        check_checksum_file(&format!("{zeros}  agent-kit-skills.tar.gz\n"), &tarball)
            .unwrap_err()
            .contains("checksum mismatch")
    );
    assert!(check_checksum_file("", &tarball)
        .unwrap_err()
        .contains("not a sha256sum line"));
    let gone = tmp.join("gone.tar.gz");
    assert!(check_checksum_file(&format!("{digest}  gone.tar.gz\n"), &gone).is_err());
}

#[test]
fn the_command_line() {
    let result = run(
        &["verify-release-asset", "stamp-version", "v4.1.0"],
        &script(),
    );
    assert_eq!(result.code, 0);
    assert_eq!(result.out, stamp_version(&script(), "v4.1.0").unwrap());
    assert_eq!(
        run(
            &["verify-release-asset", "check-version", "v4.1.0"],
            &result.out
        )
        .code,
        0
    );
    let wrong = run(
        &["verify-release-asset", "check-version", "v4.2.0"],
        &result.out,
    );
    assert_eq!(wrong.code, 1);
    assert!(wrong.err.contains("version token mismatch"));
    assert_eq!(
        run(
            &["verify-release-asset", "stamp-version", "v1"],
            "echo hi\n"
        )
        .code,
        1
    );

    let tmp = Tmp::new();
    let tarball = tmp.write("t.tar.gz", "bytes");
    let sums = tmp.write("t.sha256", &format!("{}  t.tar.gz\n", sha256_hex(b"bytes")));
    let (tarball, sums) = (tarball.display().to_string(), sums.display().to_string());
    assert_eq!(
        run(
            &["verify-release-asset", "check-checksum", &tarball, &sums],
            ""
        )
        .code,
        0
    );
    let missing = run(
        &[
            "verify-release-asset",
            "check-checksum",
            &tarball,
            &tmp.join("none").display().to_string(),
        ],
        "",
    );
    assert_eq!(missing.code, 1);
    let other = tmp.write("o.tar.gz", "other");
    assert_eq!(
        run(
            &[
                "verify-release-asset",
                "check-checksum",
                &other.display().to_string(),
                &sums
            ],
            ""
        )
        .code,
        1
    );

    assert_eq!(run(&["verify-release-asset"], "").code, 2);
    assert_eq!(run(&["verify-release-asset", "stamp-version"], "").code, 2);
}

#[test]
fn the_dispatcher_refuses_an_unknown_helper() {
    let result = run(&["nope"], "");
    assert_eq!(result.code, 2);
    assert!(result.err.starts_with("usage: ci-helpers"));
    assert_eq!(run(&[], "").code, 2);
}

#[test]
fn every_helper_is_reachable_and_the_real_streams_work() {
    assert_eq!(crate::run(vec![]), 2);
    let tmp = Tmp::new();
    let sources = tmp.join("workstation");
    std::fs::create_dir_all(&sources).unwrap();
    let args = |name: &str, rest: &[&str]| {
        std::iter::once(name)
            .chain(rest.iter().copied())
            .map(String::from)
            .collect::<Vec<_>>()
    };
    assert_eq!(
        crate::run(args(
            "skill-sources",
            &["--validate", "--root", &sources.display().to_string()]
        )),
        0
    );
    assert_eq!(run(&["marketplace-bump"], "").code, 1);
    assert_eq!(run(&["skill-sources", "--bogus"], "").code, 2);
}
