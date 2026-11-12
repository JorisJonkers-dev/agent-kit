import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  AGENT_KIT_REMOTE,
  claudeResult,
  codexStream,
  FakeEnv,
  FakeWorkProcess,
  fenced,
  finding,
  harness,
  LaunchLog,
  SPEC_CANARY,
  type Harness,
  type WorkCli,
  type WorkScript,
} from '../../../test/critique/harness.js'
import { NodeCritiqueFiles, WorkClaudeCriticClient, WorkCodexCriticClient, type CriticClient, type FanOutResult, type PrepareResult } from './index.js'
import { clientReadiness, clientSignIn, clientVersion } from './application/ports.js'

const NEDAP_REMOTE = { name: 'origin', url: 'git@github.com:nedap/widgets.git' }

const GRANT = {
  answer: 'yes',
  cap_usd: 5,
  client: 'codex',
  expires_at: '2026-10-31T12:00:00Z',
  id: 'g1',
  question: 'may I spend up to $5 of nedap money on the widgets spec?',
  workflow: 'widgets-spec',
  written_at: '2026-10-30T11:00:00Z',
}

const NOTHING = { findings: [], no_findings_reason: 'Compared every story with the glossary.' }

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const codexRun = (value: unknown): WorkScript => ({ stdout: codexStream(fenced(value)) })
const claudeRun = (value: unknown): WorkScript => ({ stdout: claudeResult(fenced(value)) })

async function nedap(options: Parameters<typeof harness>[0]): Promise<Harness> {
  const h = await harness({ grants: [GRANT], remotes: [NEDAP_REMOTE], ...options })
  open.push(h)
  return h
}

async function prepare(h: Harness, extra: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
    workflow: 'widgets-spec',
    ...extra,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

async function fanOut(h: Harness, run: PrepareResult): Promise<FanOutResult> {
  return h.commands.fanOut.run({ run_dir: run.run_dir })
}

describe('the Codex critic of a nedap repository', () => {
  it('is listed as ready for a Claude orchestrator under a live grant, and runs through codex exec, read-only in its packet', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [codexRun({ findings: [finding()] })] } } })

    const run = await prepare(h)
    expect(run.critics.map((critic) => critic.client)).toEqual(['codex-work'])
    const result = await fanOut(h, run)

    const [launch] = h.workCli.codex?.launches ?? []
    const packet = join(run.run_dir, 'critics', 'c1-codex-work', 'packet')
    expect(launch?.command).toBe('codex')
    expect(launch?.cwd).toBe(packet)
    expect(launch?.args).toEqual(expect.arrayContaining(['exec', '--json', '-m', 'gpt-5.5', '-s', 'read-only', '-C', packet]))
    expect(launch?.args).not.toContain('workspace-write')
    expect(launch?.stdin).toContain('# Tools')
    expect(result.critics).toMatchObject([{ client: 'codex-work', family: 'openai', finding_count: 1, status: 'ok', tokens: { cached: 40, input: 100, output: 20 } }])
    expect(result.findings.map((found) => found.id)).toEqual(['F1'])
    expect(result.critics[0]?.charged_usd).toBe(0)
  })

  it('records its run in the gateway ledger under the client name routing gives it', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [codexRun(NOTHING)] } } })

    await fanOut(h, await prepare(h))

    expect(await h.gatewayLedger()).toContain('"codex"')
  })

  it('reports a run that exits non-zero, with what the CLI said', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [{ exitCode: 2, stderr: 'boom\n', stdout: '' }] } } })

    const result = await fanOut(h, await prepare(h))

    expect(result.critics[0]).toMatchObject({ detail: 'codex exited 2: boom', failure: 'exit-2', status: 'failed' })
    expect(result.degraded).toBe(true)
  })

  it('reports a run the outer timer killed', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [{ stdout: '', timedOut: true }] } } })

    const result = await fanOut(h, await prepare(h))

    expect(result.critics[0]).toMatchObject({ failure: 'timeout', status: 'failed' })
    expect(result.critics[0]?.detail).toMatch(/codex did not return within \d+s/)
  })

  it('reports a run that printed nothing', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [{ stdout: '  \n' }] } } })

    expect((await fanOut(h, await prepare(h))).critics[0]).toMatchObject({ failure: 'empty-output', status: 'failed' })
  })

  it('reports a run codex itself marks failed', async () => {
    const stdout = `${JSON.stringify({ type: 'error' })}\n${JSON.stringify({ message: 'the model refused', type: 'turn.failed' })}`
    const h = await nedap({ workCli: { codex: { scripts: [{ stdout }] } } })

    const result = await fanOut(h, await prepare(h))

    expect(result.critics[0]).toMatchObject({ failure: 'infrastructure-failure', status: 'failed' })
    expect(result.critics[0]?.detail).toContain('the model refused')
  })

  it('reports a run that ends with no final message', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [{ stdout: JSON.stringify({ type: 'turn.completed', usage: {} }) }] } } })

    expect((await fanOut(h, await prepare(h))).critics[0]).toMatchObject({ failure: 'unparseable-output', status: 'failed' })
  })

  it('reports a final message without the findings block', async () => {
    const h = await nedap({ workCli: { codex: { scripts: [{ stdout: codexStream('I looked at it and it is fine.') }] } } })

    expect((await fanOut(h, await prepare(h))).critics[0]).toMatchObject({ failure: 'no-findings-block', status: 'failed' })
  })

  it('discards the findings of a run that wrote into its packet', async () => {
    const effect: WorkScript['effect'] = async ({ cwd }) => {
      await writeFile(join(cwd, 'artifact', 'spec.md'), 'changed')
    }
    const h = await nedap({ workCli: { codex: { scripts: [{ ...codexRun({ findings: [finding()] }), effect }] } } })

    const result = await fanOut(h, await prepare(h))

    expect(result.critics[0]).toMatchObject({ finding_count: 0, status: 'violated-readonly' })
    expect(result.findings).toEqual([])
  })
})

describe('the work Claude critic of a nedap repository', () => {
  it('is listed as ready for a Codex orchestrator and runs in plan mode with the read tools, in the work profile', async () => {
    const h = await nedap({
      env: { CLAUDE_CONFIG_DIR: '/somewhere/else' },
      workCli: { claude: { scripts: [claudeRun({ findings: [finding()] })] } },
    })

    const run = await prepare(h, { orchestrator_family: 'openai', workflow: undefined })
    expect(run.critics.map((critic) => critic.client)).toEqual(['claude-work'])
    const result = await fanOut(h, run)

    const [launch] = h.workCli.claude?.launches ?? []
    expect(launch?.command).toBe('claude')
    expect(launch?.cwd).toBe(join(run.run_dir, 'critics', 'c1-claude-work', 'packet'))
    expect(launch?.args).toEqual(
      expect.arrayContaining(['-p', '--model', 'claude-sonnet-5-5', '--output-format', 'json', '--permission-mode', 'plan', '--allowedTools', 'Read,Grep,Glob']),
    )
    expect(launch?.env).toEqual({ CLAUDE_CONFIG_DIR: join(h.home, '.claude') })
    expect(result.critics).toMatchObject([{ client: 'claude-work', family: 'anthropic', finding_count: 1, status: 'ok', tokens: { cached: 40, input: 110, output: 20 } }])
    expect(result.critics[0]?.cost_usd).toBe(0.05)
    expect(result.critics[0]?.charged_usd).toBe(0)
  })

  it('reports a run claude marks as an error', async () => {
    const h = await nedap({ workCli: { claude: { scripts: [{ stdout: claudeResult('Credit balance is too low', { is_error: true }) }] } } })

    const result = await fanOut(h, await prepare(h, { orchestrator_family: 'openai' }))

    expect(result.critics[0]).toMatchObject({ failure: 'infrastructure-failure', status: 'failed' })
    expect(result.critics[0]?.detail).toContain('claude reported a failed run')
  })

  it('reports an answer that is not claude output', async () => {
    const h = await nedap({ workCli: { claude: { scripts: [{ stdout: 'not json' }] } } })

    expect((await fanOut(h, await prepare(h, { orchestrator_family: 'openai' }))).critics[0]).toMatchObject({
      failure: 'unparseable-output',
      status: 'failed',
    })
  })
})

const detailOf = (dropped: PrepareResult['dropped'], client: string): string => dropped.find((critic) => critic.client === client)?.detail ?? ''

describe('readiness of the work critics', () => {
  const reasons = async (workCli: { codex?: WorkCli; claude?: WorkCli }) => {
    const h = await nedap({ workCli })
    return (await prepare(h, { orchestrator_family: 'mistral' })).dropped
  }

  it('names a missing codex CLI with setup\'s wording', async () => {
    expect(await reasons({ claude: {}, codex: { onPath: false } })).toContainEqual({
      client: 'codex-work',
      detail: 'codex: codex is not on PATH',
      family: 'openai',
      reason: 'unavailable',
    })
  })

  it('names a missing claude CLI', async () => {
    expect(await reasons({ claude: { onPath: false }, codex: {} })).toContainEqual(
      expect.objectContaining({ client: 'claude-work', detail: 'claude: claude is not on PATH' }),
    )
  })

  it('names a signed-out codex and how to sign in', async () => {
    expect(await reasons({ claude: {}, codex: { status: 'signed-out' } })).toContainEqual(
      expect.objectContaining({
        client: 'codex-work',
        detail: 'codex: not signed in; run codex login with the work account, then re-run setup (Not logged in)',
      }),
    )
  })

  it('names a signed-out work Claude profile and how to sign in', async () => {
    expect(await reasons({ claude: { status: 'signed-out' }, codex: {} })).toContainEqual(
      expect.objectContaining({
        client: 'claude-work',
        detail: 'claude: not signed in to the work profile; run claude and sign in with the work account, then re-run setup (Not logged in)',
      }),
    )
  })

  it('does not count a status command that cannot start or hangs as signed in', async () => {
    const hung = { exitCode: 0, stderr: '', stdout: '', timedOut: true }
    const dropped = await reasons({ claude: { status: hung }, codex: { status: 'spawn-error' } })

    expect(detailOf(dropped, 'codex-work')).toContain('spawn codex EACCES')
    expect(detailOf(dropped, 'claude-work')).toContain('no answer within 20s')
  })

  it('says exit code when a failing status command says nothing', async () => {
    const dropped = await reasons({ claude: {}, codex: { status: { exitCode: 3, stderr: '', stdout: '' } } })

    expect(detailOf(dropped, 'codex-work')).toContain('(exit 3)')
  })

  it('asks the claude status of the work profile directory, whatever the environment says', async () => {
    const h = await nedap({ env: { CLAUDE_CONFIG_DIR: '/elsewhere' }, workCli: { claude: {}, codex: {} } })

    await prepare(h, { orchestrator_family: 'mistral' })

    expect(h.workCli.claude?.statusCalls.map((call) => [call.args, call.env])).toEqual([[['auth', 'status'], { CLAUDE_CONFIG_DIR: join(h.home, '.claude') }]])
    expect(h.workCli.codex?.statusCalls.map((call) => call.args)).toEqual([['login', 'status']])
  })

  it('answers sign-in, readiness and version for each client', async () => {
    const h = await nedap({ workCli: { claude: { version: '2.1.0' }, codex: { version: 'codex-cli 0.9.0' } } })
    const routing = JSON.parse(await readFile(join(h.assetsRoot, 'routing.json'), 'utf8')) as { clients: Record<string, CriticClient> }
    const codex = routing.clients['codex-work']
    const claude = routing.clients['claude-work']
    if (codex === undefined || claude === undefined) throw new Error('routing names no work client')
    const files = new NodeCritiqueFiles()
    const env = new FakeEnv({ HOME: h.home, PATH: join(h.home, '..', 'bin') })
    const log = new LaunchLog()
    const codexClient = new WorkCodexCriticClient(new FakeWorkProcess('codex', h.repo, { version: 'codex-cli 0.9.0' }, log), files, env)
    const claudeClient = new WorkClaudeCriticClient(new FakeWorkProcess('claude', h.repo, { version: '2.1.0' }, log), files, env)
    const clients = { agy: undefined as never, claude: claudeClient, codex: codexClient, vibe: undefined as never }

    expect(await clientVersion(clients, codex)).toBe('codex-cli 0.9.0')
    expect(await clientVersion(clients, claude)).toBe('2.1.0')
    expect(await clientSignIn(clients, codex)).toEqual({ ready: true })
    expect(await clientReadiness(clients, claude)).toEqual({ ready: true })
  })

  it('reads no version from a CLI that is not installed', async () => {
    const files = new NodeCritiqueFiles()
    const env = new FakeEnv({ HOME: '/home/x', PATH: '' })
    const log = new LaunchLog()
    const codex = new WorkCodexCriticClient(new FakeWorkProcess('codex', '/r', { onPath: false }, log), files, env)

    expect(await codex.version({ binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' })).toBeNull()
    expect(await codex.signIn({ binary: '/nonexistent/codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' })).toEqual({
      detail: 'codex: /nonexistent/codex is not on PATH',
      ready: false,
    })
  })
})

describe('the grant check', () => {
  const routingWith = (codex: Record<string, unknown> | undefined) => ({
    clients: {
      ...(codex === undefined ? {} : { 'codex-work': { binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5', ...codex } }),
      'claude-work': { binary: 'claude', family: 'anthropic', kind: 'claude', model: 'claude-sonnet-5-5' },
    },
    stages: {
      spec: {
        base: [],
        distinct_families: true,
        work: [
          ...(codex === undefined ? [] : [{ client: 'codex-work', lenses: ['fidelity'], max_price_usd: 1, role: 'spec-redteam', time_limit_s: 60 }]),
          { client: 'claude-work', lenses: ['fidelity'], max_price_usd: 1, role: 'spec-redteam', time_limit_s: 60 },
        ],
      },
    },
    version: 1,
  })

  it("asks for the grant under the client routing's gateway entry names", async () => {
    const h = await nedap({ routing: routingWith({ gateway: { client: 'codex', model: 'gpt-5.5' } }), workCli: { codex: {} } })

    expect((await prepare(h)).critics.map((critic) => critic.client)).toEqual(['codex-work'])
  })

  it('asks under the kind when routing pins no gateway client, and names it when it is missing', async () => {
    const h = await nedap({ grants: [{ ...GRANT, client: 'another' }], routing: routingWith({}), workCli: { codex: {} } })

    expect((await prepare(h)).dropped[0]).toMatchObject({
      detail: 'no live grant covers workflow widgets-spec for codex: ask the owner, then write one with model-gateway grant write',
      reason: 'no-grant',
    })
  })

  it('has nothing to grant when the stage routes no Codex work client', async () => {
    const h = await nedap({ routing: routingWith(undefined), workCli: {} })

    expect((await prepare(h, { orchestrator_family: 'openai' })).critics.map((critic) => critic.client)).toEqual([])
  })
})

describe('the refute round of a nedap repository', () => {
  const MISTRAL = { orchestrator_family: 'mistral' }
  const accepted = [{ finding_id: 'F1', verdict: 'accepted' }]
  const refutation = finding({
    check: 'refute',
    claim: 'The spec states the window, so nothing contradicts the decision.',
    evidence: [
      { file: 'artifact/spec.md', kind: 'quote', value: SPEC_CANARY },
      { kind: 'input', value: 'A widget created at second three is frobnicated by second seven, as the story says.' },
    ],
  })

  it('refutes through the other work critic, read-only', async () => {
    const h = await nedap({
      workCli: {
        claude: { scripts: [claudeRun(NOTHING), claudeRun({ findings: [refutation] })] },
        codex: { scripts: [codexRun({ findings: [finding({ severity: 'critical' })] })] },
      },
    })
    const run = await prepare(h, MISTRAL)
    await fanOut(h, run)

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted })

    expect(result.results[0]).toMatchObject({ finding_id: 'F1', outcome: 'refute', refuter: { client: 'claude-work' } })
    const refuter = h.workCli.claude?.launches.at(-1)
    expect(refuter?.cwd).toBe(join(run.run_dir, 'refute', 'F1', 'packet'))
    expect(refuter?.args).toEqual(expect.arrayContaining(['--permission-mode', 'plan']))
    expect(h.workCli.codex?.launches).toHaveLength(1)
  })

  it('picks Codex when the work Claude critic raised the finding and the grant is still live', async () => {
    const h = await nedap({
      workCli: {
        claude: { scripts: [claudeRun({ findings: [finding({ severity: 'critical' })] })] },
        codex: { scripts: [codexRun(NOTHING), codexRun({ findings: [refutation] })] },
      },
    })
    const run = await prepare(h, MISTRAL)
    await fanOut(h, run)

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted })

    expect(result.results[0]).toMatchObject({ outcome: 'refute', refuter: { client: 'codex-work' } })
  })

  it('skips, naming each client, when the only other work critic is the orchestrator\'s family', async () => {
    const { h, run } = await reviewedCodexOnly()

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted })

    expect(result.results).toEqual([
      {
        detail: 'no routed client can refute it (codex-work: openai raised the finding; claude-work: anthropic is the orchestrator\'s family)',
        finding_id: 'F1',
        outcome: 'skipped',
        reason_code: 'no-eligible-client',
      },
    ])
  })

  it('skips Codex once its grant has lapsed since the run was prepared', async () => {
    const h = await nedap({
      workCli: {
        claude: { scripts: [claudeRun({ findings: [finding({ severity: 'critical' })] })] },
        codex: { scripts: [codexRun(NOTHING)] },
      },
    })
    const run = await prepare(h, MISTRAL)
    await fanOut(h, run)
    await h.writeText(join(h.home, '.config', 'agent-kit', 'model-gateway-grants.json'), JSON.stringify({ grants: [{ ...GRANT, workflow: 'another' }], version: 1 }))

    const result = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted })

    expect(result.results[0]).toMatchObject({ outcome: 'skipped' })
    expect(result.results[0]?.detail).toContain('codex-work: no live grant covers workflow widgets-spec for codex: ask the owner')
    expect(h.workCli.codex?.launches).toHaveLength(1)
  })

  it('skips Codex when no grant covered the run, and when the grants cannot be read', async () => {
    const h = await nedap({
      grants: [],
      workCli: { claude: { scripts: [claudeRun({ findings: [finding({ severity: 'critical' })] })] }, codex: {} },
    })
    const run = await prepare(h, { ...MISTRAL, workflow: undefined })
    await fanOut(h, run)

    const none = await h.commands.refute.run({ run_dir: run.run_dir, verdicts: accepted })
    expect(none.results[0]?.detail).toContain("codex-work: no live grant covered the run's workflow for codex when it was prepared")

    const covered = await nedap({ workCli: { claude: { scripts: [claudeRun({ findings: [finding({ severity: 'critical' })] })] }, codex: { scripts: [codexRun(NOTHING)] } } })
    const second = await prepare(covered, MISTRAL)
    await fanOut(covered, second)
    await covered.writeText(join(covered.home, '.config', 'agent-kit', 'model-gateway-grants.json'), 'not json')
    const unreadable = await covered.commands.refute.run({ run_dir: second.run_dir, verdicts: accepted })
    expect(unreadable.results[0]?.detail).toContain('codex-work: no live grant covers workflow widgets-spec for codex')
  })

  it('never offers a client outside the work roster to a nedap run', async () => {
    const { h, run } = await reviewedCodexOnly()

    const manifest = JSON.parse(await h.readText(run.manifest_path)) as { work?: unknown }

    expect(manifest.work).toEqual({ grant_workflow: 'widgets-spec' })
  })

  async function reviewedCodexOnly(): Promise<{ h: Harness; run: PrepareResult }> {
    const h = await nedap({ workCli: { claude: {}, codex: { scripts: [codexRun({ findings: [finding({ severity: 'critical' })] })] } } })
    const run = await prepare(h)
    await fanOut(h, run)
    return { h, run }
  }

  it('records a manifest with no grant workflow when none covered the run', async () => {
    const h = await nedap({ grants: [], workCli: { claude: {}, codex: {} } })

    const run = await prepare(h, { workflow: undefined })

    expect(JSON.parse(await h.readText(run.manifest_path))).toMatchObject({ work: {} })
    expect(await h.readText(run.manifest_path)).not.toContain('grant_workflow')
  })
})

describe('fan-out on a repository that is not nedap\'s', () => {
  it('refuses a run whose manifest names a work client', async () => {
    const h = await harness({ remotes: [AGENT_KIT_REMOTE], workCli: { claude: {}, codex: { scripts: [codexRun(NOTHING)] } } })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    const manifest = JSON.parse(await h.readText(run.manifest_path)) as { critics: { client: string; client_config: unknown }[] }
    manifest.critics = [
      { ...manifest.critics[0], client: 'codex-work', client_config: { binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' } },
    ]
    await h.writeText(run.manifest_path, JSON.stringify(manifest))

    await expect(fanOut(h, run)).rejects.toMatchObject({ code: 'invalid-run' })
    expect(h.workCli.codex?.launches).toEqual([])
  })
})

describe('the production wiring', () => {
  it('lists both work critics as ready on a machine whose CLIs are signed in, and fans out through them', async () => {
    const { cp, mkdtemp, rm } = await import('node:fs/promises')
    const { tmpdir } = await import('node:os')
    const { execFileSync } = await import('node:child_process')
    const { defaultCritiqueRuntime, runCritiqueCli } = await import('../../critique-cli/index.js')
    const { ASSETS_ROOT } = await import('../../../test/critique/harness.js')
    const base = await mkdtemp(join(tmpdir(), 'critique-work-'))
    try {
      const bin = join(base, 'bin')
      const home = join(base, 'home')
      const repo = join(base, 'repo')
      await mkdir(bin, { recursive: true })
      // Stub CLIs: they answer the status and version calls and print a canned review; nothing reaches a real service.
      const review = JSON.stringify({ is_error: false, result: fenced(NOTHING), type: 'result', usage: {} })
      await writeFile(join(bin, 'claude'), `#!/bin/sh\ncat >/dev/null\ncase "$1" in auth|--version) exit 0;; esac\nprintf '%s' '${review}'\n`, { mode: 0o755 })
      await writeFile(join(bin, 'codex'), '#!/bin/sh\ncat >/dev/null\nexit 0\n', { mode: 0o755 })
      await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
      await writeFile(join(home, '.config', 'agent-kit', 'model-gateway-grants.json'), JSON.stringify({ grants: [{ ...GRANT, expires_at: '2999-01-01T00:00:00Z' }], version: 1 }))
      await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
      await writeFile(join(repo, '.scratch', 'widgets', 'spec.md'), '# Widgets\n')
      await writeFile(join(repo, 'CONTEXT.md'), '# Words\n')
      await mkdir(join(repo, 'docs', 'agents'), { recursive: true })
      await writeFile(join(repo, 'docs', 'agents', 'issue-tracker.md'), '# Issue tracker: Local Markdown\n')
      execFileSync('git', ['init', '-q', repo])
      execFileSync('git', ['-C', repo, 'remote', 'add', 'origin', 'git@github.com:nedap/widgets.git'])
      // The runner starts a CLI from the process's own PATH, so the stubs are named by path in a private copy of routing.
      const assets = join(base, 'critique')
      await cp(ASSETS_ROOT, assets, { recursive: true })
      await cp(join(ASSETS_ROOT, '..', 'model-gateway', 'defaults.json'), join(base, 'model-gateway', 'defaults.json'))
      const routing = JSON.parse(await readFile(join(assets, 'routing.json'), 'utf8')) as { clients: Record<string, { binary: string }> }
      for (const name of ['claude', 'codex'] as const) {
        const client = routing.clients[`${name}-work`]
        if (client !== undefined) client.binary = join(bin, name)
      }
      await writeFile(join(assets, 'routing.json'), JSON.stringify(routing))
      const runtime = defaultCritiqueRuntime(assets, () => Promise.resolve(''), { HOME: home, PATH: bin })
      const call = async (command: string, input: unknown): Promise<Record<string, unknown>> =>
        JSON.parse((await runCritiqueCli([command, '--json', JSON.stringify(input)], runtime)).stdout) as Record<string, unknown>

      const prepared = await call('prepare', { orchestrator_family: 'mistral', repo, spec: '.scratch/widgets/spec.md', stage: 'spec', workflow: 'widgets-spec' })

      expect((prepared.critics as { client: string }[]).map((critic) => critic.client)).toEqual(['codex-work', 'claude-work'])
      expect(prepared.dropped).toEqual([])
      const fanned = await call('fan-out', { run_dir: prepared.run_dir })
      const claude = (fanned.critics as { client: string; status: string }[]).find((critic) => critic.client === 'claude-work')
      expect(claude?.status).toBe('ok')
    } finally {
      await rm(base, { force: true, recursive: true })
    }
  })
})
