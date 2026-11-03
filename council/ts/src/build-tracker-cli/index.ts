import {
  BuildTrackerError,
  createBuildTrackerCommands,
  GhCliBuildTrackerIssues,
  GitBuildTrackerRepositories,
  NodeBuildTrackerFiles,
  NodeBuildTrackerProcess,
  type BuildTrackerCommands,
  type BuildTrackerDependencies,
} from '../contexts/build-tracker/index.js'

export interface BuildTrackerCliResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

export interface BuildTrackerCliRuntime {
  readonly commands: BuildTrackerCommands
}

export const BUILD_TRACKER_USAGE = `usage: build-tracker init --spec SPEC [--milestone <name>=<ticket> ...]
       build-tracker sync --spec SPEC
       build-tracker path --spec SPEC
       build-tracker refresh --spec SPEC [--landed <ticket> ...]
       build-tracker render --state FILE --out FILE
       build-tracker set --state FILE <ticket>=<status>[:note] ...

SPEC is a GitHub issue (#12, owner/repo#12 or its URL) whose sub-issues are
the tickets, or a local markdown spec file whose tickets are the numbered
files (01-name.md) in the issues/ folder beside it.

init writes a new state for the spec from its tickets: each open ticket todo,
each closed one merged, with its blockers. Each --milestone, in order, names a
milestone and the ticket that closes it. init refuses a spec that already has
a state; sync brings that state up to date instead. sync adds new tickets,
updates titles and blockers, marks newly closed tickets merged, and keeps
every status, note and milestone set on open tickets. path prints where the
spec's state file and page live, under $XDG_STATE_HOME (else
~/.local/state)/agent-kit/build-tracker.

refresh brings the spec's page up to date in one step, as /flow runs it: init
(with no milestones) when the spec has no state, else sync; then each
--landed ticket (12, #12, or a local ticket's 01) turns merged, keeping its
note; then the page is rendered at the path path prints.

render writes the build tracker page for the state file. set changes the
named tickets' statuses in the state file; a note after the status replaces
the ticket's note, an empty one removes it, and none keeps it. The statuses
are todo, running, review, merged and owner.

Each command prints one JSON object. An error prints "status": "error" with
its "code" and exits 1, and leaves every file as it was. A missing or
unreadable state file is an error that names init. Bad arguments print this
usage and exit 2.
`

/** A required flag is given once; a repeatable one any number of times, none included. */
type FlagKind = 'required' | 'repeatable'

type Flags = Readonly<Record<string, readonly string[]>>

interface CommandSpec {
  /** The flags the command takes. */
  readonly flags: Readonly<Record<string, FlagKind>>
  /** Whether the command takes one or more positional arguments. */
  readonly positionals: boolean
  run(commands: BuildTrackerCommands, flags: Flags, positionals: readonly string[]): Promise<unknown>
}

const COMMANDS: Readonly<Record<string, CommandSpec>> = {
  init: {
    flags: { milestone: 'repeatable', spec: 'required' },
    positionals: false,
    run: (commands, flags) => commands.init.run({ milestones: flags.milestone ?? [], spec: flag(flags, 'spec') }),
  },
  path: {
    flags: { spec: 'required' },
    positionals: false,
    run: (commands, flags) => commands.path.run({ spec: flag(flags, 'spec') }),
  },
  refresh: {
    flags: { landed: 'repeatable', spec: 'required' },
    positionals: false,
    run: (commands, flags) => commands.refresh.run({ landed: flags.landed ?? [], spec: flag(flags, 'spec') }),
  },
  render: {
    flags: { out: 'required', state: 'required' },
    positionals: false,
    run: (commands, flags) => commands.render.run({ out: flag(flags, 'out'), state: flag(flags, 'state') }),
  },
  set: {
    flags: { state: 'required' },
    positionals: true,
    run: (commands, flags, positionals) => commands.set.run({ assignments: positionals, state: flag(flags, 'state') }),
  },
  sync: {
    flags: { spec: 'required' },
    positionals: false,
    run: (commands, flags) => commands.sync.run({ spec: flag(flags, 'spec') }),
  },
}

/** The build tracker's command line: one command, one JSON object out. */
export async function runBuildTrackerCli(argv: readonly string[], runtime: BuildTrackerCliRuntime): Promise<BuildTrackerCliResult> {
  const [name, ...args] = argv
  if (name === '--help' || name === '-h') return { exitCode: 0, stderr: '', stdout: BUILD_TRACKER_USAGE }
  const spec = name !== undefined && Object.hasOwn(COMMANDS, name) ? COMMANDS[name] : undefined
  const parsed = spec === undefined ? undefined : parseArguments(args, spec)
  if (spec === undefined || parsed === undefined) return { exitCode: 2, stderr: BUILD_TRACKER_USAGE, stdout: '' }
  try {
    const result = await spec.run(runtime.commands, parsed.flags, parsed.positionals)
    return { exitCode: 0, stderr: '', stdout: `${JSON.stringify(result, null, 2)}\n` }
  } catch (error) {
    const message = error instanceof Error ? error.message : String(error)
    const code = error instanceof BuildTrackerError ? error.code : 'unexpected'
    return {
      exitCode: 1,
      stderr: `build-tracker ${String(name)}: ${message}\n`,
      stdout: `${JSON.stringify({ code, message, status: 'error' })}\n`,
    }
  }
}

/** `--flag value` pairs and positional arguments, or undefined when they do not fit the command. */
function parseArguments(
  args: readonly string[],
  spec: CommandSpec,
): { flags: Flags; positionals: readonly string[] } | undefined {
  const flags: Record<string, string[]> = {}
  const positionals: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? ''
    if (!arg.startsWith('--')) {
      positionals.push(arg)
      continue
    }
    const name = arg.slice(2)
    const kind = Object.hasOwn(spec.flags, name) ? spec.flags[name] : undefined
    const value = args[index + 1]
    if (kind === undefined || (kind === 'required' && Object.hasOwn(flags, name)) || value === undefined) return undefined
    flags[name] = [...(flags[name] ?? []), value]
    index += 1
  }
  const flagsOk = Object.entries(spec.flags).every(([name, kind]) => kind !== 'required' || Object.hasOwn(flags, name))
  const positionalsOk = spec.positionals ? positionals.length > 0 : positionals.length === 0
  return flagsOk && positionalsOk ? { flags, positionals } : undefined
}

function flag(flags: Flags, name: string): string {
  return flags[name]?.[0] ?? ''
}

/** Wires the real files, clock, gh, git, working directory and environment; tests replace any of them. */
export function defaultBuildTrackerRuntime(overrides: Partial<BuildTrackerDependencies> = {}): BuildTrackerCliRuntime {
  const process = new NodeBuildTrackerProcess()
  return {
    commands: createBuildTrackerCommands({
      clock: { now: () => new Date() },
      cwd: globalThis.process.cwd(),
      env: globalThis.process.env,
      files: new NodeBuildTrackerFiles(),
      issues: new GhCliBuildTrackerIssues(process),
      repositories: new GitBuildTrackerRepositories(process),
      ...overrides,
    }),
  }
}
