import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  fenced,
  finding,
  harness,
  isolatedExport,
  vibeStdout,
  type Harness,
  type VibeScript,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

async function preparedRun(scripts: VibeScript[]): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness({ scripts })
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  const run = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })
  if (run.status !== 'ready') throw new Error('not ready')
  return { h, run }
}

async function fanOut(script: VibeScript): Promise<{ h: Harness; run: PrepareResult; result: FanOutResult }> {
  const { h, run } = await preparedRun([script])
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

function asset(path: string): Promise<string> {
  return readFile(join(ASSETS_ROOT, path), 'utf8')
}

const ok = (text: string, exportJson: unknown = isolatedExport()): VibeScript => ({
  exitCode: 0,
  exportJson,
  stdout: vibeStdout(text),
})

describe('fan-out launches the critic', () => {
  it('runs plain vibe in the packet, read-only, on the personal account', async () => {
    const { h, run } = await fanOut(ok(fenced({ findings: [finding()] })))

    const [command] = h.process.commands
    expect(command?.cwd).toBe(run.packet_dir)
    expect(command?.command).toBe('env')
    const args = command?.args ?? []
    expect(args.slice(0, 5)).toEqual(['-u', 'MISTRAL_API_KEY', '-u', 'VIBE_HOME', 'vibe'])
    expect(args).toEqual(expect.arrayContaining(['--output', 'json', '--trust', '--agent', 'critic-mistral-large']))
    const tools = args.flatMap((arg, i) => (args[i - 1] === '--enabled-tools' ? [arg] : []))
    expect(tools).toEqual(['read_file'])
    expect(args[args.indexOf('--max-price') + 1]).toBe('0.5')
    expect(args[args.indexOf('--time-limit') + 1]).toBe('600')
    expect(args[args.indexOf('--output-dir') + 1]).toBe(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'export'))
    expect(command?.env).toEqual({
      VIBE_ENABLE_MCP: 'false',
      VIBE_ENABLE_SUBAGENTS: 'false',
      VIBE_INCLUDE_PROJECT_CONTEXT: 'false',
      VIBE_OUTSIDE_WORKDIR: 'deny',
    })
    expect(args.some((arg) => arg.includes('vibe-study') || arg.includes('vibe-private'))).toBe(false)
  })

  it('gives the critic only the preamble, role, lenses, output contract and packet file list', async () => {
    const { h } = await fanOut(ok(fenced({ findings: [finding()] })))

    const args = h.process.commands[0]?.args ?? []
    const promptPath = args[args.indexOf('--prompt-file') + 1] ?? ''
    expect(promptPath.startsWith(join(h.repo, '.git', 'critique'))).toBe(true)
    let rest = await readFile(promptPath, 'utf8')
    for (const block of [
      await asset('prompts/preamble.md'),
      await asset('roles/spec-redteam.md'),
      await asset('lenses/fidelity.md'),
      await asset('lenses/testability.md'),
      await asset('prompts/output-contract.md'),
      await asset('schemas/finding.schema.json'),
    ]) {
      const at = rest.indexOf(block)
      expect(at).toBeGreaterThanOrEqual(0)
      rest = rest.slice(0, at) + rest.slice(at + block.length)
    }
    expect(rest.replace(/\s+/g, ' ').trim()).toBe(
      '--- --- --- ```json ``` --- # Files Your working directory holds these files and no others. ' +
        'Read each one with read_file, using the path exactly as written here: ' +
        '- `artifact/spec.md`: the artifact under review - `references/CONTEXT.md`: reference material',
    )
  })

  it('keeps the prompt out of the packet', async () => {
    const { run } = await fanOut(ok(fenced({ findings: [finding()] })))

    const manifest = JSON.parse(await readFile(run.manifest_path, 'utf8')) as { packet_files: unknown[] }
    expect(manifest.packet_files).toHaveLength(2)
  })
})

describe('fan-out reads the critic output', () => {
  it('turns a valid fenced JSON block into findings', async () => {
    const { result } = await fanOut(ok(fenced({ findings: [finding(), finding({ id: 'F2', severity: 'minor' })] })))

    expect(result.critics).toEqual([
      expect.objectContaining({
        client: 'vibe-mistral-large',
        cost_usd: 0.0123,
        critic_id: 'c1-vibe-mistral-large',
        duration_ms: 1500,
        exit_code: 0,
        family: 'mistral',
        finding_count: 2,
        model: 'mistral-large-4-0-3',
        status: 'ok',
      }),
    ])
    expect(result.findings.map((f) => [f.id, f.critic_finding_id, f.severity])).toEqual([
      ['F1', 'F1', 'major'],
      ['F2', 'F2', 'minor'],
    ])
    expect(result.findings[0]).toMatchObject({
      category: 'contradiction',
      location: 'User story 3',
      raised_by: ['c1-vibe-mistral-large'],
    })
    expect(result.degraded).toBe(false)
  })

  it('reads the last fenced block of the final assistant message', async () => {
    const text = `Example:\n\n\`\`\`json\n{"findings": []}\n\`\`\`\n\nReal answer:\n${fenced({ findings: [finding()] })}`

    const { result } = await fanOut(ok(text))

    expect(result.critics[0]?.finding_count).toBe(1)
  })

  it('accepts an empty list that says what was checked', async () => {
    const { result } = await fanOut(ok(fenced({ findings: [], no_findings_reason: 'Compared every story with CONTEXT.md.' })))

    expect(result.critics[0]).toMatchObject({
      finding_count: 0,
      no_findings_reason: 'Compared every story with CONTEXT.md.',
      status: 'ok',
    })
  })

  it.each<[string, VibeScript, string]>([
    ['empty stdout with exit code 0', { exitCode: 0, exportJson: isolatedExport(), stdout: '' }, 'empty-output'],
    ['stdout that is not JSON', { exitCode: 0, exportJson: isolatedExport(), stdout: 'Traceback' }, 'unparseable-output'],
    ['a journal with no assistant message', { exitCode: 0, exportJson: isolatedExport(), stdout: '[]' }, 'unparseable-output'],
    ['a reply with no fenced block', ok('Looks fine to me.'), 'no-findings-block'],
    ['a fenced block that is not JSON', ok('```json\n{nope\n```'), 'unparseable-findings'],
    ['an empty list without no_findings_reason', ok(fenced({ findings: [] })), 'missing-no-findings-reason'],
    ['a finding with an unknown severity', ok(fenced({ findings: [finding({ severity: 'huge' })] })), 'invalid-findings'],
    ['a finding with malformed evidence', ok(fenced({ findings: [finding({ evidence: [{ kind: 'quote' }] })] })), 'invalid-findings'],
    ['a block without a findings list', ok(fenced({ result: 'fine' })), 'invalid-findings'],
    ['a run with no export file', { exitCode: 0, stdout: vibeStdout(fenced({ findings: [finding()] })) }, 'missing-export'],
  ])('records %s as a failed critic', async (_name, script, failure) => {
    const { result } = await fanOut(script)

    expect(result.critics[0]).toMatchObject({ failure, finding_count: 0, status: 'failed' })
    expect(result.findings).toEqual([])
    expect(result.degraded).toBe(true)
  })

  it.each<[number, string, string]>([
    [1, 'config_error', 'config-error'],
    [2, 'infrastructure_failure', 'infrastructure-failure'],
    [3, 'deadline', 'stopped-early'],
    [3, 'price_limit', 'stopped-early'],
    [137, 'killed', 'exit-137'],
  ])('maps Vibe exit code %i (%s) to a failed critic even with parseable findings', async (exitCode, outcome, failure) => {
    const { result } = await fanOut({
      exitCode,
      exportJson: { ...isolatedExport(), exit_code: exitCode, outcome },
      stderr: 'Error: stopped',
      stdout: vibeStdout(fenced({ findings: [finding()] })),
    })

    expect(result.critics[0]).toMatchObject({ exit_code: exitCode, failure, status: 'failed' })
    expect(result.critics[0]?.detail).toContain(outcome)
    expect(result.findings).toEqual([])
  })

  it('describes a non-zero exit from stderr when there is no export file', async () => {
    const { result } = await fanOut({ exitCode: 1, stderr: "error: invalid value 'abc'", stdout: '' })

    expect(result.critics[0]).toMatchObject({ cost_usd: null, failure: 'config-error', status: 'failed' })
    expect(result.critics[0]?.detail).toContain("invalid value 'abc'")
  })

  it.each<[string, Record<string, unknown>, string]>([
    ['MCP on', { enable_mcp: true }, 'enable_mcp'],
    ['a tool beyond read_file', { enabled_tools: ['read_file', 'bash'] }, 'enabled_tools'],
    ['the instructions pointer included', { include_project_context: true }, 'include_project_context'],
    ['access outside the packet', { outside_workdir: 'ask' }, 'outside_workdir'],
  ])('discards the findings of a critic whose effective config has %s', async (_name, override, setting) => {
    const { result } = await fanOut(ok(fenced({ findings: [finding()] }), isolatedExport(override)))

    expect(result.critics[0]).toMatchObject({ failure: 'unsafe-config', finding_count: 0, status: 'failed' })
    expect(result.critics[0]?.detail).toContain(setting)
    expect(result.findings).toEqual([])
  })

  it('treats an unreadable export file as missing', async () => {
    const { h, run } = await preparedRun([{ exitCode: 0, stdout: vibeStdout(fenced({ findings: [finding()] })) }])
    await h.writeText(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'export', 'export.json'), '{oops')

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result.critics[0]).toMatchObject({ failure: 'missing-export', status: 'failed' })
  })

  it('keeps the raw output and the merged findings in the run directory', async () => {
    const { result, run } = await fanOut(ok(fenced({ findings: [finding()] })))

    const critic = result.critics[0]
    expect(await readFile(critic?.stdout_path ?? '', 'utf8')).toContain('"assistant"')
    expect(JSON.parse(await readFile(result.findings_path, 'utf8'))).toEqual(result)
    expect(result.findings_path).toBe(join(run.run_dir, 'findings.json'))
  })

  it('records the hash of every prompt and lens file', async () => {
    const { result } = await fanOut(ok(fenced({ findings: [finding()] })))

    const hashes = result.critics[0]
    if (hashes === undefined) throw new Error('no critic')
    expect(Object.keys(hashes.prompt_hashes).sort()).toEqual(['contract', 'preamble', 'role', 'schema'])
    expect(Object.keys(hashes.lens_hashes)).toEqual(['fidelity', 'testability'])
    const promptHashes: Readonly<Record<string, string>> = { ...hashes.prompt_hashes }
    const all = [...Object.values(promptHashes), ...Object.values(hashes.lens_hashes)]
    for (const hash of all) expect(hash).toMatch(/^[0-9a-f]{64}$/)
    expect(new Set(all).size).toBe(6)
  })

  it('launches nothing and reports the run degraded when it has no critics', async () => {
    const h = await harness()
    open.push(h)
    await h.commands.allow.run({ repo: h.repo, families: ['google'] })
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')

    const result = await h.commands.fanOut.run({ run_dir: run.run_dir })

    expect(result).toMatchObject({ critics: [], degraded: true, findings: [] })
    expect(h.process.commands).toEqual([])
  })

  it('rejects a manifest that is not a run manifest', async () => {
    const { h, run } = await preparedRun([])
    await h.writeText(run.manifest_path, JSON.stringify({ critics: 'all of them', version: 1 }))

    await expect(h.commands.fanOut.run({ run_dir: run.run_dir })).rejects.toThrow(/manifest\.json: .*must be/)
    expect(h.process.commands).toEqual([])
  })

  it('rejects a run directory without a manifest', async () => {
    const { h, run } = await preparedRun([])

    await expect(h.commands.fanOut.run({ run_dir: join(run.run_dir, 'nope') })).rejects.toThrow()
  })
})
