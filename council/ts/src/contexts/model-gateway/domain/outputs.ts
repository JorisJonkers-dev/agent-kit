import { isObject, type JsonObject } from './config.js'
import { chargedOf, finalAnswerOf, hermesUsage, NO_TOKENS, oneShotAnswer, runTokens, type ClientError, type RunTokens } from './run.js'

/**
 * What one client's own output says about a run: the final answer, the tokens
 * it counted, the dollars it charged when it priced the run, why it failed
 * when its output says so although it exited 0, and the error its own
 * structured output carries.
 */
export interface ClientOutput {
  /** The delegate's final answer; undefined when the output holds none. */
  readonly answer: string | undefined
  readonly tokens: RunTokens
  /** The dollars the client itself charged; undefined when it priced nothing. */
  readonly charged: number | undefined
  /** `timeout`, `denied-action` or `client-error`; undefined when the output reports no failure. */
  readonly failure?: string | undefined
  /** The error the client's own structured output carries; undefined when it reports none. */
  readonly error?: ClientError | undefined
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

/**
 * The error a client's structured output carries, as text the refusal
 * patterns can read: its `error` field, its stop reason, and the result of a
 * run it marks as an error. A client that reports no error has none.
 */
function structuredError(exported: unknown): ClientError | undefined {
  if (!isObject(exported)) return undefined
  const parts: string[] = []
  const error = exported.error
  let status: number | undefined
  if (typeof error === 'string') parts.push(error)
  else if (isObject(error)) {
    parts.push(...[error.message, error.type, error.code, error.name].filter((part): part is string => typeof part === 'string'))
    status = httpStatus(error)
  }
  const stop = exported.stop_reason ?? exported.stopReason
  if (typeof stop === 'string') parts.push(stop)
  if ((exported.is_error === true || exported.isError === true) && typeof exported.result === 'string') parts.push(exported.result)
  status ??= httpStatus(exported)
  if (parts.length === 0 && status === undefined) return undefined
  return { text: parts.join(' '), ...(status === undefined ? {} : { status }) }
}

/** The HTTP status an object names, when it names one as a whole number. */
function httpStatus(value: JsonObject): number | undefined {
  for (const key of ['status', 'status_code', 'statusCode', 'code']) {
    const found = value[key]
    if (typeof found === 'number' && Number.isInteger(found)) return found
  }
  return undefined
}

const NOTHING: ClientOutput = { answer: undefined, charged: undefined, tokens: NO_TOKENS }

/** Vibe: the journal on stdout holds the answer, and the export it wrote holds the tokens, the dollars and any error. */
export function vibeOutput(stdout: string, exported: unknown): ClientOutput {
  return { answer: finalAnswerOf(stdout), charged: chargedOf(exported), error: structuredError(exported), tokens: runTokens(exported) }
}

/**
 * Antigravity: one object with the `response` and a `usage` of token counts,
 * and no dollars (it is quota, not money), whether it printed that object
 * alone (`--output-format json`) or as the `result` event of a stream
 * (`--output-format stream-json`). agy reports `SUCCESS` and exits 0 for runs
 * that produced nothing, so the fields decide: a print timeout, a denied
 * action or another status is a failure.
 */
export function agyOutput(stdout: string): ClientOutput {
  const result = agyResultOf(stdout)
  if (!isObject(result)) return NOTHING
  const usage = isObject(result.usage) ? result.usage : {}
  return {
    answer: textOf(result.response),
    charged: undefined,
    error: structuredError(result),
    failure: agyFailure(result),
    tokens: { cached: count(usage.cache_read_tokens), input: count(usage.input_tokens), output: count(usage.output_tokens) },
  }
}

/**
 * The result an agy run printed: the `result` of its last result event when it
 * printed a stream, else the one object it printed, else nothing.
 */
function agyResultOf(stdout: string): unknown {
  const events = stdout.split('\n').map(parseJson).filter(isObject)
  const streamed = events.findLast((event) => event.event === 'result')?.result
  return streamed ?? events.findLast((event) => event.response !== undefined || event.usage !== undefined)
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
    error: structuredError(result),
    failure: result.is_error === true ? 'client-error' : undefined,
    tokens: {
      cached,
      input: count(usage.input_tokens) + cached + count(usage.cache_creation_input_tokens),
      output: count(usage.output_tokens),
    },
  }
}

/**
 * Hermes, one-shot: `hermes -z` prints the response text alone, and the usage
 * report it wrote holds the tokens it counted, the dollars it estimated and
 * whether it counts the run as failed. A report that is missing or holds
 * nonsense counts no tokens and no dollars, and a run it does not call failed
 * is not failed.
 */
export function hermesOutput(stdout: string, usage: unknown): ClientOutput {
  const report = hermesUsage(usage)
  return {
    answer: oneShotAnswer(stdout),
    charged: report.cost,
    failure: report.failed ? 'failed' : undefined,
    tokens: report.tokens,
  }
}

/**
 * Codex, `exec --json`: one event per line. The answer is the text of the last
 * `agent_message` item; every `turn.completed` adds the tokens its turn
 * reported, its cached prompt counted apart from its input as Vibe's export
 * counts it, and its reasoning tokens already inside its output; an `error` or
 * a `turn.failed` event fails the run. Codex prices nothing, so its dollars
 * come from the catalogue's prices.
 */
export function codexOutput(stdout: string): ClientOutput {
  let answer: string | undefined
  let error: ClientError | undefined
  let failure: string | undefined
  const sum = { cached: 0, input: 0, output: 0 }
  for (const line of stdout.split('\n')) {
    const event = parseJson(line)
    if (!isObject(event)) continue
    const item = isObject(event.item) ? event.item : {}
    if (event.type === 'item.completed' && item.type === 'agent_message' && textOf(item.text) !== undefined) {
      answer = String(item.text)
    } else if (event.type === 'turn.completed') {
      const usage = isObject(event.usage) ? event.usage : {}
      sum.cached += count(usage.cached_input_tokens)
      sum.input += count(usage.input_tokens)
      sum.output += count(usage.output_tokens)
    } else if (event.type === 'error' || event.type === 'turn.failed') {
      failure = 'client-error'
      error = structuredError(event) ?? saidOf(event)
    }
  }
  return { answer, charged: undefined, error, failure, tokens: sum }
}

/** What an event says in its own `message`, as the error the refusal patterns read. */
function saidOf(event: JsonObject): ClientError | undefined {
  const said = textOf(event.message)
  return said === undefined ? undefined : { text: said }
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
  let error: ClientError | undefined
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
    } else if (event.type === 'error') {
      failure = 'client-error'
      error = structuredError(event)
    }
  }
  return { answer, charged: chargedAmount(charged), error, failure, tokens: sum }
}
