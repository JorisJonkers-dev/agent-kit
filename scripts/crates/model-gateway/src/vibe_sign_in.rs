//! Whether a Vibe home is signed in, checked where Vibe itself keeps its key,
//! without reading the key. Vibe stores `MISTRAL_API_KEY` in the OS keyring
//! under the service `ai.mistral.vibe`, and falls back to a line in the home's
//! `.env` when it has no keyring:
//!
//! - on macOS the keyring is the login Keychain, asked with `security
//!   find-generic-password` without `-w`, so the value is never printed;
//! - on Linux Vibe goes through Python's `keyring`, whose Secret Service
//!   backend stores an item with the attributes `service` and `username`. The
//!   check asks the Secret Service for matching item paths over D-Bus
//!   (`SearchItems`), which returns paths only, never a secret;
//! - on every platform, a non-empty `MISTRAL_API_KEY=` line in `<home>/.env`.
//!
//! A keyring that cannot be asked (no `security`, no `gdbus`, no Secret
//! Service on the bus) counts as holding nothing, so the `.env` decides.
//! Elsewhere (Windows, where Vibe's keyring is the Credential Manager) no
//! keyring is asked yet, so only the `.env` counts there.

use crate::error::GatewayError;
use crate::ports::{Files, Process, ProcessCommand};
use crate::text::{is_js_whitespace, join, js_trim_start};

/// The keyring service and account Vibe stores its Mistral key under.
pub const VIBE_KEYRING_SERVICE: &str = "ai.mistral.vibe";
pub const VIBE_KEYRING_ACCOUNT: &str = "MISTRAL_API_KEY";

/// Seconds the Secret Service may take to answer before the keyring counts as holding nothing.
const SECRET_SERVICE_TIMEOUT_S: u64 = 5;

/// The platform as Node names it (`linux`, `darwin`, `win32`), from the name Rust gives the operating system.
pub fn node_platform_of(os: &str) -> &str {
    match os {
        "macos" => "darwin",
        "windows" => "win32",
        other => other,
    }
}

/// The platform this process runs on, as Node names it.
pub fn node_platform() -> &'static str {
    node_platform_of(std::env::consts::OS)
}

/// Whether the home is signed in; `platform` is a Node platform name, so a test can ask for any.
pub fn vibe_signed_in(process: &dyn Process, files: &dyn Files, home: &str, platform: &str) -> Result<bool, GatewayError> {
    if keyring_holds_key(process, platform) {
        return Ok(true);
    }
    let dotenv = join(&[home, ".env"]);
    if !files.exists(&dotenv).map_err(GatewayError::from_fs)? {
        return Ok(false);
    }
    let text = files.read_text(&dotenv).map_err(GatewayError::from_fs)?;
    Ok(text.split('\n').any(sets_the_key))
}

/// A `.env` line that sets the key to something, quoted or not: `^\s*MISTRAL_API_KEY\s*=\s*['"]?[^'"\s]`.
fn sets_the_key(line: &str) -> bool {
    let Some(rest) = js_trim_start(line).strip_prefix(VIBE_KEYRING_ACCOUNT) else {
        return false;
    };
    let Some(rest) = js_trim_start(rest).strip_prefix('=') else {
        return false;
    };
    let rest = js_trim_start(rest);
    let rest = rest.strip_prefix(['\'', '"']).unwrap_or(rest);
    rest.chars().next().is_some_and(|first| first != '\'' && first != '"' && !is_js_whitespace(first))
}

/// The command that asks this platform's keyring whether Vibe's item exists;
/// `None` on a platform this check does not ask a keyring on. The Keychain may
/// wait for its owner to unlock it, so only the Secret Service lookup is timed.
pub fn keyring_query(platform: &str) -> Option<ProcessCommand> {
    match platform {
        "darwin" => Some(ProcessCommand::new("security", &["find-generic-password", "-s", VIBE_KEYRING_SERVICE, "-a", VIBE_KEYRING_ACCOUNT])),
        "linux" => {
            let attributes = format!("{{'service': '{VIBE_KEYRING_SERVICE}', 'username': '{VIBE_KEYRING_ACCOUNT}'}}");
            let timeout = SECRET_SERVICE_TIMEOUT_S.to_string();
            let mut query = ProcessCommand::new(
                "gdbus",
                &[
                    "call",
                    "--session",
                    "--timeout",
                    &timeout,
                    "--dest",
                    "org.freedesktop.secrets",
                    "--object-path",
                    "/org/freedesktop/secrets",
                    "--method",
                    "org.freedesktop.Secret.Service.SearchItems",
                    &attributes,
                ],
            );
            query.timeout_ms = Some((SECRET_SERVICE_TIMEOUT_S + 1) * 1000);
            Some(query)
        }
        _ => None,
    }
}

fn keyring_holds_key(process: &dyn Process, platform: &str) -> bool {
    let Some(query) = keyring_query(platform) else {
        return false;
    };
    let Ok(result) = process.run(&query) else {
        return false;
    };
    if result.exit_code != 0 {
        return false;
    }
    // `security` exits 0 only when the item exists. `SearchItems` answers two
    // lists of item paths, unlocked and locked, such as
    // `([objectpath '/org/freedesktop/secrets/collection/login/3'], @ao [])`;
    // an item in either list exists.
    platform == "darwin" || names_an_item_path(&result.stdout)
}

/// Whether the text holds a quoted absolute path: `'\/[^']+'`.
fn names_an_item_path(text: &str) -> bool {
    text.match_indices("'/").any(|(at, _)| text[at + 2..].find('\'').is_some_and(|end| end > 0))
}
