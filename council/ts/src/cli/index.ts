import {
  CouncilApp,
  type ConfigPaths,
  type FleetInput,
  type PlanInput,
  type FanoutInput,
} from '../app/index.js'
import type { CouncilConfig } from '../domain/config/index.js'
import type { TriageInput } from '../domain/triage/index.js'

export type CliCommand =
  | 'amend'
  | 'config'
  | 'context'
  | 'design'
  | 'fanout'
  | 'fleet'
  | 'grill'
  | 'inject'
  | 'plan'
  | 'review-pack'
  | 'self-test'
  | 'split'
  | 'status'
  | 'supervise'
  | 'survey'
  | 'sync-bmad'
  | 'sync-skills'
  | 'tail'
  | 'triage'

export interface CliResult {
  readonly exitCode: number
  readonly stderr: string
  readonly stdout: string
}

export interface CliRuntime {
  readonly app?: CouncilApp
  readonly configPaths?: ConfigPaths
}

export interface CommandSpec {
  readonly help: string
  readonly name: CliCommand
}

const COMMANDS: readonly CommandSpec[] = [
  { help: 'validate amendment payloads and append them to a run', name: 'amend' },
  { help: 'show or change council.toml while preserving unrelated lines', name: 'config' },
  { help: 'assemble context packs for downstream stages', name: 'context' },
  { help: 'run design stages D0-D5', name: 'design' },
  { help: 'execute a planned task DAG', name: 'fanout' },
  { help: 'round-robin a task DAG across an explicit agent pool', name: 'fleet' },
  { help: 'adversarially question task readiness', name: 'grill' },
  { help: 'inject operator guidance into a supervised worker', name: 'inject' },
  { help: 'compose planning stages without auto-executing workers', name: 'plan' },
  { help: 'assemble checkpoint review packs', name: 'review-pack' },
  { help: 'run TS parity checks for Python self-test cases', name: 'self-test' },
  { help: 'extract a subtree into a destination repo', name: 'split' },
  { help: 'summarize a run directory', name: 'status' },
  { help: 'supervise a worker process with watchdog controls', name: 'supervise' },
  { help: 'survey repository context', name: 'survey' },
  { help: 'synchronize BMAD assets', name: 'sync-bmad' },
  { help: 'synchronize council skills', name: 'sync-skills' },
  { help: 'tail one task log', name: 'tail' },
  { help: 'classify request routing before planning', name: 'triage' },
]

export function commandRegistry(): readonly CommandSpec[] {
  return COMMANDS
}

export async function runCli(argv: readonly string[], runtime: CliRuntime = {}): Promise<CliResult> {
  const app = runtime.app ?? new CouncilApp()
  const [command, ...rest] = argv
  try {
    if (command === undefined || command === '--help' || command === '-h') {
      return ok(renderHelp())
    }
    if (command === '--self-test' || command === 'self-test') {
      return ok(JSON.stringify(await appSelfTest(), null, 2))
    }
    if (!isCommand(command)) {
      return fail(`unknown command: ${command}`)
    }

    switch (command) {
      case 'plan':
        return okJson(await app.plan(parsePlan(rest)))
      case 'fanout':
        return okJson(await app.fanout(parseFanout(rest)))
      case 'fleet':
        return okJson(await app.fleet(parseFleet(rest)))
      case 'config':
        return okJson(
          await app.config({
            ...parseConfig(rest),
            paths: runtime.configPaths ?? defaultConfigPaths(),
          }),
        )
      case 'status':
        return okJson(await app.status({ runDir: requireFlag(parseFlags(rest), 'run') }))
      case 'review-pack':
        return okJson(await app.readReviewPack(parseReviewPack(rest)))
      case 'triage':
        return okJson((await app.plan({ triage: parseTriage(rest) })).triage ?? {})
      case 'design':
      case 'amend':
      case 'context':
      case 'grill':
      case 'inject':
      case 'split':
      case 'supervise':
      case 'survey':
      case 'sync-bmad':
      case 'sync-skills':
      case 'tail':
        return okJson({ command, compiled: true })
    }
    return fail(`unknown command: ${command}`)
  } catch (error) {
    return fail(error instanceof Error ? error.message : String(error))
  }
}

async function appSelfTest(): Promise<unknown> {
  const { pythonSelfTestGolden } = await import('../app/index.js')
  return pythonSelfTestGolden()
}

function parsePlan(argv: readonly string[]): PlanInput {
  const flags = parseFlags(argv)
  return {
    config: configOverrides(flags),
    design: flags.has('design'),
    ...(flags.get('brief') ? { brief: requireFlag(flags, 'brief') } : {}),
    ...(flags.get('run') ? { runDir: requireFlag(flags, 'run') } : {}),
    ...(flags.has('triage') ? { triage: parseTriageFlag(requireFlag(flags, 'triage')) } : {}),
  }
}

function parseFanout(argv: readonly string[]): FanoutInput {
  const flags = parseFlags(argv)
  return {
    dryRun: flags.has('dry-run'),
    github: flags.has('github'),
    runDir: requireFlag(flags, 'run'),
  }
}

function parseFleet(argv: readonly string[]): FleetInput {
  const flags = parseFlags(argv)
  return {
    agents: requireFlag(flags, 'agents'),
    dryRun: flags.has('dry-run'),
    github: flags.has('github'),
    tasksPath: requireFlag(flags, 'tasks'),
  }
}

function parseConfig(argv: readonly string[]): {
  readonly action: 'show' | 'get' | 'set' | 'unset' | 'path'
  readonly key?: string
  readonly project?: boolean
  readonly value?: string
} {
  const positional = argv.filter((arg) => !arg.startsWith('--'))
  const flags = parseFlags(argv)
  const action = positional[0]
  if (!isConfigAction(action)) throw new Error('config requires action show|get|set|unset|path')
  return {
    action,
    ...(positional[1] ? { key: positional[1] } : {}),
    project: flags.has('project'),
    ...(positional[2] ? { value: positional[2] } : {}),
  }
}

function parseReviewPack(argv: readonly string[]): { readonly gate: '1' | 'design' | '2'; readonly runDir: string } {
  const flags = parseFlags(argv)
  const gate = requireFlag(flags, 'gate')
  if (gate !== '1' && gate !== 'design' && gate !== '2') throw new Error('--gate must be 1, design, or 2')
  return { gate, runDir: requireFlag(flags, 'run') }
}

function parseTriage(argv: readonly string[]): TriageInput {
  return parseTriageFlag(requireFlag(parseFlags(argv), 'input'))
}

function parseTriageFlag(raw: string): TriageInput {
  const parsed = JSON.parse(raw) as TriageInput
  return parsed
}

function parseFlags(argv: readonly string[]): Map<string, string> {
  const flags = new Map<string, string>()
  for (let index = 0; index < argv.length; index += 1) {
    const arg = argv[index]
    if (!arg?.startsWith('--')) continue
    const key = arg.slice(2)
    const next = argv[index + 1]
    if (next === undefined || next.startsWith('--')) {
      flags.set(key, 'true')
    } else {
      flags.set(key, next)
      index += 1
    }
  }
  return flags
}

function configOverrides(flags: ReadonlyMap<string, string>): CouncilConfig {
  const config: Record<string, unknown> = {}
  const intensity = flags.get('intensity')
  const rounds = flags.get('rounds')
  const plannerA = flags.get('planner-a')
  const plannerB = flags.get('planner-b')
  const consolidator = flags.get('consolidator')
  const codexEffort = flags.get('codex-effort')
  if (intensity !== undefined) config.intensity = intensity
  if (rounds !== undefined) config.rounds = Number.parseInt(rounds, 10)
  if (plannerA !== undefined) config.planner_a = plannerA
  if (plannerB !== undefined) config.planner_b = plannerB
  if (consolidator !== undefined) config.consolidator = consolidator
  if (codexEffort !== undefined) config.codex_effort = codexEffort
  return config
}

function requireFlag(flags: ReadonlyMap<string, string>, name: string): string {
  const value = flags.get(name)
  if (value === undefined || value === 'true') throw new Error(`--${name} is required`)
  return value
}

function isCommand(value: string): value is CliCommand {
  return COMMANDS.some((command) => command.name === value)
}

function isConfigAction(value: string | undefined): value is 'show' | 'get' | 'set' | 'unset' | 'path' {
  return value === 'show' || value === 'get' || value === 'set' || value === 'unset' || value === 'path'
}

function renderHelp(): string {
  return COMMANDS.map((command) => `${command.name}\t${command.help}`).join('\n')
}

function defaultConfigPaths(): ConfigPaths {
  return {
    project: '.council.toml',
    user: `${process.env.HOME ?? '.'}/.config/council/council.toml`,
  }
}

function ok(stdout: string): CliResult {
  return { exitCode: 0, stderr: '', stdout: `${stdout.trimEnd()}\n` }
}

function okJson(value: unknown): CliResult {
  return ok(JSON.stringify(value, null, 2))
}

function fail(stderr: string): CliResult {
  return { exitCode: 2, stderr: `${stderr.trimEnd()}\n`, stdout: '' }
}

/* c8 ignore start -- process entry bootstrap; not unit-testable */
if (import.meta.url === `file://${process.argv[1] ?? ''}`) {
  void runCli(process.argv.slice(2)).then((result) => {
    if (result.stdout) process.stdout.write(result.stdout)
    if (result.stderr) process.stderr.write(result.stderr)
    process.exitCode = result.exitCode
  })
}
/* c8 ignore stop */
