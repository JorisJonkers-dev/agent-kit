//! Literal-secret detection for registry and overlay values: a secret-named
//! field must hold an environment reference or a keychain-proxy name, and no
//! value may contain one of the credential shapes below.

/// Why `value` at `path` may not be declared, or `None` when it may. The
/// refusal never prints the value.
pub fn secret_problem(path: &str, value: &str) -> Option<String> {
    if secret_named(path) {
        let plain = matches!(value, "true" | "false");
        if plain || env_ref(value) || keychain_name(value) {
            return None;
        }
        return Some(format!(
            "{path} is secret-named and must hold an environment reference ($NAME) or a keychain name, not a literal"
        ));
    }
    if env_ref(value) {
        return None;
    }
    secret_shape(value).map(|shape| format!("{path} holds a literal secret ({shape})"))
}

/// An environment-name body for `$NAME`, `${NAME}` and `adapter.env.<NAME>`:
/// `[A-Z_][A-Z0-9_]*`.
pub(crate) fn env_name(name: &str) -> bool {
    let mut bytes = name.bytes();
    match bytes.next() {
        Some(b) if b.is_ascii_uppercase() || b == b'_' => {}
        _ => return false,
    }
    bytes.all(|b| b.is_ascii_uppercase() || b.is_ascii_digit() || b == b'_')
}

/// True when `body` is an environment-variable reference: `$NAME` or
/// `${NAME}` with a name of `[A-Z_][A-Z0-9_]*`. Such a value exposes no
/// literal secret, so it is accepted wherever one appears.
fn env_ref(body: &str) -> bool {
    if let Some(rest) = body.strip_prefix("${") {
        match rest.strip_suffix('}') {
            Some(name) => env_name(name),
            None => false,
        }
    } else if let Some(name) = body.strip_prefix('$') {
        env_name(name)
    } else {
        false
    }
}

/// True when `body` is a keychain-proxy field name: one leading letter then
/// letters, digits, `_`, `.` or `-`, 64 characters at most — and no secret
/// shape.
fn keychain_name(body: &str) -> bool {
    let length = body.chars().count();
    if length == 0 || length > 64 {
        return false;
    }
    let mut chars = body.chars();
    chars.next().is_some_and(|c| c.is_ascii_alphabetic())
        && chars.all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && secret_shape(body).is_none()
}

/// True when the field path's last segment, lowercased, names a secret.
fn secret_named(path: &str) -> bool {
    const WORDS: [&str; 5] = ["token", "secret", "key", "password", "authorization"];
    let last = path.rsplit('.').next().unwrap_or("").to_ascii_lowercase();
    WORDS.iter().any(|word| last.contains(word))
}

/// One literal-secret shape: the name reported in the refusal line and the
/// matcher that recognises the shape.
pub struct SecretShape {
    pub name: &'static str,
    pub matches: fn(&str) -> bool,
}

impl SecretShape {
    fn recognised(&self, value: &str) -> bool {
        (self.matches)(value)
    }
}

/// The secret shapes the registry refuses, whatever field they appear in:
/// each one is a literal credential, so a match is a refusal, named by shape.
pub const SECRET_SHAPES: [SecretShape; 7] = [
    SecretShape {
        name: "openai key",
        matches: openai_key,
    },
    SecretShape {
        name: "github token",
        matches: github_token,
    },
    SecretShape {
        name: "slack token",
        matches: slack_token,
    },
    SecretShape {
        name: "aws access key",
        matches: aws_access_key,
    },
    SecretShape {
        name: "bearer token",
        matches: bearer_token,
    },
    SecretShape {
        name: "jwt",
        matches: jwt,
    },
    SecretShape {
        name: "long opaque token",
        matches: opaque_token,
    },
];

/// The shape a value literally contains, if any.
fn secret_shape(value: &str) -> Option<&'static str> {
    SECRET_SHAPES
        .iter()
        .find(|shape| shape.recognised(value))
        .map(|shape| shape.name)
}

/// The first place `needle` occurs in `haystack`, if it occurs at all.
fn find(haystack: &[u8], needle: &[u8]) -> Option<usize> {
    haystack
        .windows(needle.len())
        .position(|window| window == needle)
}

/// True when somewhere after `prefix` there are `least` bytes of `fits`.
fn after_prefix(value: &str, prefix: &str, fits: impl Fn(u8) -> bool, least: usize) -> bool {
    let bytes = value.as_bytes();
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], prefix.as_bytes()) {
        let start = from + at + prefix.len();
        let mut seen = 0;
        while start + seen < bytes.len() && fits(bytes[start + seen]) {
            seen += 1;
        }
        if seen >= least {
            return true;
        }
        from = from + at + 1;
    }
    false
}

/// `sk-` followed by 16 or more word characters.
fn openai_key(value: &str) -> bool {
    after_prefix(value, "sk-", openai_word, 16)
}

fn openai_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_' || b == b'-'
}

/// `ghp_`, `gho_`, `ghs_`, `ghu_` or `github_pat_` followed by 20 or more
/// alphanumerics or underscores.
fn github_token(value: &str) -> bool {
    const PREFIXES: [&str; 5] = ["ghp_", "gho_", "ghs_", "ghu_", "github_pat_"];
    PREFIXES
        .iter()
        .any(|prefix| after_prefix(value, prefix, github_word, 20))
}

fn github_word(b: u8) -> bool {
    b.is_ascii_alphanumeric() || b == b'_'
}

/// A Slack token: `xoxb-` or `xoxp-` anywhere in the value.
fn slack_token(value: &str) -> bool {
    value.contains("xoxb-") || value.contains("xoxp-")
}

/// An AWS access key: `AKIA` followed by 16 uppercase letters or digits.
fn aws_access_key(value: &str) -> bool {
    after_prefix(value, "AKIA", aws_key, 16)
}

fn aws_key(b: u8) -> bool {
    b.is_ascii_uppercase() || b.is_ascii_digit()
}

/// `Bearer ` followed by 16 or more non-whitespace characters,
/// case-insensitively.
fn bearer_token(value: &str) -> bool {
    let needle = b"bearer ";
    let bytes = value.as_bytes();
    if bytes.len() < needle.len() {
        return false;
    }
    let mut at = 0;
    while at + needle.len() <= bytes.len() {
        if bytes[at..at + needle.len()].eq_ignore_ascii_case(needle) {
            let start = at + needle.len();
            let mut seen = 0;
            while start + seen < bytes.len() && !bytes[start + seen].is_ascii_whitespace() {
                seen += 1;
            }
            if seen >= 16 {
                return true;
            }
        }
        at += 1;
    }
    false
}

/// A JWT: three base64url segments between dots, the first starting `eyJ`.
fn jwt(value: &str) -> bool {
    let bytes = value.as_bytes();
    let b64 = |b: u8| b.is_ascii_alphanumeric() || b == b'_' || b == b'-';
    let end = |from: usize| {
        let mut at = from;
        while at < bytes.len() && b64(bytes[at]) {
            at += 1;
        }
        at
    };
    let mut from = 0;
    while let Some(at) = find(&bytes[from..], b"eyJ") {
        let head = from + at;
        let head_end = end(head);
        let dot1 = head_end < bytes.len() && bytes[head_end] == b'.';
        if dot1 {
            let middle_start = head_end + 1;
            let middle_end = end(middle_start);
            let dot2 =
                middle_end > middle_start && middle_end < bytes.len() && bytes[middle_end] == b'.';
            if dot2 {
                let tail_start = middle_end + 1;
                if end(tail_start) > tail_start {
                    return true;
                }
            }
        }
        from = head + 1;
    }
    false
}

/// Any run of 32 or more token characters with both a letter and a digit —
/// except a `/`-separated path whose every segment is shorter than 32.
fn opaque_token(value: &str) -> bool {
    let is_path = value.starts_with('/') || value.starts_with("~/") || value.starts_with("./");
    if is_path
        || (value.contains('/') && value.split('/').all(|segment| segment.chars().count() < 32))
    {
        return false;
    }
    let mut run = 0;
    let mut digit = false;
    let mut letter = false;
    for c in value.chars() {
        if c.is_ascii_alphanumeric() || matches!(c, '+' | '/' | '=' | '_' | '-') {
            run += 1;
            digit |= c.is_ascii_digit();
            letter |= c.is_ascii_alphabetic();
            if run >= 32 && digit && letter {
                return true;
            }
        } else {
            run = 0;
            digit = false;
            letter = false;
        }
    }
    false
}


#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn env_reference_and_keychain_name_pass_in_secret_named_fields() {
        assert_eq!(secret_problem("env.API_TOKEN", "$API_TOKEN"), None);
        assert_eq!(secret_problem("env.API_TOKEN", "${API_TOKEN}"), None);
        assert_eq!(secret_problem("url.key", "work-api.key"), None);
        assert_eq!(secret_problem("settings.keyboard", "true"), None);
    }

    #[test]
    fn literal_in_secret_named_field_is_refused_without_printing_it() {
        for bad in ["hunter2 hunter2", "", "$lower", "${A", "1abc", &"a".repeat(65)] {
            let problem = secret_problem("env.API_TOKEN", bad).expect("refused");
            assert!(problem.contains("secret-named"));
            if !bad.is_empty() {
                assert!(!problem.contains(bad));
            }
        }
        assert!(secret_problem("env.API_KEY", "sk-aaaaaaaaaaaaaaaaaaaa").is_some());
    }

    #[test]
    fn every_secret_shape_is_named() {
        let cases = [
            ("sk-aaaaaaaaaaaaaaaaaaaa", "openai key"),
            ("ghp_aaaaaaaaaaaaaaaaaaaaaaaa", "github token"),
            ("github_pat_aaaaaaaaaaaaaaaaaaaaaaaa", "github token"),
            ("xoxb-123", "slack token"),
            ("xoxp-123", "slack token"),
            ("AKIAAAAAAAAAAAAAAAAA", "aws access key"),
            ("Bearer abcdefghijklmnopqrstuv", "bearer token"),
            ("eyJhbGciOi.eyJzdWIi.c2ln", "jwt"),
            ("abcdefghij0123456789abcdefghij0123", "long opaque token"),
        ];
        for (value, shape) in cases {
            let problem = secret_problem("settings.note", value).expect(value);
            assert_eq!(problem, format!("settings.note holds a literal secret ({shape})"));
        }
    }

    #[test]
    fn near_misses_and_references_are_allowed() {
        for ok in [
            "sk-short",
            "ghp_short",
            "AKIA123",
            "Bearer short",
            "bearer",
            "eyJabc",
            "eyJabc.def",
            "eyJabc.def.",
            "$HOME",
            "~/.local/bin/some-tool-with-a-long-name-0123456789",
            "/a/b/0123456789abcdefghij0123456789abcdefghij/x",
            "plain words only",
            "abcdefghijklmnopqrstuvwxyzabcdefgh",
            "0123456789012345678901234567890123",
            "abc def 0123456789012345678901234567890123 x",
        ] {
            assert_eq!(secret_problem("settings.note", ok), None, "{ok}");
        }
    }

    #[test]
    fn opaque_runs_break_on_a_separator() {
        assert!(secret_problem("a.b", "0123456789abcdef 0123456789abcdef0123456789abcdef").is_some());
        assert!(secret_problem("a.b", "0123456789abcdef 0123456789abcdef").is_none());
    }
}
