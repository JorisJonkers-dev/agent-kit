import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import {
  createModelGatewayCommands,
  ModelGatewayError,
  modelSources,
  NodeModelGatewayFiles,
  NodeModelGatewayGit,
  NodeModelGatewayLedger,
  NodeModelGatewayProcess,
  NodeModelGatewayProof,
  NodeModelGatewayWorkflows,
  type ModelGatewayCommands,
  type ModelGatewayDependencies,
} from '../contexts/model-gateway/index.js'

export interface ModelGatewayCliResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

/** The contract's name for what one run prints and how it exits. */
export type CliResult = ModelGatewayCliResult

export interface ModelGatewayRuntime {
  readonly commands: ModelGatewayCommands
  readFile(path: string): Promise<string>
}

export const MODEL_GATEWAY_USAGE = `usage: model-gateway models --json '{"repo": "/path/to/repo"}'
       model-gateway run --json '{"workflow": "spec-293", "client": "vibe",
                                 "model": "zai-glm-5-3", "mode": "read-only",
                                 "prompt": "read this", "repo": "/path/to/repo"}'
       model-gateway workflow start --json '{"name": "spec-293"}'
       model-gateway workflow end --json '{"name": "spec-293"}'
       model-gateway workflow list --json '{}'
       model-gateway <command> --input FILE

Each command reads one JSON object, from --json or from FILE, and prints one
JSON object. models lists every client model this machine can use now for the
repository the caller works in, with its client, model id, family, cost class,
difficulty tier, capability notes, where the list came from (live, config or
declared) and what the data policy lets a delegation to it carry. It names
each client it knows but cannot use, and says in "policy" why the data policy
listed what it did. Without "repo", or outside any repository, only inline
content may be sent.

run hands one read-only task to one client model and prints a short result:
its status, run id, workflow, client model, family, mode, tokens, cost, the
path of its full output and a summary of at most "summary_max_chars"
characters. It takes "workflow", "client", "model", "mode", "prompt", and
optionally "repo", "carries" (code and history), "time_limit_s" (900 by
default), "max_price_usd" and "provider" (which a hermes run must name, and
which this machine must allow). The workflow's life, the data policy and the
client model's read-only proof are checked before anything is launched. An
error prints "status": "error" with its "code" and exits 1.

workflow start, workflow end and workflow list keep a workflow's life: start
and end take "name", and list takes nothing. A workflow nobody ends within 24
hours counts as ended, and a run may not join one that ended.
`

/** The subcommands the gateway has; #294 ships models, #296 adds run, #297 the workflow commands. */
const COMMANDS = {
  models: 'models',
  run: 'run',
  'workflow end': 'workflowEnd',
  'workflow list': 'workflowList',
  'workflow start': 'workflowStart',
} as const satisfies Record<string, keyof ModelGatewayCommands>

/** The subcommands, as an error names them. */
const COMMAND_NAMES = 'models, run, workflow start, workflow end and workflow list'

/** The gateway's command line: one subcommand, one JSON object in, one JSON object out. */
export async function runModelGatewayCli(argv: readonly string[], runtime: ModelGatewayRuntime): Promise<ModelGatewayCliResult> {
  const [first] = argv
  if (first === '--help' || first === '-h') return { exitCode: 0, stderr: '', stdout: MODEL_GATEWAY_USAGE }
  const named = subcommand(argv)
  if (named === undefined) {
    return failure('unknown-command', `unknown subcommand ${unknownSubcommand(argv)}; the gateway has ${COMMAND_NAMES}`)
  }
  const { args, method, name } = named
  const input = parseArguments(args)
  if (input === undefined) return failure('invalid-input', `${name} takes one of --json '<object>' or --input <file>`, name)
  try {
    const text = 'json' in input ? input.json : await readInput(runtime, input.input)
    const result = await runtime.commands[method].run(parseInput(text))
    return { exitCode: 0, stderr: '', stdout: `${JSON.stringify(result, null, 2)}\n` }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return failure(error instanceof ModelGatewayError ? error.code : 'unexpected', message, name)
  }
}

/** The subcommand the arguments name, and the arguments after it; a two-word subcommand wins over a one-word one. */
function subcommand(argv: readonly string[]): { readonly args: readonly string[]; readonly method: keyof ModelGatewayCommands; readonly name: string } | undefined {
  const two = argv.slice(0, 2).join(' ')
  if (Object.hasOwn(COMMANDS, two)) return { args: argv.slice(2), method: COMMANDS[two as keyof typeof COMMANDS], name: two }
  const one = argv[0]
  if (one !== undefined && Object.hasOwn(COMMANDS, one)) {
    return { args: argv.slice(1), method: COMMANDS[one as keyof typeof COMMANDS], name: one }
  }
  return undefined
}

/** What an unknown subcommand is called in the error: both words when the first one is a group, else the one word. */
function unknownSubcommand(argv: readonly string[]): string {
  const [first, second] = argv
  if (first === undefined) return '(none)'
  const grouped = Object.keys(COMMANDS).some((name) => name.startsWith(`${first} `))
  return grouped ? `"${`${first} ${second ?? ''}`.trim()}"` : `"${first}"`
}

/** `--json <object>` or `--input <file>`, exactly one of them and nothing else. */
type Input = { readonly json: string } | { readonly input: string }

function parseArguments(args: readonly string[]): Input | undefined {
  const [flag, value, ...rest] = args
  if (value === undefined || rest.length > 0) return undefined
  if (flag === '--json') return { json: value }
  if (flag === '--input') return { input: value }
  return undefined
}

async function readInput(runtime: ModelGatewayRuntime, path: string): Promise<string> {
  try {
    return await runtime.readFile(path)
  } catch (error) {
    throw new ModelGatewayError('invalid-input', `${path}: cannot read the input: ${messageOf(error)}`)
  }
}

function parseInput(text: string): Readonly<Record<string, unknown>> {
  let raw: unknown
  try {
    raw = JSON.parse(text)
  } catch (error) {
    throw new ModelGatewayError('invalid-input', `the input is not JSON: ${messageOf(error)}`)
  }
  if (typeof raw !== 'object' || raw === null || Array.isArray(raw)) {
    throw new ModelGatewayError('invalid-input', 'the input must be a JSON object')
  }
  return raw as Readonly<Record<string, unknown>>
}

/** One error, as the contract prints it: one JSON object on stdout, and a line for the owner on stderr. */
function failure(code: string, message: string, name?: string): ModelGatewayCliResult {
  return {
    exitCode: 1,
    stderr: `model-gateway${name === undefined ? '' : ` ${name}`}: ${message}\n`,
    stdout: `${JSON.stringify({ status: 'error', code, message })}\n`,
  }
}

function messageOf(error: unknown): string {
  return error instanceof Error ? error.message : String(error)
}

/** The shipped critique routing, beside the critique skill's runner: the read-only proofs are matched against it. */
const DEFAULT_ROUTING_PATH = join(import.meta.dirname, '..', 'critique', 'routing.json')

export interface ModelGatewayRuntimeOptions extends Partial<ModelGatewayDependencies> {
  /** The shipped critique routing, which a test points at its own copy. */
  readonly routingPath?: string
}

/** Wires the real files, PATH, clock, ledger and environment; tests replace any of them. */
export function defaultModelGatewayRuntime(options: ModelGatewayRuntimeOptions = {}): ModelGatewayRuntime {
  const { routingPath = DEFAULT_ROUTING_PATH, ...overrides } = options
  const env = overrides.env ?? new ProcessEnvAdapter()
  const files = overrides.files ?? new NodeModelGatewayFiles()
  const git = overrides.git ?? new NodeModelGatewayGit()
  const process = overrides.process ?? new NodeModelGatewayProcess()
  return {
    commands: createModelGatewayCommands({
      clock: new SystemClockAdapter(),
      defaultsPath: join(import.meta.dirname, 'defaults.json'),
      env,
      files,
      git,
      ledger: new NodeModelGatewayLedger(env),
      process,
      proof: new NodeModelGatewayProof({ env, files, routingPath }),
      sources: modelSources({ env, files, process }),
      workflows: new NodeModelGatewayWorkflows(env),
      ...overrides,
    }),
    readFile: (path) => readFile(path, 'utf8'),
  }
}
