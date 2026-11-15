//! Stamping and checking the release version token, split out of
//! scripts/publish-installer-artifacts.sh so the comparison that decides
//! pass or fail is testable without a network call.
//!
//! `stamp-version TAG` reads a script on stdin and writes it with the token
//! inserted after the shebang and the release tag set. `check-version TAG`
//! reads a script and fails unless its token matches. `check-checksum TARBALL
//! SHA256_FILE` fails unless the checksum file names and matches the tarball.

use crate::Io;
use std::fmt::Write as _;
use std::fs;
use std::io::Read;
use std::path::Path;

pub const VERSION_PREFIX: &str = "# agent-kit ";
/// scripts/render_registry.py renders `AGENT_KIT_RELEASE_TAG='<placeholder>'`;
/// stamping puts the release tag there.
pub const RELEASE_TAG_PLACEHOLDER: &str = "@AGENT_KIT_RELEASE_TAG@";
const RELEASE_TAG_VAR: &str = "AGENT_KIT_RELEASE_TAG";

fn release_tag_line(tag: &str) -> String {
    format!("{RELEASE_TAG_VAR}='{tag}'")
}

/// Insert the version token as line 2, right after the shebang, and set the release tag.
pub fn stamp_version(script: &str, tag: &str) -> Result<String, String> {
    let mut lines = script.split_inclusive('\n');
    let shebang = lines
        .next()
        .filter(|line| line.starts_with("#!"))
        .ok_or("script has no shebang on line 1")?;
    if tag.is_empty()
        || !tag
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "._+-".contains(c))
    {
        return Err(format!("tag '{tag}' is not a plain release tag"));
    }
    let body = lines.collect::<String>();
    let placeholder = release_tag_line(RELEASE_TAG_PLACEHOLDER);
    if body.matches(&placeholder).count() != 1 {
        return Err(format!(
            "script must carry exactly one {placeholder} placeholder line"
        ));
    }
    Ok(format!(
        "{shebang}{VERSION_PREFIX}{tag}\n{}",
        body.replace(&placeholder, &release_tag_line(tag))
    ))
}

/// The stamped version token, or None if line 2 does not carry one.
pub fn extract_version(script: &str) -> Option<&str> {
    script.lines().nth(1)?.strip_prefix(VERSION_PREFIX)
}

pub fn check_version(script: &str, expected: &str) -> Result<(), String> {
    let actual = extract_version(script);
    if actual != Some(expected) {
        let shown = actual.map_or("None".to_string(), |token| format!("'{token}'"));
        return Err(format!(
            "version token mismatch: expected '{expected}', got {shown}"
        ));
    }
    let prefix = format!("{RELEASE_TAG_VAR}=");
    let found: Vec<&str> = script
        .lines()
        .filter(|line| line.starts_with(&prefix))
        .collect();
    let wanted = release_tag_line(expected);
    if found != [wanted.as_str()] {
        return Err(format!(
            "release tag mismatch: expected '{wanted}', got {found:?}"
        ));
    }
    Ok(())
}

const K: [u32; 64] = [
    0x428a2f98, 0x71374491, 0xb5c0fbcf, 0xe9b5dba5, 0x3956c25b, 0x59f111f1, 0x923f82a4, 0xab1c5ed5,
    0xd807aa98, 0x12835b01, 0x243185be, 0x550c7dc3, 0x72be5d74, 0x80deb1fe, 0x9bdc06a7, 0xc19bf174,
    0xe49b69c1, 0xefbe4786, 0x0fc19dc6, 0x240ca1cc, 0x2de92c6f, 0x4a7484aa, 0x5cb0a9dc, 0x76f988da,
    0x983e5152, 0xa831c66d, 0xb00327c8, 0xbf597fc7, 0xc6e00bf3, 0xd5a79147, 0x06ca6351, 0x14292967,
    0x27b70a85, 0x2e1b2138, 0x4d2c6dfc, 0x53380d13, 0x650a7354, 0x766a0abb, 0x81c2c92e, 0x92722c85,
    0xa2bfe8a1, 0xa81a664b, 0xc24b8b70, 0xc76c51a3, 0xd192e819, 0xd6990624, 0xf40e3585, 0x106aa070,
    0x19a4c116, 0x1e376c08, 0x2748774c, 0x34b0bcb5, 0x391c0cb3, 0x4ed8aa4a, 0x5b9cca4f, 0x682e6ff3,
    0x748f82ee, 0x78a5636f, 0x84c87814, 0x8cc70208, 0x90befffa, 0xa4506ceb, 0xbef9a3f7, 0xc67178f2,
];

/// SHA-256 of `data`, as lowercase hex.
pub fn sha256_hex(data: &[u8]) -> String {
    let mut state: [u32; 8] = [
        0x6a09e667, 0xbb67ae85, 0x3c6ef372, 0xa54ff53a, 0x510e527f, 0x9b05688c, 0x1f83d9ab,
        0x5be0cd19,
    ];
    let mut padded = data.to_vec();
    padded.push(0x80);
    while padded.len() % 64 != 56 {
        padded.push(0);
    }
    padded.extend_from_slice(&((data.len() as u64) * 8).to_be_bytes());
    for block in padded.chunks(64) {
        let mut w = [0u32; 64];
        for (i, word) in block.chunks(4).enumerate() {
            w[i] = u32::from_be_bytes([word[0], word[1], word[2], word[3]]);
        }
        for i in 16..64 {
            let s0 = w[i - 15].rotate_right(7) ^ w[i - 15].rotate_right(18) ^ (w[i - 15] >> 3);
            let s1 = w[i - 2].rotate_right(17) ^ w[i - 2].rotate_right(19) ^ (w[i - 2] >> 10);
            w[i] = w[i - 16]
                .wrapping_add(s0)
                .wrapping_add(w[i - 7])
                .wrapping_add(s1);
        }
        let mut v = state;
        for i in 0..64 {
            let s1 = v[4].rotate_right(6) ^ v[4].rotate_right(11) ^ v[4].rotate_right(25);
            let ch = (v[4] & v[5]) ^ (!v[4] & v[6]);
            let t1 = v[7]
                .wrapping_add(s1)
                .wrapping_add(ch)
                .wrapping_add(K[i])
                .wrapping_add(w[i]);
            let s0 = v[0].rotate_right(2) ^ v[0].rotate_right(13) ^ v[0].rotate_right(22);
            let maj = (v[0] & v[1]) ^ (v[0] & v[2]) ^ (v[1] & v[2]);
            let t2 = s0.wrapping_add(maj);
            v = [
                t1.wrapping_add(t2),
                v[0],
                v[1],
                v[2],
                v[3].wrapping_add(t1),
                v[4],
                v[5],
                v[6],
            ];
        }
        for (slot, add) in state.iter_mut().zip(v) {
            *slot = slot.wrapping_add(add);
        }
    }
    state.iter().fold(String::new(), |mut hex, word| {
        let _ = write!(hex, "{word:08x}");
        hex
    })
}

/// Parse one `sha256sum` output line: `<hex>  <name>`.
pub fn parse_sha256sum_line(line: &str) -> Result<(String, String), String> {
    let stripped = line.trim();
    match stripped.split_once(char::is_whitespace) {
        Some((digest, name)) => Ok((
            digest.to_string(),
            name.trim_start().trim_start_matches('*').to_string(),
        )),
        None => Err(format!("not a sha256sum line: {line:?}")),
    }
}

pub fn check_checksum_file(checksum_text: &str, tarball: &Path) -> Result<(), String> {
    let (digest, name) = parse_sha256sum_line(checksum_text.trim().lines().next().unwrap_or(""))?;
    let tarball_name = tarball
        .file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .unwrap_or_default();
    if name != tarball_name {
        return Err(format!(
            "checksum file names '{name}', expected '{tarball_name}'"
        ));
    }
    let actual =
        sha256_hex(&fs::read(tarball).map_err(|error| format!("{}: {error}", tarball.display()))?);
    if actual != digest {
        return Err(format!(
            "checksum mismatch: file claims {digest}, tarball hashes to {actual}"
        ));
    }
    Ok(())
}

fn read_all(input: &mut dyn Read) -> String {
    let mut text = String::new();
    let _ = input.read_to_string(&mut text);
    text
}

pub fn run(args: &[String], io: &mut Io) -> i32 {
    let outcome = match args
        .iter()
        .map(String::as_str)
        .collect::<Vec<_>>()
        .as_slice()
    {
        ["stamp-version", tag] => stamp_version(&read_all(io.stdin), tag).map(|stamped| {
            let _ = io.out.write_all(stamped.as_bytes());
        }),
        ["check-version", tag] => check_version(&read_all(io.stdin), tag),
        ["check-checksum", tarball, sha256_file] => fs::read_to_string(sha256_file)
            .map_err(|error| format!("{sha256_file}: {error}"))
            .and_then(|text| check_checksum_file(&text, Path::new(tarball))),
        _ => {
            io.complain("usage: ci-helpers verify-release-asset stamp-version TAG | check-version TAG | check-checksum TARBALL SHA256_FILE");
            return 2;
        }
    };
    match outcome {
        Ok(()) => 0,
        Err(message) => {
            io.complain(&message);
            1
        }
    }
}
