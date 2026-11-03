import {
  BuildTrackerError,
  createBuildTrackerCommands,
  NodeBuildTrackerFiles,
  type BuildTrackerClockPort,
  type BuildTrackerCommands,
} from '../contexts/build-tracker/index.js'

export interface BuildTrackerCliResult {
  readonly exitCode: number
  readonly stdout: string
  readonly stderr: string
}

export interface BuildTrackerCliRuntime {
  readonly commands: BuildTrackerCommands
}

export const BUILD_TRACKER_USAGE = `usage: build-tracker render --state FILE --out FILE
       build-tracker set --state FILE <ticket>=<status>[:note] ...

render writes the build tracker page for the state file. set changes the
named tickets' statuses in the state file; a note after the status replaces
the ticket's note, an empty one removes it, and none keeps it. The statuses
are todo, running, review, merged and owner.

Each command prints one JSON object. An error prints "status": "error" with
its "code" and exits 1, and leaves every file as it was. A missing or
unreadable state file is an error that names init. Bad arguments print this
usage and exit 2.
`

interface CommandSpec {
  /** The flags the command takes; every one is required. */
  readonly flags: readonly string[]
  /** Whether the command takes one or more positional arguments. */
  readonly positionals: boolean
  run(commands: BuildTrackerCommands, flags: Readonly<Record<string, string>>, positionals: readonly string[]): Promise<unknown>
}

const COMMANDS: Readonly<Record<string, CommandSpec>> = {
  render: {
    flags: ['state', 'out'],
    positionals: false,
    run: (commands, flags) => commands.render.run({ out: flag(flags, 'out'), state: flag(flags, 'state') }),
  },
  set: {
    flags: ['state'],
    positionals: true,
    run: (commands, flags, positionals) => commands.set.run({ assignments: positionals, state: flag(flags, 'state') }),
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
): { flags: Readonly<Record<string, string>>; positionals: readonly string[] } | undefined {
  const flags: Record<string, string> = {}
  const positionals: string[] = []
  for (let index = 0; index < args.length; index += 1) {
    const arg = args[index] ?? ''
    if (!arg.startsWith('--')) {
      positionals.push(arg)
      continue
    }
    const name = arg.slice(2)
    const value = args[index + 1]
    if (!spec.flags.includes(name) || Object.hasOwn(flags, name) || value === undefined) return undefined
    flags[name] = value
    index += 1
  }
  const flagsOk = spec.flags.every((name) => Object.hasOwn(flags, name))
  const positionalsOk = spec.positionals ? positionals.length > 0 : positionals.length === 0
  return flagsOk && positionalsOk ? { flags, positionals } : undefined
}

function flag(flags: Readonly<Record<string, string>>, name: string): string {
  return flags[name] ?? ''
}

/** Wires the real files and clock. */
export function defaultBuildTrackerRuntime(clock: BuildTrackerClockPort = { now: () => new Date() }): BuildTrackerCliRuntime {
  return { commands: createBuildTrackerCommands({ clock, files: new NodeBuildTrackerFiles() }) }
}
