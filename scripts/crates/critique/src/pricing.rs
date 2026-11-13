//! What a client's model costs, in USD per million tokens.

use crate::error::ShapeError;
use crate::fields::Fields;

/// What a client's model costs, in USD per million tokens. Cached input is prompt the provider served from its
/// cache: Mistral bills it at 10% of the input price, and a critic re-reads its whole conversation every turn, so
/// most of its input is cached.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenPrices {
    pub input: f64,
    pub cached_input: f64,
    pub output: f64,
}

/// What one run used, in tokens; cached input is prompt the provider served from its cache.
#[derive(Clone, Debug, PartialEq)]
pub struct TokenUsage {
    pub cached_input: f64,
    pub input: f64,
    pub output: f64,
}

/// What a run's tokens cost at `prices`: the input beyond the cached part at the input price, the cached part at
/// the cached price, and the output at the output price.
pub fn token_cost(usage: &TokenUsage, prices: &TokenPrices) -> f64 {
    let uncached = (usage.input - usage.cached_input).max(0.0);
    (uncached * prices.input + usage.cached_input * prices.cached_input + usage.output * prices.output) / 1_000_000.0
}

/// Reads a client's prices. Input and output must be above 0, since a free model would never reach a price cap;
/// cached input may be 0.
pub fn read_token_prices(fields: &Fields<'_>) -> Result<TokenPrices, ShapeError> {
    Ok(TokenPrices {
        cached_input: price(fields, "cached_input", false)?,
        input: price(fields, "input", true)?,
        output: price(fields, "output", true)?,
    })
}

/// One price, which must be above 0 or at least 0. The message names the price under `prices`, as the node reader
/// words it, whatever path the reader was given.
fn price(fields: &Fields<'_>, key: &str, above_zero: bool) -> Result<f64, ShapeError> {
    let value = fields.number(key)?;
    let (wrong, least) = if above_zero { (value <= 0.0, "above 0") } else { (value < 0.0, "at least 0") };
    if wrong {
        return Err(ShapeError(format!("prices.{key} must be {least} USD per million tokens")));
    }
    Ok(value)
}
