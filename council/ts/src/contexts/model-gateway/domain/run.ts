import { isObject, ModelGatewayError, type ModelPrices } from './config.js'

/** The one mode this ticket runs; writing runs are #299's. */
export type RunMode = 'read-only'

/** The clients this ticket can hand a delegation to: Vibe only. */
export const RUN_CLIENTS = ['vibe'] as const

/** A run with no time limit of its own gets this many seconds. */
export const DEFAULT_TIME_LIMIT_S = 900

/** What a run's prompt carries: code, and history such as commit messages and issue text. */
export interface Carries {
  readonly code: boolean
  readonly history: boolean
}

/**
 * One delegation: a named workflow's task handed to one client model. The
 * input is exactly what the caller sends; everything optional has a default
 * the run applies, never a second way to say the same thing.
 */
export interface RunInput {
  readonly workflow: string
  readonly client: string
  readonly model: string
  readonly mode: RunMode
  readonly prompt: string
  readonly repo?: string
  readonly carries?: Carries
  readonly time_limit_s: number
  readonly max_price_usd?: number
}

/** Reads a run's input, or says what is wrong with it. */
export function readRunInput(raw: unknown): RunInput {
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  if (typeof raw.workflow !== 'string' || raw.workflow === '') {
    throw new ModelGatewayError('invalid-input', 'a run must name its workflow')
  }
  if (typeof raw.client !== 'string' || !(RUN_CLIENTS as readonly string[]).includes(raw.client)) {
    throw new ModelGatewayError('not-supported-yet', `only ${RUN_CLIENTS.join(', ')} can run a delegation, not ${shown(raw.client)}`)
  }
  if (raw.mode !== 'read-only') {
    throw new ModelGatewayError('not-supported-yet', `only read-only runs are supported, not ${shown(raw.mode)}`)
  }
  if (typeof raw.model !== 'string' || raw.model === '') {
    throw new ModelGatewayError('invalid-input', 'model must be a non-empty string')
  }
  if (typeof raw.prompt !== 'string' || raw.prompt === '') {
    throw new ModelGatewayError('invalid-input', 'prompt must be a non-empty string')
  }
  const repo = readOptionalString(raw.repo, 'repo')
  const carries = readCarries(raw.carries)
  const timeLimit = raw.time_limit_s === undefined ? DEFAULT_TIME_LIMIT_S : requiredWholeNumber(raw.time_limit_s, 'time_limit_s')
  if (timeLimit <= 0) throw new ModelGatewayError('invalid-input', 'time_limit_s must be a whole number of seconds above 0')
  const maxPrice = raw.max_price_usd === undefined ? undefined : requiredNumberAbove0(raw.max_price_usd, 'max_price_usd')
  return {
    client: raw.client,
    mode: 'read-only',
    model: raw.model,
    prompt: raw.prompt,
    time_limit_s: timeLimit,
    workflow: raw.workflow,
    ...(carries === undefined ? {} : { carries }),
    ...(maxPrice === undefined ? {} : { max_price_usd: maxPrice }),
    ...(repo === undefined ? {} : { repo }),
  }
}

function readOptionalString(value: unknown, name: string): string | undefined {
  if (value === undefined) return undefined
  if (typeof value !== 'string' || value === '') throw new ModelGatewayError('invalid-input', `${name} must be a non-empty string`)
  return value
}

/** What the run claims its prompt carries; the caller marks it, the gateway checks the marks against the policy. */
function readCarries(value: unknown): Carries | undefined {
  if (value === undefined) return undefined
  if (!isObject(value) || typeof value.code !== 'boolean' || typeof value.history !== 'boolean') {
    throw new ModelGatewayError('invalid-input', 'carries must name code and history, each true or false')
  }
  return { code: value.code, history: value.history }
}

function requiredWholeNumber(value: unknown, name: string): number {
  if (typeof value !== 'number' || !Number.isInteger(value)) {
    throw new ModelGatewayError('invalid-input', `${name} must be a whole number`)
  }
  return value
}

function requiredNumberAbove0(value: unknown, name: string): number {
  if (typeof value !== 'number' || !Number.isFinite(value) || value <= 0) {
    throw new ModelGatewayError('invalid-input', `${name} must be a number above 0`)
  }
  return value
}

/** What a refused input named, as the message shows it. */
function shown(value: unknown): string {
  return value === undefined ? 'missing' : JSON.stringify(value)
}

/**
 * The prompt the delegate receives: the caller's prompt, and the instruction
 * that ends its answer with a SUMMARY line the gateway can cut to the
 * summary's limit.
 */
export function wrappedPrompt(prompt: string, summaryMaxChars: number): string {
  return `${prompt}\n\nEnd your answer with a line "SUMMARY:" followed by at most ${String(summaryMaxChars)} characters that summarise the result.`
}

/**
 * The run's summary: the text after the delegate's last SUMMARY line, cut at
 * the limit; when it wrote none, the last characters of its final answer.
 */
export function summaryOf(answer: string, summaryMaxChars: number): string {
  const at = answer.lastIndexOf('SUMMARY:')
  if (at === -1) return answer.slice(-summaryMaxChars)
  return answer.slice(at + 'SUMMARY:'.length).trimStart().slice(0, summaryMaxChars)
}

/**
 * The last assistant message of a Vibe journal, or undefined when stdout
 * holds none: the journal is one JSON array of entries, an assistant
 * message being one whose content joins into its text.
 */
export function finalAnswerOf(stdout: string): string | undefined {
  let journal: unknown
  try {
    journal = JSON.parse(stdout)
  } catch {
    return undefined
  }
  if (!Array.isArray(journal)) return undefined
  const last = journal.findLast(
    (entry): entry is { content?: unknown[]; role?: unknown; type?: unknown } =>
      isObject(entry) && entry.type === 'message' && entry.role === 'assistant',
  )
  if (last === undefined || !Array.isArray(last.content)) return undefined
  const text = last.content.map((part) => (isObject(part) && typeof part.text === 'string' ? part.text : '')).join('')
  return text === '' ? undefined : text
}

/** The tokens a run used, as Vibe's export counts them; a run that reported none used none. */
export interface RunTokens {
  readonly input: number
  readonly output: number
  readonly cached: number
}

export const NO_TOKENS: RunTokens = { cached: 0, input: 0, output: 0 }

/** The export's token counts, or none when the export holds none. */
export function runTokens(exported: unknown): RunTokens {
  const usage = isObject(exported) && isObject(exported.usage) ? exported.usage : undefined
  const count = (value: unknown): number => (typeof value === 'number' && Number.isFinite(value) && value >= 0 ? value : 0)
  if (usage === undefined) return NO_TOKENS
  return { cached: count(usage.cached_input_tokens), input: count(usage.input_tokens), output: count(usage.output_tokens) }
}

/**
 * The run's dollars: Vibe's own figure when it priced the run, else the
 * tokens times the catalogue price, else 0 when nothing prices it.
 */
export function runCost(exported: unknown, tokens: RunTokens, prices: ModelPrices | undefined): number {
  const charged = isObject(exported) ? exported.cost_usd : undefined
  if (typeof charged === 'number' && Number.isFinite(charged) && charged > 0) return charged
  if (prices === undefined) return 0
  const uncached = Math.max(tokens.input - tokens.cached, 0)
  return (uncached * prices.input + tokens.cached * prices.cached_input + tokens.output * prices.output) / 1_000_000
}

/** Why a run failed: a slug the result's `reason` and the ledger's outcome carry. */
export function runFailureReason(launch: { exitCode: number; timedOut?: boolean }, answer: string | undefined): string | undefined {
  if (launch.timedOut === true) return 'timeout'
  if (launch.exitCode !== 0) return `exit-${String(launch.exitCode)}`
  if (answer === undefined) return 'parse-error'
  return undefined
}

/** A UTC moment as a run id's first half: `20261104T105215Z`. */
export function compactTimestamp(at: Date): string {
  return at.toISOString().replace(/[-:]/g, '').replace(/\.\d{3}/, '')
}

/** A run's id: the UTC moment it started, and a short suffix that keeps two runs of one second apart. */
export function runId(at: Date, suffix: string): string {
  return `${compactTimestamp(at)}-${suffix}`
}

/** Whether a client model has a read-only proof that passed on this machine. */
export type ReadOnlyProof =
  | { readonly status: 'proven'; readonly critic: string }
  | { readonly status: 'unproven'; readonly reason: string }

/** The files setup and critique leave on this machine, as they were read; any of them may be missing. */
export interface ProofFiles {
  /** Critique's routing: the critic clients, each with the model it runs. */
  readonly routing: string | undefined
  /** This machine's routing override, merged over the shipped routing client by client. */
  readonly routingOverride: string | undefined
  /** Setup's recorded results, one per check id. */
  readonly setupResults: string | undefined
}

/**
 * The read-only proof of a client model: setup's `read-only:<critic client>`
 * result for the Vibe critic client whose model this is in critique's
 * routing. A run names a model by the alias `models` prints, while the
 * routing names it by the model's own id, so a critic client matches on the
 * alias or on any name the catalogue keeps for it. A model no critic client
 * runs, and a proof that is missing, failed or unproven, are all unproven: a
 * run goes in place only on a proof that passed.
 */
export function readOnlyProof(files: ProofFiles, model: string, names: readonly string[] = []): ReadOnlyProof {
  const routing = parseJsonObject(files.routing)
  if (routing === undefined) {
    return unproven("critique's routing is not installed, so no Vibe critic client proves a model read-only")
  }
  const critic = criticFor(routing.clients, parseJsonObject(files.routingOverride)?.clients, [model, ...names])
  if (critic === undefined) return unproven(`no Vibe critic client in critique's routing runs ${model}`)
  const result = checkResult(parseJsonObject(files.setupResults)?.checks, `read-only:${critic}`)
  if (result !== 'pass') return unproven(`setup's read-only proof for ${critic} is ${result ?? 'not recorded'}`)
  return { critic, status: 'proven' }
}

function unproven(reason: string): ReadOnlyProof {
  return { reason, status: 'unproven' }
}

/** The first Vibe critic client, in name order, whose model is one of `models`, with the override applied over the shipped routing. */
function criticFor(routing: unknown, override: unknown, models: readonly string[]): string | undefined {
  const clients = new Map<string, Readonly<Record<string, unknown>>>()
  for (const source of [routing, override]) {
    if (!isObject(source)) continue
    for (const [name, entry] of Object.entries(source)) {
      if (isObject(entry)) clients.set(name, { ...clients.get(name), ...entry })
    }
  }
  return [...clients]
    .filter(([, entry]) => entry.kind === 'vibe' && typeof entry.model === 'string' && models.includes(entry.model))
    .map(([name]) => name)
    .sort()[0]
}

/** One check's recorded result, or undefined when setup recorded none. */
function checkResult(checks: unknown, id: string): string | undefined {
  if (!isObject(checks)) return undefined
  const check = checks[id]
  return isObject(check) && typeof check.result === 'string' ? check.result : undefined
}

/** Parsed JSON that is an object, or undefined when the text is missing or is not one. */
function parseJsonObject(text: string | undefined): Readonly<Record<string, unknown>> | undefined {
  if (text === undefined) return undefined
  try {
    const parsed: unknown = JSON.parse(text)
    return isObject(parsed) ? parsed : undefined
  } catch {
    return undefined
  }
}
