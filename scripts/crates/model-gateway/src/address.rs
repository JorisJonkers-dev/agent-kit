//! A pinned model address, `<harness>:<pricing>/<model>` (for example `pi:free/glm-4.7-flash`, `pi-free:free/hy4-flash`
//! or `opencode:free/deepseek-v4.1-flash`), names the harness that runs the work and leaves the proxy's address
//! (`free/<model>`) as it is: the proxy still chooses the provider. A run that names its harness this way is
//! pinned (ADR-0018): the gateway never swaps the harness for another, the free fallback included.
//!
//! The address stands where a run's `client` and `model` stand; a run that names its `client` keeps naming it
//! (`client` alone does not pin: only a `<harness>:` address or `client_pinned: true` does). An address whose
//! harness or pricing prefix is unknown is refused with the ones there are. No address names a provider: the
//! harness is Pi (`pi`, which takes the Pi route of the pricing prefix, or `pi-<route>` itself) or OpenCode.

use crate::error::GatewayError;
use crate::run::is_pi_client;

/// The pricing prefixes of the proxy's addresses.
pub const PRICINGS: [&str; 3] = ["free", "paid", "auto"];

/// The harnesses a pinned address may name, as the refusal lists them.
const HARNESSES: &str = "pi, pi-<route> (such as pi-free) and opencode";

/// What an address comes to: the client that runs, and the model it is given.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Addressed {
    pub client: String,
    pub model: String,
}

fn refusal(code: &str, message: String) -> GatewayError {
    GatewayError::new(code, message)
}

/// Whether `model` is the proxy's own address, `<pricing>/<model>`: what a run through the proxy's chains names.
pub fn is_proxy_address(model: &str) -> bool {
    model.split_once('/').is_some_and(|(pricing, name)| PRICINGS.contains(&pricing) && !name.is_empty())
}

/// Whether `model` is written as a pinned address: a harness, a colon, and a name with a `/` after it. A model
/// such as `qwen3:32b` has no `/` and names its own tag.
pub fn is_address(model: &str) -> bool {
    model.split_once(':').is_some_and(|(harness, rest)| !harness.is_empty() && rest.contains('/') && !harness.contains('/'))
}

/// The client and model a pinned address names, or why it is refused.
pub fn resolve(address: &str) -> Result<Addressed, GatewayError> {
    let Some((harness, rest)) = address.split_once(':') else {
        return Err(refusal("invalid-input", format!("{address} is not a pinned address: it is <harness>:<pricing>/<model>")));
    };
    let Some((pricing, name)) = rest.split_once('/') else {
        return Err(refusal("unknown-pricing", format!("{address} names no pricing prefix: it is <harness>:<pricing>/<model>, where the pricing is {}", PRICINGS.join(", "))));
    };
    if !PRICINGS.contains(&pricing) {
        return Err(refusal("unknown-pricing", format!("{address} names the pricing prefix {pricing:?}, which the proxy does not serve: use {}", PRICINGS.join(", "))));
    }
    if name.is_empty() {
        return Err(refusal("invalid-input", format!("{address} names no model after {pricing}/")));
    }
    let proxy = format!("{pricing}/{name}");
    match harness {
        "pi" => Ok(Addressed { client: format!("pi-{pricing}"), model: proxy }),
        "opencode" => Ok(Addressed { client: "opencode".to_string(), model: format!("litellm-{pricing}/{proxy}") }),
        route if is_pi_client(route) => {
            if route.strip_prefix("pi-") == Some(pricing) {
                Ok(Addressed { client: route.to_string(), model: proxy })
            } else {
                Err(refusal("unknown-harness", format!("{address}: {route} does not serve {pricing}/ addresses: pin pi:{pricing}/{name}, or pi-{pricing}:{pricing}/{name}")))
            }
        }
        other => Err(refusal("unknown-harness", format!("{address} names the harness {other:?}, which takes no proxy address: a pinned address names {HARNESSES}"))),
    }
}
