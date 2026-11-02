import { JsonShapeError, type JsonFields } from './json.js'

/**
 * What a client's model costs, in USD per million tokens. Cached input is
 * prompt the provider served from its cache: Mistral bills it at 10% of the
 * input price, and a critic re-reads its whole conversation every turn, so
 * most of its input is cached.
 */
export interface TokenPrices {
  readonly input: number
  readonly cached_input: number
  readonly output: number
}

/** The tokens one launch used. `input` counts every input token, the cached ones included. */
export interface TokenUsage {
  readonly input: number
  readonly cached_input: number
  readonly output: number
}

/** Uncached input at the input price, cached input at the cached price, output at the output price. */
export function tokenCost(usage: TokenUsage, prices: TokenPrices): number {
  const uncached = Math.max(usage.input - usage.cached_input, 0)
  return (uncached * prices.input + usage.cached_input * prices.cached_input + usage.output * prices.output) / 1_000_000
}

/**
 * Reads a client's prices. Input and output must be above 0, since a free
 * model would never reach a price cap; cached input may be 0.
 */
export function readTokenPrices(fields: JsonFields): TokenPrices {
  const price = (key: keyof TokenPrices, least: 'positive' | 'zero') => {
    const value = fields.number(key)
    if (least === 'positive' ? value <= 0 : value < 0) {
      throw new JsonShapeError(`prices.${key} must be ${least === 'positive' ? 'above 0' : 'at least 0'} USD per million tokens`)
    }
    return value
  }
  return { cached_input: price('cached_input', 'zero'), input: price('input', 'positive'), output: price('output', 'positive') }
}
