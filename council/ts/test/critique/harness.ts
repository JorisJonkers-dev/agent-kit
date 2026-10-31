import { cp, mkdir, mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import type { ClockPort, EnvPort, GitRemote, ProcessCommand, ProcessPort, ProcessResult } from '../../src/ports/index.js'
import {
  createCritiqueCommands,
  NodeCritiqueFiles,
  VibeCriticClient,
  type CritiqueCommands,
  type CritiqueGitPort,
} from '../../src/contexts/critique/index.js'

export const ASSETS_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../../../skills/critique')

export const SPEC_CANARY = 'Every widget must be frobnicated within the canary window of seven seconds.'
export const DECISION_CANARY = 'Decided: widgets are frobnicated by the nightly job only.'

export class FakeEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.values[name]
    if (value === undefined) throw new Error(`missing ${name}`)
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

export class FixedClock implements ClockPort {
  private ticks = 0

  constructor(private readonly iso: string) {}

  now(): Date {
    return new Date(this.iso)
  }

  monotonicMs(): number {
    this.ticks += 1500
    return this.ticks
  }

  async sleep(): Promise<void> {
    await Promise.resolve()
  }
}

export class FakeGit implements CritiqueGitPort {
  constructor(
    private readonly repoRoot: string,
    public remoteList: readonly GitRemote[],
  ) {}

  async root(): Promise<string> {
    return Promise.resolve(this.repoRoot)
  }

  async commonDir(): Promise<string> {
    return Promise.resolve(join(this.repoRoot, '.git'))
  }

  async remotes(): Promise<readonly GitRemote[]> {
    return Promise.resolve(this.remoteList)
  }
}

export interface VibeScript {
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode: number
  readonly exportJson?: unknown
}

/** Plays canned Vibe runs: writes export.json into --output-dir and returns stdout and exit code. */
export class FakeVibeProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly scripts: VibeScript[]) {}

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    const script = this.scripts.shift()
    if (script === undefined) throw new Error('unexpected process launch')
    const outputDirIndex = command.args.indexOf('--output-dir')
    const outputDir = command.args[outputDirIndex + 1]
    if (script.exportJson !== undefined && outputDir !== undefined) {
      await mkdir(outputDir, { recursive: true })
      await writeFile(join(outputDir, 'export.json'), JSON.stringify(script.exportJson))
    }
    return { exitCode: script.exitCode, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

export function vibeStdout(finalText: string): string {
  return JSON.stringify([
    { type: 'message', role: 'user', content: [{ type: 'text', text: 'prompt' }] },
    { type: 'effect', detail: { toolName: 'read_file' }, state: { status: 'completed' } },
    { type: 'message', role: 'assistant', content: [{ type: 'text', text: 'Reading.' }] },
    {
      type: 'message',
      role: 'assistant',
      generationStatus: 'completed',
      content: [{ type: 'text', text: finalText }],
      outcome: { type: 'committed' },
    },
  ])
}

export function isolatedExport(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    outcome: 'finished',
    stop_reason: null,
    error: null,
    exit_code: 0,
    cost_usd: 0.0123,
    steps: 3,
    config: {
      active_model: 'mistral-large-4-0-3',
      enable_mcp: false,
      enabled_tools: ['read_file'],
      include_project_context: false,
      outside_workdir: 'deny',
      ...overrides,
    },
  }
}

export function fenced(value: unknown): string {
  return `I read both files.\n\n\`\`\`json\n${JSON.stringify(value, null, 2)}\n\`\`\`\n`
}

export function finding(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'F1',
    severity: 'major',
    category: 'contradiction',
    location: 'User story 3',
    claim: 'The spec contradicts the recorded decision about when widgets are frobnicated.',
    evidence: [
      { kind: 'quote', file: 'artifact/spec.md', value: SPEC_CANARY },
      { kind: 'quote', file: 'references/CONTEXT.md', value: DECISION_CANARY },
    ],
    suggestion: 'Say that only the nightly job frobnicates widgets.',
    ...overrides,
  }
}

export interface Harness {
  readonly home: string
  readonly repo: string
  readonly assetsRoot: string
  readonly git: FakeGit
  readonly process: FakeVibeProcess
  readonly commands: CritiqueCommands
  readonly cleanup: () => Promise<void>
  readonly readText: (path: string) => Promise<string>
  readonly writeText: (path: string, content: string) => Promise<void>
  readonly removeFile: (path: string) => Promise<void>
}

export const AGENT_KIT_REMOTE: GitRemote = {
  name: 'origin',
  url: 'git@github.com:JorisJonkers-dev/agent-kit.git',
}

export async function harness(
  options: {
    readonly remotes?: readonly GitRemote[]
    readonly scripts?: VibeScript[]
    /** Replaces the shipped routing.json in a private copy of the assets. */
    readonly routing?: unknown
  } = {},
): Promise<Harness> {
  const base = await mkdtemp(join(tmpdir(), 'critique-'))
  let assetsRoot = ASSETS_ROOT
  if (options.routing !== undefined) {
    assetsRoot = join(base, 'assets')
    await cp(ASSETS_ROOT, assetsRoot, { recursive: true })
    await writeFile(join(assetsRoot, 'routing.json'), JSON.stringify(options.routing))
  }
  const home = join(base, 'home')
  const repo = join(base, 'repo')
  await mkdir(join(repo, '.git'), { recursive: true })
  await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
  await writeFile(
    join(repo, '.scratch', 'widgets', 'spec.md'),
    `# Widgets\n\n## User Stories\n\n1. As the owner, I want widgets.\n\n3. ${SPEC_CANARY}\n`,
  )
  await writeFile(join(repo, 'CONTEXT.md'), `# Glossary\n\n${DECISION_CANARY}\n`)
  const git = new FakeGit(repo, options.remotes ?? [AGENT_KIT_REMOTE])
  const process = new FakeVibeProcess(options.scripts ?? [])
  const files = new NodeCritiqueFiles()
  const commands = createCritiqueCommands({
    assetsRoot,
    clients: { vibe: new VibeCriticClient(process, files) },
    clock: new FixedClock('2026-10-30T12:00:00.000Z'),
    env: new FakeEnv({ HOME: home }),
    files,
    git,
  })
  return {
    assetsRoot,
    cleanup: () => rm(base, { force: true, recursive: true }),
    commands,
    git,
    home,
    process,
    readText: (path) => readFile(path, 'utf8'),
    removeFile: (path) => rm(path),
    repo,
    writeText: async (path, content) => {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content)
    },
  }
}
