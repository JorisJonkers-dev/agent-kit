/**
 * Which of council's engines the model gateway guards, and how a council run
 * is named to it. Pure: the gateway itself is reached from the adapters.
 */

/** The engines whose processes the gateway authorises before they start and records when they end; any other engine (`generic`, a configured one) is council's own. */
export const GUARDED_ENGINES: readonly string[] = ['claude', 'codex']

/** Whether the gateway guards this engine. */
export function isGuardedEngine(name: string): boolean {
  return GUARDED_ENGINES.includes(name)
}

/** An engine and the model it runs, as `--engine codex:gpt-5.5` names them. */
export interface EngineSpec {
  readonly name: string
  readonly model: string
}

/** Reads `<engine>:<model>`; a guarded engine needs its model, because the gateway holds the run against the model's price. */
export function parseEngineSpec(spec: string): EngineSpec {
  const colon = spec.indexOf(':')
  const name = colon === -1 ? spec : spec.slice(0, colon)
  const model = colon === -1 ? '' : spec.slice(colon + 1)
  if (name === '' || model === '') {
    throw new Error(`--engine must name an engine and its model, as <engine>:<model> (for example codex:gpt-5.5), not ${JSON.stringify(spec)}`)
  }
  return { model, name }
}

/** What a gateway workflow name may hold: a letter or digit first, then letters, digits and `._:/-`, at most 80. */
const WORKFLOW_NAME_LIMIT = 80

/**
 * The workflow a council run's engine runs belong to in the gateway: the run's
 * id, with anything the gateway's name rule refuses turned into `-`. The
 * owner's grant names this workflow, so it is the one name the grant and the
 * run share.
 */
export function engineWorkflowName(runId: string): string {
  const safe = runId
    .replaceAll(/[^A-Za-z0-9._:/-]/g, '-')
    .replace(/^[^A-Za-z0-9]+/, '')
    .slice(0, WORKFLOW_NAME_LIMIT)
  return safe === '' ? 'council' : safe
}

/** Why council refused to start a guarded engine itself, before the gateway was asked. */
export class EngineGuardError extends Error {
  constructor(
    readonly code: 'engine-lease-mismatch' | 'engine-mismatch' | 'engine-model-override' | 'engine-wrapper' | 'no-engine-guard',
    message: string,
  ) {
    super(message)
    this.name = 'EngineGuardError'
  }
}

/** The file name a command or argument is, without its directory. */
function baseName(path: string): string {
  return path.slice(path.lastIndexOf('/') + 1)
}

/** Programs that start other programs: any of them given `codex` or `claude` would run the engine without the gateway. */
const WRAPPERS: readonly string[] = ['bash', 'command', 'dash', 'env', 'exec', 'ksh', 'nohup', 'node', 'npx', 'python', 'python3', 'sh', 'sudo', 'time', 'xargs', 'zsh']

/**
 * The guarded engine a command that is none would start past the gateway, or
 * undefined when it starts none. A program-looking argument (a `codex` or
 * `claude` file name, with or without a directory) names it for any command;
 * for a wrapper or interpreter, so does the word in any argument, which is how
 * `sh -c 'codex exec ...'` and `env sh -c ...` are caught. It errs on the side
 * of refusing.
 */
export function wrappedEngine(command: string, args: readonly string[]): string | undefined {
  for (const arg of args) {
    const named = baseName(arg)
    if (isGuardedEngine(named)) return named
  }
  if (!WRAPPERS.includes(baseName(command))) return undefined
  for (const arg of args) {
    const word = /\b(claude|codex)\b/.exec(arg)
    if (word?.[1] !== undefined) return word[1]
  }
  return undefined
}

/** The models a guarded command's own arguments pick, and the arguments that could change the model in a way the gateway cannot see. */
export interface ModelPicks {
  readonly models: readonly string[]
  readonly overrides: readonly string[]
}

/**
 * Reads what a `codex` or `claude` command's arguments say about its model:
 * `-m x`, `--model x`, `--model=x` and a `-c`/`--config` whose key is `model`
 * pick one; a `-c`/`--config` for `model_provider`, and a profile (`-p` for
 * codex, `--profile` for either; a profile can set the model), are overrides.
 */
export function modelPicks(engine: string, args: readonly string[]): ModelPicks {
  const models: string[] = []
  const overrides: string[] = []
  const config = (value: string | undefined): void => {
    const match = value === undefined ? null : /^\s*([A-Za-z0-9_.-]+)\s*=\s*(.*?)\s*$/s.exec(value)
    if (match === null) return
    const [, key = '', raw = ''] = match
    if (key === 'model') models.push(raw.replace(/^(["'])(.*)\1$/s, '$2'))
    if (key === 'model_provider') overrides.push(`${key}=${raw}`)
  }
  for (const [index, arg] of args.entries()) {
    const next = args[index + 1]
    if ((arg === '-m' || arg === '--model') && next !== undefined) models.push(next)
    else if (arg.startsWith('--model=')) models.push(arg.slice('--model='.length))
    else if (arg === '-c' || arg === '--config') config(next)
    else if (arg.startsWith('--config=')) config(arg.slice('--config='.length))
    else if (arg === '--profile' || arg.startsWith('--profile=') || (engine === 'codex' && arg === '-p')) overrides.push(arg)
  }
  return { models, overrides }
}

/** Where a `claude` or `codex` process is started from: the engine its command is, which is the only engine that command can be. */
export function commandEngine(command: string): string | undefined {
  const name = command.slice(command.lastIndexOf('/') + 1)
  return isGuardedEngine(name) ? name : undefined
}
