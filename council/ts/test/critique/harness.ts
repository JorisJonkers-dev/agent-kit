import { cp, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'
import { fileURLToPath } from 'node:url'

import type {
  ClockPort,
  EnvPort,
  GhIssue,
  GhIssueLocator,
  GhIssueRequest,
  GhLinkedIssue,
  GitPorcelainEntry,
  GitRemote,
  ProcessCommand,
  ProcessPort,
  ProcessResult,
} from '../../src/ports/index.js'
import {
  AgyCriticClient,
  createCritiqueCommands,
  NodeCritiqueFiles,
  VibeCriticClient,
  type CritiqueCommands,
  type CritiqueGhPort,
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

/**
 * A repository without git: `porcelainStatus` compares the files on disk,
 * outside `.git/`, with those present at the last `commit()`, the way git
 * compares the working tree with HEAD.
 */
export class FakeGit implements CritiqueGitPort {
  private committed = new Map<string, string>()

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

  async commit(): Promise<void> {
    this.committed = await this.workingTree()
  }

  async porcelainStatus(): Promise<readonly GitPorcelainEntry[]> {
    const tree = await this.workingTree()
    const entries: GitPorcelainEntry[] = []
    for (const [path, text] of tree) {
      const committed = this.committed.get(path)
      if (committed === undefined) entries.push({ code: '??', path })
      else if (committed !== text) entries.push({ code: ' M', path })
    }
    for (const path of this.committed.keys()) if (!tree.has(path)) entries.push({ code: ' D', path })
    return entries.sort((a, b) => a.path.localeCompare(b.path))
  }

  private async workingTree(): Promise<Map<string, string>> {
    const tree = new Map<string, string>()
    for (const entry of await readdir(this.repoRoot, { recursive: true, withFileTypes: true })) {
      const path = relative(this.repoRoot, join(entry.parentPath, entry.name))
      if (entry.isDirectory() || path === '.git' || path.startsWith('.git/')) continue
      tree.set(path, await readFile(join(this.repoRoot, path), 'utf8'))
    }
    return tree
  }
}

/**
 * How the fake's issues link to each other, keyed by `<repo>#<number>` like
 * the issues: each spec's sub-issues, by number in the spec's repository,
 * and each issue's native blocked-by links, by `<repo>#<number>`. A string in
 * place of a list is the stderr of a gh failure.
 */
export interface FakeGhLinks {
  readonly subIssues?: Readonly<Record<string, readonly number[] | string>>
  readonly blockedBy?: Readonly<Record<string, readonly string[] | string>>
}

/**
 * Serves GitHub issues by `<repo>#<number>`, where `repo` is what the runner
 * passes as `--repo`. A string in place of an issue is the stderr of a gh
 * failure. An issue it does not hold fails the way gh does.
 */
export class FakeGh implements CritiqueGhPort {
  readonly requests: GhIssueRequest[] = []
  /** Every sub-issue and blocked-by read, as `<kind> <repo>#<number>`. */
  readonly linkRequests: string[] = []

  private readonly issues: Record<string, GhIssue | string>

  constructor(
    issues: Readonly<Record<string, GhIssue | string>>,
    private readonly links: FakeGhLinks = {},
  ) {
    this.issues = { ...issues }
  }

  /** Serves `issue` as `<repo>#<number>` from now on. */
  serve(key: string, issue: GhIssue | string): void {
    this.issues[key] = issue
  }

  async viewIssue(request: GhIssueRequest): Promise<GhIssue> {
    this.requests.push(request)
    const issue = this.issue(request.repo, request.number)
    return Promise.resolve(request.comments ? issue : { ...issue, comments: [] })
  }

  async subIssues(request: GhIssueLocator): Promise<readonly GhIssue[]> {
    const numbers = this.linked('sub-issues', this.links.subIssues, request)
    return Promise.resolve(numbers.map((number) => ({ ...this.issue(request.repo, number), comments: [] })))
  }

  async blockedBy(request: GhIssueLocator): Promise<readonly GhLinkedIssue[]> {
    const references = this.linked('blocked-by', this.links.blockedBy, request)
    return Promise.resolve(
      references.map((reference) => {
        const [repo = '', number = ''] = reference.split('#')
        const [host, ...path] = repo.split('/')
        return { number: Number(number), url: `https://${host ?? ''}/${path.join('/')}/issues/${number}` }
      }),
    )
  }

  private issue(repo: string, number: number): GhIssue {
    const issue = this.issues[`${repo}#${String(number)}`]
    if (issue === undefined) {
      throw new Error(
        `gh issue view ${String(number)} failed: GraphQL: Could not resolve to an issue or pull request with the number of ${String(number)}. (repository.issue)`,
      )
    }
    if (typeof issue === 'string') throw new Error(issue)
    return issue
  }

  private linked<T>(
    kind: string,
    table: Readonly<Record<string, readonly T[] | string>> | undefined,
    request: GhIssueLocator,
  ): readonly T[] {
    const key = `${request.repo}#${String(request.number)}`
    this.linkRequests.push(`${kind} ${key}`)
    const entry = table?.[key] ?? []
    if (typeof entry === 'string') throw new Error(entry)
    return entry
  }
}

/** A ticket issue: a sub-issue of a spec, with its acceptance criteria. */
export function ticketIssue(number: number, title: string, body: string): GhIssue {
  return {
    body,
    comments: [],
    labels: ['type:feature'],
    number,
    title,
    url: `https://github.com/JorisJonkers-dev/agent-kit/issues/${String(number)}`,
  }
}

/** A spec issue in agent-kit's tracker, as gh reports it. */
export function specIssue(overrides: Partial<GhIssue> = {}): GhIssue {
  return {
    body: `## User Stories\r\n\r\n1. As the owner, I want widgets.\r\n\r\n3. ${SPEC_CANARY}\r\n`,
    comments: [{ author: 'joris', body: 'Amended: the window is seven seconds.', createdAt: '2026-10-30T09:00:00Z' }],
    labels: ['type:feature'],
    number: 167,
    title: 'Widgets',
    url: 'https://github.com/JorisJonkers-dev/agent-kit/issues/167',
    ...overrides,
  }
}

/** The tracker headings Pocock's setup writes into `docs/agents/issue-tracker.md`. */
export const TRACKER_DOCS = {
  github: '# Issue tracker: GitHub\n\nIssues and specs for this repo live as GitHub issues.\n',
  gitlab: '# Issue tracker: GitLab\n\nIssues and specs for this repo live as GitLab issues.\n',
  'local-markdown': '# Issue tracker: Local Markdown\n\nIssues and specs for this repo live as markdown files in `.scratch/`.\n',
} as const

export interface VibeScript {
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode: number
  readonly exportJson?: unknown
  /** Plays this run only for the critic whose `--agent` this is; otherwise for the next launch. */
  readonly agent?: string
  /** What the critic does while it runs: `cwd` is its working directory, `repo` the repository. */
  readonly effect?: (where: { readonly cwd: string; readonly repo: string }) => Promise<void>
  /** Do not return until this many critics have started, so a test can see critics overlap. */
  readonly waitForStarts?: number
}

/** Every critic launch and return, in the order they happened, across all critic clients. */
export class LaunchLog {
  readonly events: string[] = []

  /**
   * Records a critic's start, waits until `waitForStarts` critics have
   * started (for at most two seconds, so critics run one after another make
   * the test fail rather than hang), then records its end.
   */
  async play(name: string, waitForStarts = 0): Promise<void> {
    this.events.push(`start ${name}`)
    const deadline = Date.now() + 2000
    while (this.events.filter((event) => event.startsWith('start')).length < waitForStarts && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
    this.events.push(`end ${name}`)
  }
}

/** Plays canned Vibe runs: writes export.json into --output-dir and returns stdout and exit code. */
export class FakeVibeProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(
    private readonly repo: string,
    private readonly scripts: VibeScript[],
    private readonly log: LaunchLog = new LaunchLog(),
  ) {}

  /** Adds runs to play, for a test that needs the prepared run's paths to write them. */
  queue(...scripts: VibeScript[]): void {
    this.scripts.push(...scripts)
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    const agent = command.args[command.args.indexOf('--agent') + 1]
    const named = this.scripts.findIndex((script) => script.agent === agent)
    const index = named === -1 ? this.scripts.findIndex((script) => script.agent === undefined) : named
    const [script] = index === -1 ? [] : this.scripts.splice(index, 1)
    if (script === undefined) throw new Error('unexpected process launch')
    await this.log.play('vibe', script.waitForStarts)
    const outputDirIndex = command.args.indexOf('--output-dir')
    const outputDir = command.args[outputDirIndex + 1]
    if (script.exportJson !== undefined && outputDir !== undefined) {
      await mkdir(outputDir, { recursive: true })
      await writeFile(join(outputDir, 'export.json'), JSON.stringify(script.exportJson))
    }
    await script.effect?.({ cwd: command.cwd ?? '', repo: this.repo })
    return { exitCode: script.exitCode, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

export interface AgyScript {
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode?: number
  /** Do not return until this many critics have started. */
  readonly waitForStarts?: number
  /** What the critic does while it runs: `cwd` is its working directory, `repo` the repository. */
  readonly effect?: (where: { readonly cwd: string; readonly repo: string }) => Promise<void>
}

/**
 * Plays canned agy runs, and answers the Keychain lookup the Antigravity
 * readiness check makes with `security`.
 */
export class FakeAgyProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(
    private readonly repo: string,
    private readonly scripts: AgyScript[],
    private readonly signedIn: boolean | 'no-security-tool',
    private readonly log: LaunchLog = new LaunchLog(),
  ) {}

  /** The critic launches only, without the readiness checks. */
  get launches(): readonly ProcessCommand[] {
    return this.commands.filter((command) => command.command !== 'security')
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (command.command === 'security') {
      if (this.signedIn === 'no-security-tool') throw new Error('spawn security ENOENT')
      return this.signedIn
        ? { exitCode: 0, stderr: '', stdout: 'keychain: "/Users/me/Library/Keychains/login.keychain-db"\n' }
        : { exitCode: 44, stderr: 'security: SecKeychainSearchCopyNext: The specified item could not be found in the keychain.\n', stdout: '' }
    }
    const script = this.scripts.shift()
    if (script === undefined) throw new Error('unexpected agy launch')
    await this.log.play('agy', script.waitForStarts)
    await script.effect?.({ cwd: command.cwd ?? '', repo: this.repo })
    return { exitCode: script.exitCode ?? 0, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

/** The `result` event agy prints last, for a run that returned the given structured output. */
export function agyResult(structured: unknown, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    conversation_id: 'c0ffee00-0000-4000-8000-000000000000',
    status: 'SUCCESS',
    response: 'I read both files and report what I found.\n',
    duration_seconds: 21.5,
    num_turns: 2,
    usage: { input_tokens: 9370, output_tokens: 812, thinking_tokens: 300, cache_read_tokens: 0, total_tokens: 10482 },
    ...(structured === undefined ? {} : { structured_output: structured }),
    ...overrides,
  }
}

/** agy's `stream-json` output: the init event, one tool step per named tool, then the result event. */
export function agyStream(result: Record<string, unknown>, tools: readonly string[] = ['read_file', 'read_file']): string {
  const id = 'c0ffee00-0000-4000-8000-000000000000'
  const events: unknown[] = [
    { event: 'init', conversation_id: id, init: { model: 'gemini-3.8-flash-high', cwd: '/packet', tools: ['read_file'], permission_mode: 'request-review' } },
    { event: 'step_update', step_update: { conversation_id: id, step_index: 0, state: 'DONE', step_type: 'user_input' } },
    ...tools.flatMap((tool, i) => [
      { event: 'step_update', step_update: { conversation_id: id, step_index: i + 1, state: 'ACTIVE', step_type: 'tool', tool_name: tool, tool_info: { name: tool, parameters: {} } } },
      { event: 'step_update', step_update: { conversation_id: id, step_index: i + 1, state: 'DONE', step_type: 'tool', tool_name: tool, tool_info: { name: tool, parameters: {} } } },
    ]),
    { event: 'step_update', step_update: { conversation_id: id, step_index: tools.length + 1, state: 'DONE', step_type: 'agent_response', text_delta: 'Done.' } },
    { event: 'result', result },
  ]
  return `${events.map((event) => JSON.stringify(event)).join('\n')}\n`
}

/** An agy run that returned the given findings as structured output. */
export function agyFindings(value: unknown, tools?: readonly string[]): AgyScript {
  return { stdout: agyStream(agyResult(value), tools) }
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

/** Two Vibe critics of different families, both in the spec stage's base roster. */
export function twoCriticRouting(): unknown {
  return {
    clients: {
      'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm-5' },
      'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0-3' },
    },
    stages: {
      spec: {
        base: [
          { client: 'vibe-mistral-large', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
          { client: 'vibe-glm', lenses: ['testability'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
        ],
        distinct_families: true,
      },
    },
    version: 1,
  }
}

export interface Harness {
  readonly home: string
  readonly repo: string
  readonly assetsRoot: string
  readonly git: FakeGit
  readonly gh: FakeGh
  readonly process: FakeVibeProcess
  readonly agy: FakeAgyProcess
  readonly launches: LaunchLog
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
    readonly agyScripts?: AgyScript[]
    /** Replaces the shipped routing.json in a private copy of the assets. */
    readonly routing?: unknown
    /**
     * This machine's routing override in `~/.config/agent-kit/critique-routing.json`:
     * a value is written as JSON, a string as it is.
     */
    readonly routingOverride?: unknown
    /** The platform the runner reports; Antigravity is set up on macOS only. */
    readonly platform?: NodeJS.Platform
    /** Whether `agy` is on PATH. */
    readonly agyOnPath?: boolean
    /** Whether the Keychain holds the `Antigravity CLI` item, or whether there is no `security` tool to ask. */
    readonly agySignedIn?: boolean | 'no-security-tool'
    /**
     * The issue-tracker doc Pocock's setup wrote, or `none` for a repository
     * without one. Local markdown unless named, so the `.scratch/` spec is the
     * tracker's.
     */
    readonly tracker?: keyof typeof TRACKER_DOCS | 'none'
    /** The issues gh serves, by `<repo>#<number>`. */
    readonly issues?: Readonly<Record<string, GhIssue | string>>
    /** How those issues link: sub-issues and native blocked-by links. */
    readonly links?: FakeGhLinks
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
  if (options.routingOverride !== undefined) {
    await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
    const override = options.routingOverride
    await writeFile(
      join(home, '.config', 'agent-kit', 'critique-routing.json'),
      typeof override === 'string' ? override : JSON.stringify(override),
    )
  }
  const repo = join(base, 'repo')
  await mkdir(join(repo, '.git'), { recursive: true })
  await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
  await writeFile(
    join(repo, '.scratch', 'widgets', 'spec.md'),
    `# Widgets\n\n## User Stories\n\n1. As the owner, I want widgets.\n\n3. ${SPEC_CANARY}\n`,
  )
  await writeFile(join(repo, 'CONTEXT.md'), `# Glossary\n\n${DECISION_CANARY}\n`)
  const bin = join(base, 'bin')
  await mkdir(bin, { recursive: true })
  if (options.agyOnPath ?? true) await writeFile(join(bin, 'agy'), '#!/bin/sh\n')
  const tracker = options.tracker ?? 'local-markdown'
  if (tracker !== 'none') {
    await mkdir(join(repo, 'docs', 'agents'), { recursive: true })
    await writeFile(join(repo, 'docs', 'agents', 'issue-tracker.md'), TRACKER_DOCS[tracker])
  }
  const git = new FakeGit(repo, options.remotes ?? [AGENT_KIT_REMOTE])
  await git.commit()
  const gh = new FakeGh(options.issues ?? {}, options.links)
  const launches = new LaunchLog()
  const process = new FakeVibeProcess(repo, options.scripts ?? [], launches)
  const agy = new FakeAgyProcess(repo, options.agyScripts ?? [], options.agySignedIn ?? true, launches)
  const files = new NodeCritiqueFiles()
  const env = new FakeEnv({ HOME: home, PATH: [join(base, 'nothing-here'), bin].join(':') })
  const commands = createCritiqueCommands({
    assetsRoot,
    clients: {
      agy: new AgyCriticClient(agy, files, env, options.platform ?? 'darwin'),
      vibe: new VibeCriticClient(process, files),
    },
    clock: new FixedClock('2026-10-30T12:00:00.000Z'),
    env,
    files,
    gh,
    git,
  })
  return {
    agy,
    assetsRoot,
    cleanup: () => rm(base, { force: true, recursive: true }),
    commands,
    gh,
    git,
    home,
    launches,
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
