import { isObject, ModelGatewayError, type ModelPrices } from './config.js'

/** What a run may do: read the repository, or change it in a worktree of its own. */
export type RunMode = 'read-only' | 'write'

/** The clients a delegation can be handed to: Vibe, Antigravity, Claude Code and OpenCode. */
export const RUN_CLIENTS = ['vibe', 'agy', 'claude', 'opencode'] as const
export type RunClient = (typeof RUN_CLIENTS)[number]

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
  /** A writing run's starting point: the ref its worktree is created off. */
  readonly base?: string
  /** A writing run's check: a command the gateway runs with `sh -c` in the worktree once the delegate ends. */
  readonly check?: string
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
  if (raw.mode !== 'read-only' && raw.mode !== 'write') {
    throw new ModelGatewayError('not-supported-yet', `only read-only and write runs are supported, not ${shown(raw.mode)}`)
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
  const writing = readWriting(raw, repo)
  return {
    client: raw.client,
    mode: raw.mode,
    model: raw.model,
    prompt: raw.prompt,
    time_limit_s: timeLimit,
    workflow: raw.workflow,
    ...(carries === undefined ? {} : { carries }),
    ...(maxPrice === undefined ? {} : { max_price_usd: maxPrice }),
    ...(repo === undefined ? {} : { repo }),
    ...writing,
  }
}

/**
 * What a writing run adds to the input: its base and its check, and a
 * repository to work in. A read-only run has neither, and one that names
 * them is refused rather than having them ignored.
 */
function readWriting(raw: Record<string, unknown>, repo: string | undefined): { base?: string; check?: string } {
  if (raw.mode !== 'write') {
    if (raw.base !== undefined || raw.check !== undefined) {
      throw new ModelGatewayError('invalid-input', 'base and check are for writing runs only')
    }
    return {}
  }
  if (repo === undefined) {
    throw new ModelGatewayError('write-needs-repository', 'a writing run changes a repository, so it must name its repo')
  }
  if (typeof raw.base !== 'string' || raw.base === '') {
    throw new ModelGatewayError('invalid-input', 'a writing run must name its base, a ref the worktree starts from')
  }
  if (typeof raw.check !== 'string' || raw.check === '') {
    throw new ModelGatewayError('invalid-input', 'a writing run must name its check, a command that tests the change')
  }
  return { base: raw.base, check: raw.check }
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

/** What a writing delegate is told: where it may edit, and that committing is the gateway's caller's. */
export const WRITE_RULE = 'Edit only files inside the working directory. Do not commit.'

/**
 * The prompt the delegate receives: the caller's prompt, for a writing run
 * the rule about where it edits, and the instruction that ends its answer
 * with a SUMMARY line the gateway can cut to the summary's limit.
 */
export function wrappedPrompt(prompt: string, summaryMaxChars: number, mode: RunMode = 'read-only'): string {
  const rule = mode === 'write' ? `\n\n${WRITE_RULE}` : ''
  return `${prompt}${rule}\n\nEnd your answer with a line "SUMMARY:" followed by at most ${String(summaryMaxChars)} characters that summarise the result.`
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

/** The dollars Vibe's export says it charged; undefined when it priced nothing. */
export function chargedOf(exported: unknown): number | undefined {
  const charged = isObject(exported) ? exported.cost_usd : undefined
  return typeof charged === 'number' && Number.isFinite(charged) && charged > 0 ? charged : undefined
}

/**
 * The run's dollars: Vibe's own figure when it priced the run, else the
 * tokens times the catalogue price, else 0 when nothing prices it.
 */
export function runCost(exported: unknown, tokens: RunTokens, prices: ModelPrices | undefined): number {
  return priceRun(chargedOf(exported), tokens, prices)
}

/**
 * The run's dollars whatever the client: the figure the client charged when
 * it priced the run, else the tokens times the catalogue price, else 0.
 */
export function priceRun(charged: number | undefined, tokens: RunTokens, prices: ModelPrices | undefined): number {
  if (charged !== undefined) return charged
  if (prices === undefined) return 0
  const uncached = Math.max(tokens.input - tokens.cached, 0)
  return (uncached * prices.input + tokens.cached * prices.cached_input + tokens.output * prices.output) / 1_000_000
}

/**
 * The error a client's own structured output carries: the text it printed
 * about it, and the HTTP status where it names one.
 */
export interface ClientError {
  readonly text: string
  readonly status?: number
}

/**
 * What a client's own output reported about a run's failure: the failure slug
 * its structured output names, and the error that output carries. Each
 * client's parser in `outputs.ts` fills it in from that client's own output.
 */
export interface ClientReport {
  readonly failure?: string | undefined
  readonly error?: ClientError | undefined
}

/**
 * Why a run failed: a slug the result's `reason` and the ledger's outcome
 * carry. A client that says it was rate-limited or refused the credentials
 * gets that slug rather than its exit code or its own failure slug, so
 * availability can find it in the ledger afterwards. The client's own report
 * of a failure (`timeout`, `denied-action`, `client-error`) counts when it
 * exited 0, and comes before a missing answer.
 *
 * The slug is read from the client's own error channel only: its stderr, the
 * error its structured output carries, and its exit code. Never from the
 * delegate's answer, which is the model's text and may quote any of these
 * words without meaning them.
 */
export function runFailureReason(
  launch: { exitCode: number; timedOut?: boolean; stderr?: string },
  answer: string | undefined,
  reported?: ClientReport,
): string | undefined {
  if (launch.timedOut === true) return 'timeout'
  const refusal = refusalOf(launch, reported?.error)
  if (refusal !== undefined) return refusal
  if (launch.exitCode !== 0) return `exit-${String(launch.exitCode)}`
  if (reported?.failure !== undefined) return reported.failure
  if (answer === undefined) return 'parse-error'
  return undefined
}

/** The providers an OpenCode run may reach when the machine names none. */
export const DEFAULT_OPENCODE_PROVIDERS: readonly string[] = ['opencode-go']

/**
 * OpenCode's own pay-per-token providers (`opencode/<model>` is its Zen). The
 * owner's rule is that a run never reaches one, so no allowlist can open it.
 */
const PAY_PER_TOKEN_PROVIDERS: readonly string[] = ['opencode']

/** The provider of an OpenCode model named `<provider>/<model>`; undefined when it names none. */
export function openCodeProvider(model: string): string | undefined {
  const slash = model.indexOf('/')
  return slash <= 0 || slash === model.length - 1 ? undefined : model.slice(0, slash)
}

/**
 * Refuses an OpenCode model whose provider the machine's
 * `clients.opencode.providers` (else `opencode-go` alone) does not name, and
 * every pay-per-token one whatever it names.
 */
export function requireAllowedProvider(model: string, allowed: readonly string[] | undefined): void {
  const providers = allowed ?? DEFAULT_OPENCODE_PROVIDERS
  const provider = openCodeProvider(model)
  if (provider === undefined) {
    throw new ModelGatewayError(
      'provider-not-allowed',
      `${model}: an OpenCode model is named <provider>/<model>, and the allowed providers are ${providers.join(', ')}`,
    )
  }
  if (PAY_PER_TOKEN_PROVIDERS.includes(provider) || !providers.includes(provider)) {
    throw new ModelGatewayError(
      'provider-not-allowed',
      `${model}: the provider ${provider} is not allowed (clients.opencode.providers: ${providers.join(', ')}), and OpenCode’s pay-per-token models are never reached`,
    )
  }
}

/** What a client's own error channel says refused the run, when it says so. */
function refusalOf(
  launch: { exitCode: number; stderr?: string },
  error: ClientError | undefined,
): 'rate-limited' | 'auth' | undefined {
  const said = `${launch.stderr ?? ''}\n${error?.text ?? ''}`
  if (RATE_LIMITED.test(said) || error?.status === 429 || launch.exitCode === 429) return 'rate-limited'
  if (
    REFUSED_CREDENTIALS.test(said) ||
    error?.status === 401 ||
    error?.status === 403 ||
    launch.exitCode === 401 ||
    launch.exitCode === 403
  ) {
    return 'auth'
  }
  return undefined
}

/** What a client prints when it is out of allowance for now. */
const RATE_LIMITED = /rate.?limit|too many requests|quota exceeded/i

/** What a client prints when it will not run without a login or a valid key. */
const REFUSED_CREDENTIALS = /unauthori[sz]ed|unauthenticated|not logged in|log ?in required|invalid api key|authentication failed|forbidden/i

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
 * The critic client kind whose read-only proof counts for a client, and what
 * the messages call it. Only Vibe and Antigravity have a critic: the others
 * (Claude Code, OpenCode, Codex, Hermes) are never proven.
 */
const CRITIC_KINDS: Readonly<Record<string, { readonly kind: string; readonly label: string }>> = {
  agy: { kind: 'agy', label: 'Antigravity' },
  vibe: { kind: 'vibe', label: 'Vibe' },
}

/**
 * The read-only proof of a client model: setup's `read-only:<critic client>`
 * result for the critic client of the client's own kind (Vibe, or Antigravity
 * as the Gemini critic) whose model this is in critique's routing. A run names
 * a model by the alias `models` prints, while the routing names it by the
 * model's own id, so a critic client matches on the alias or on any name the
 * catalogue keeps for it. A client with no critic kind, a model no critic
 * client runs, and a proof that is missing, failed or unproven, are all
 * unproven: a run goes in place only on a proof that passed.
 */
export function readOnlyProof(files: ProofFiles, model: string, names: readonly string[] = [], client = 'vibe'): ReadOnlyProof {
  const critic = CRITIC_KINDS[client]
  if (critic === undefined) return unproven(`${client} has no critic client in critique's routing, so nothing proves it read-only`)
  const routing = parseJsonObject(files.routing)
  if (routing === undefined) {
    return unproven(`critique's routing is not installed, so no ${critic.label} critic client proves a model read-only`)
  }
  const name = criticFor(routing.clients, parseJsonObject(files.routingOverride)?.clients, critic.kind, [model, ...names])
  if (name === undefined) return unproven(`no ${critic.label} critic client in critique's routing runs ${model}`)
  const result = checkResult(parseJsonObject(files.setupResults)?.checks, `read-only:${name}`)
  if (result !== 'pass') return unproven(`setup's read-only proof for ${name} is ${result ?? 'not recorded'}`)
  return { critic: name, status: 'proven' }
}

function unproven(reason: string): ReadOnlyProof {
  return { reason, status: 'unproven' }
}

/** The first critic client of a kind, in name order, whose model is one of `models`, with the override applied over the shipped routing. */
function criticFor(routing: unknown, override: unknown, kind: string, models: readonly string[]): string | undefined {
  const clients = new Map<string, Readonly<Record<string, unknown>>>()
  for (const source of [routing, override]) {
    if (!isObject(source)) continue
    for (const [name, entry] of Object.entries(source)) {
      if (isObject(entry)) clients.set(name, { ...clients.get(name), ...entry })
    }
  }
  return [...clients]
    .filter(([, entry]) => entry.kind === kind && typeof entry.model === 'string' && models.includes(entry.model))
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
