import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { ProcessEnvAdapter } from '../adapters/clock/index.js'
import {
  createModelGatewayCommands,
  ModelGatewayError,
  NodeModelGatewayFiles,
  NodeModelGatewayGit,
  NodeModelGatewayProcess,
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
       model-gateway models --input FILE

Each command reads one JSON object, from --json or from FILE, and prints one
JSON object. models lists every client model this machine can use now for the
repository the caller works in, with its client, model id, family, cost class,
difficulty tier, capability notes, where the list came from (live, config or
declared) and what the data policy lets a delegation to it carry. It names
each client it knows but cannot use, and says in "policy" why the data policy
listed what it did. Without "repo", or outside any repository, only inline
content may be sent. An error prints "status": "error" with its "code" and
exits 1.
`

/** The subcommands the gateway has; #294 ships models, later tickets add the rest. */
const COMMANDS = {
  models: 'models',
} as const satisfies Record<string, keyof ModelGatewayCommands>

/** The gateway's command line: one subcommand, one JSON object in, one JSON object out. */
export async function runModelGatewayCli(argv: readonly string[], runtime: ModelGatewayRuntime): Promise<ModelGatewayCliResult> {
  const [name, ...args] = argv
  if (name === '--help' || name === '-h') return { exitCode: 0, stderr: '', stdout: MODEL_GATEWAY_USAGE }
  const method = name !== undefined && Object.hasOwn(COMMANDS, name) ? COMMANDS[name as keyof typeof COMMANDS] : undefined
  if (method === undefined) {
    return failure('unknown-command', `unknown subcommand ${name === undefined ? '(none)' : `"${name}"`}; the gateway has models`)
  }
  const input = parseArguments(args)
  if (input === undefined) return failure('invalid-input', `${method} takes one of --json '<object>' or --input <file>`, name)
  try {
    const text = 'json' in input ? input.json : await readInput(runtime, input.input)
    const result = await runtime.commands[method].run(parseInput(text))
    return { exitCode: 0, stderr: '', stdout: `${JSON.stringify(result, null, 2)}\n` }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    return failure(error instanceof ModelGatewayError ? error.code : 'unexpected', message, name)
  }
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

/** Wires the real files, PATH and environment; tests replace any of them. */
export function defaultModelGatewayRuntime(overrides: Partial<ModelGatewayDependencies> = {}): ModelGatewayRuntime {
  return {
    commands: createModelGatewayCommands({
      defaultsPath: join(import.meta.dirname, 'defaults.json'),
      env: new ProcessEnvAdapter(),
      files: new NodeModelGatewayFiles(),
      git: new NodeModelGatewayGit(),
      process: new NodeModelGatewayProcess(),
      ...overrides,
    }),
    readFile: (path) => readFile(path, 'utf8'),
  }
}
