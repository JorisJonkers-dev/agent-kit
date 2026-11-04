import { isObject, type JsonObject } from './config.js'
import { chargedOf, finalAnswerOf, NO_TOKENS, runTokens, type RunTokens } from './run.js'

/**
 * What one client's own output says about a run: the final answer, the tokens
 * it counted, the dollars it charged when it priced the run, and why it
 * failed when its output says so although it exited 0.
 */
export interface ClientOutput {
  /** The delegate's final answer; undefined when the output holds none. */
  readonly answer: string | undefined
  readonly tokens: RunTokens
  /** The dollars the client itself charged; undefined when it priced nothing. */
  readonly charged: number | undefined
  /** `timeout`, `denied-action` or `client-error`; undefined when the output reports no failure. */
  readonly failure?: string | undefined
}

/** A count in a client's usage block; anything but a finite number from 0 up is no count. */
function count(value: unknown): number {
  return typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0
}

/** A non-blank string, or undefined. */
function textOf(value: unknown): string | undefined {
  return typeof value === 'string' && value.trim() !== '' ? value : undefined
}

/** Parsed JSON, or undefined when the text is not JSON. */
function parseJson(text: string): unknown {
  try {
    return JSON.parse(text) as unknown
  } catch {
    return undefined
  }
}

/** The dollars a client reported when it priced the run: a number above 0, else none. */
function chargedAmount(value: unknown): number | undefined {
  return typeof value === 'number' && Number.isFinite(value) && value > 0 ? value : undefined
}

const NOTHING: ClientOutput = { answer: undefined, charged: undefined, tokens: NO_TOKENS }

/** Vibe: the journal on stdout holds the answer, and the export it wrote holds the tokens and the dollars. */
export function vibeOutput(stdout: string, exported: unknown): ClientOutput {
  return { answer: finalAnswerOf(stdout), charged: chargedOf(exported), tokens: runTokens(exported) }
}

/**
 * Antigravity, `--output-format json`: one object with the `response` and a
 * `usage` of token counts, and no dollars (it is quota, not money). agy
 * reports `SUCCESS` and exits 0 for runs that produced nothing, so the fields
 * decide: a print timeout, a denied action or another status is a failure.
 */
export function agyOutput(stdout: string): ClientOutput {
  const result = parseJson(stdout)
  if (!isObject(result)) return NOTHING
  const usage = isObject(result.usage) ? result.usage : {}
  return {
    answer: textOf(result.response),
    charged: undefined,
    failure: agyFailure(result),
    tokens: { cached: count(usage.cache_read_tokens), input: count(usage.input_tokens), output: count(usage.output_tokens) },
  }
}

function agyFailure(result: JsonObject): string | undefined {
  if (result.finished_reason === 'print_timeout') return 'timeout'
  if (Array.isArray(result.denied_actions) && result.denied_actions.length > 0) return 'denied-action'
  if (typeof result.status === 'string' && result.status !== 'SUCCESS') return 'client-error'
  return undefined
}

/**
 * Claude Code, `--output-format json`: one `result` object (or a list of
 * events that ends in one) with the answer in `result`, the dollars in
 * `total_cost_usd` and the tokens in `usage`. Anthropic counts cached prompt
 * apart from `input_tokens`, so the input here is the sum, which is what a
 * Vibe run's input counts.
 */
export function claudeOutput(stdout: string): ClientOutput {
  const parsed = parseJson(stdout)
  const result: unknown = Array.isArray(parsed) ? (parsed as unknown[]).findLast((event) => isObject(event) && event.type === 'result') : parsed
  if (!isObject(result)) return NOTHING
  const usage = isObject(result.usage) ? result.usage : {}
  const cached = count(usage.cache_read_input_tokens)
  return {
    answer: textOf(result.result),
    charged: chargedAmount(result.total_cost_usd),
    failure: result.is_error === true ? 'client-error' : undefined,
    tokens: {
      cached,
      input: count(usage.input_tokens) + cached + count(usage.cache_creation_input_tokens),
      output: count(usage.output_tokens),
    },
  }
}

/**
 * OpenCode, `--format json`: one event per line. The answer is the text of the
 * last step that wrote any; every `step_finish` adds its dollars and tokens
 * (its cache is counted apart from its input, as Anthropic's is, and its
 * reasoning is billed as output); an `error` event fails the run.
 */
export function openCodeOutput(stdout: string): ClientOutput {
  let answer: string | undefined
  let fresh = true
  let charged = 0
  let failure: string | undefined
  const sum = { cached: 0, input: 0, output: 0 }
  for (const line of stdout.split('\n')) {
    const event = parseJson(line)
    if (!isObject(event)) continue
    const part = isObject(event.part) ? event.part : {}
    if (event.type === 'step_start') fresh = true
    else if (event.type === 'text' && textOf(part.text) !== undefined) {
      answer = fresh ? String(part.text) : `${answer ?? ''}${String(part.text)}`
      fresh = false
    } else if (event.type === 'step_finish') {
      const tokens = isObject(part.tokens) ? part.tokens : {}
      const cache = isObject(tokens.cache) ? tokens.cache : {}
      charged += count(part.cost)
      sum.cached += count(cache.read)
      sum.input += count(tokens.input) + count(cache.read) + count(cache.write)
      sum.output += count(tokens.output) + count(tokens.reasoning)
    } else if (event.type === 'error') failure = 'client-error'
  }
  return { answer, charged: chargedAmount(charged), failure, tokens: sum }
}
