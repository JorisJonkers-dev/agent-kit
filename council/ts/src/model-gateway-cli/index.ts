import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { ProcessEnvAdapter, SystemClockAdapter } from '../adapters/clock/index.js'
import {
  clientStatuses,
  createModelGatewayCommands,
  ModelGatewayError,
  modelSources,
  NodeModelGatewayFiles,
  NodeModelGatewayGit,
  NodeModelGatewayGrants,
  NodeModelGatewayKept,
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
       model-gateway usage --json '{"days": 7, "workflow": "spec-293"}'
       model-gateway run --json '{"workflow": "spec-293", "client": "vibe",
                                 "model": "zai-glm-5-3", "mode": "read-only",
                                 "prompt": "read this", "repo": "/path/to/repo"}'
       model-gateway run --json '{"workflow": "spec-293", "client": "claude",
                                 "model": "claude-sonnet-5-5", "mode": "write",
                                 "prompt": "add the flag", "repo": "/path/to/repo",
                                 "base": "main", "check": "npm test"}'
       model-gateway grant write --json '{"workflow": "spec-293", "client": "codex",
                                 "cap_usd": 5, "question": "may I spend $5 of nedap money?",
                                 "answer": "yes, up to five"}'
       model-gateway grant list --json '{}'
       model-gateway workflow start --json '{"name": "spec-293"}'
       model-gateway workflow end --json '{"name": "spec-293"}'
       model-gateway workflow list --json '{}'
       model-gateway worktrees clean --json '{"workflow": "spec-293"}'
       model-gateway <command> --input FILE

Each command reads one JSON object, from --json or from FILE, and prints one
JSON object. models lists every client model this machine can use now for the
repository the caller works in, with its client, model id, family, cost class,
difficulty tier, capability notes, where the list came from (live, config or
declared) and what the data policy lets a delegation to it carry. It names
each client it knows but cannot use, and says in "policy" why the data policy
listed what it did. Without "repo", or outside any repository, only inline
content may be sent.

usage reports every client it knows: whether it is installed, whether its own
status command says it is signed in, whether its last run in the availability
window was rate-limited or refused for its credentials, what it has spent over
the last "days" days (7 by default) by day and by workflow, what its own plan
has left ("unknown" where the client reports none), each window of the plan
limit the machine file sets for it, and what its live reservations hold
("reserved_usd" and "reserved_runs"). A client at or above
"plan_warning_ratio" of a window carries a warning; one at 100% of a window is
"unable_to_start". "workflow" narrows the usage to one workflow. It settles
every reservation whose run left a result first, at what that result says the
run cost, and every one whose expiry has passed at its full estimate.

run hands one read-only task to one client model and prints a short result:
its status, run id, workflow, client model, family, mode, tokens, cost, the
path of its full output and a summary of at most "summary_max_chars"
characters. It takes "workflow", "client" (vibe, agy, claude, opencode or
hermes), "model", "mode", "prompt", and optionally "repo", "carries" (code and
history), "time_limit_s" (900 by default), "max_price_usd" (Vibe only),
"provider" (which a hermes run must name, and which this machine must allow),
"escalate" and "max_attempts".
The workflow's life, the data policy and the client model's read-only proof are
checked before anything is launched; a model without a passing proof (every one
but a proven Vibe or Antigravity model) works in a throwaway worktree of the
repository, removed after the run. An OpenCode model is named provider/model
and its provider must be in clients.opencode.providers (opencode-go by
default), never opencode. A client near its plan limit adds "warnings" to the
result.

A delegation that fails does not come straight back: "escalate" (true by
default; false turns it all off) and "max_attempts" (3 by default) govern it. A
failed result (a timeout, output that cannot be parsed, a failed check, a
non-zero exit) is retried once on the same client model with the failure in the
prompt; a second failure climbs to the next stronger permitted subsidised
model; and a client that cannot start is
swapped for another of its cost class at the same tier or above. Each attempt
is a run of its own, and the result carries "attempts" (run_id, client, model,
status, reason), "skipped" and, when nothing is left to try, "escalation":
"exhausted". When no client model of the cost class can start, the result is
{"status": "unavailable"} with the "reasons" per candidate, and exit 0.

A metered-personal run (Hermes today, and any client the machine file gives
that cost class) is held to the caps in "metered_personal_caps": its estimate,
its prompt's characters over 3.5 rounded up at the model's input price plus its
tier's "output_allowance_tokens" at the model's output price, is reserved
against the per-workflow and per-day caps before it starts, and a run that does
not fit either is refused with "cap-exceeded". A client model the catalogue
prices nowhere or gives no tier cannot be estimated, and a metered run on it is
refused with "no-estimate". The estimate is the run's hard spend limit where
the client takes one (Vibe's --max-price). When the run ends, its actual spend
replaces the reservation; a reservation whose run left a result is settled from
that result at once, and one whose run left nothing lapses at the run's time
limit plus 60 seconds and counts its full estimate as spent. A run that wrote
its own ledger line is counted from that line, so its reservation holds nothing
more.
Each attempt of a delegation reserves and settles its own estimate, and only a
metered-personal attempt reserves at all.

A run with "mode": "write" changes a repository: it needs "repo", "base" (a ref)
and "check" (a command, run with sh -c in the worktree). The gateway creates a
worktree off the base on a new branch gateway/<run id> under the repository's
git common directory, runs the delegate there with write permission, stages
what it wrote, and then runs the check, so what the check writes is not part of
the staged diff. It never commits, merges or pushes. The result adds
"worktree", "branch", "staged_files" and "check" (command, exit_code and
output_path); a check that fails is "failed" with reason "check-failed", and a
run that times out leaves its worktree unstaged. Every writing worktree is kept
until worktrees clean removes it, with its branch, for the workflow that ended
("workflow") or for all of them ("all": true).

workflow start, workflow end and workflow list keep a workflow's life: start
and end take "name", and list takes nothing. A workflow nobody ends within 24
hours counts as ended, and a run may not join one that ended.

An error prints "status": "error" with its "code" and exits 1.
`

/** The subcommands the gateway has; #294 ships models, #296 adds run, #297 the workflow commands, #298 the grant commands, #300 worktrees clean, #302 usage. */
const COMMANDS = {
  'grant list': 'grantList',
  'grant write': 'grantWrite',
  models: 'models',
  run: 'run',
  usage: 'usage',
  'workflow end': 'workflowEnd',
  'workflow list': 'workflowList',
  'workflow start': 'workflowStart',
  'worktrees clean': 'worktreesClean',
} as const satisfies Record<string, keyof ModelGatewayCommands>

/** The subcommands, as an error names them. */
const COMMAND_NAMES =
  'models, run, usage, grant write, grant list, workflow start, workflow end, workflow list and worktrees clean'

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
      grants: new NodeModelGatewayGrants(env, git),
      kept: new NodeModelGatewayKept(env),
      ledger: new NodeModelGatewayLedger(env),
      process,
      proof: new NodeModelGatewayProof({ env, files, routingPath }),
      sources: modelSources({ env, files, process }),
      statuses: clientStatuses({ process }),
      workflows: new NodeModelGatewayWorkflows(env),
      ...overrides,
    }),
    readFile: (path) => readFile(path, 'utf8'),
  }
}
