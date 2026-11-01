import { readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  agyFindings,
  agyResult,
  agyStream,
  ASSETS_ROOT,
  finding,
  harness,
  type AgyScript,
  type Harness,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const SIGN_IN = 'antigravity: needs sign-in: run agy and sign in with your Google account'

type HarnessOptions = NonNullable<Parameters<typeof harness>[0]>

/** A repository that allows only Google, so the shipped spec roster is the Gemini critic alone. */
async function geminiOnly(options: HarnessOptions = {}): Promise<{ h: Harness; run: PrepareResult }> {
  const h = await harness(options)
  open.push(h)
  await h.commands.allow.run({ families: ['google'], repo: h.repo })
  const run = await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })
  if (run.status !== 'ready') throw new Error('not ready')
  return { h, run }
}

async function geminiReview(script: AgyScript): Promise<{ h: Harness; run: PrepareResult; result: FanOutResult }> {
  const { h, run } = await geminiOnly({ agyScripts: [script] })
  const result = await h.commands.fanOut.run({ run_dir: run.run_dir })
  return { h, result, run }
}

function argAfter(args: readonly string[], flag: string): string | undefined {
  const at = args.indexOf(flag)
  return at === -1 ? undefined : args[at + 1]
}

describe('the Antigravity critic', () => {
  it('runs agy in print mode with JSON output and the finding schema, sandboxed, with slash commands off and no plan mode', async () => {
    const { h, run } = await geminiReview(agyFindings({ findings: [finding()] }))

    const [command] = h.agy.launches
    expect(command?.command).toBe('agy')
    expect(command?.cwd).toBe(join(run.run_dir, 'critics', 'c1-agy-gemini-flash-high', 'packet'))
    const args = command?.args ?? []
    const prompt = await readFile(join(run.run_dir, 'critics', 'c1-agy-gemini-flash-high', 'prompt.md'), 'utf8')
    expect(argAfter(args, '-p')).toBe(prompt)
    expect(argAfter(args, '--output-format')).toBe('stream-json')
    expect(argAfter(args, '--model')).toBe('gemini-3.8-flash-high')
    expect(argAfter(args, '--json-schema')).toBe(join(h.assetsRoot, 'schemas', 'finding.schema.json'))
    expect(argAfter(args, '--print-timeout')).toBe('600s')
    expect(args).toEqual(expect.arrayContaining(['--disable-slash-commands', '--sandbox']))
    expect(args).not.toContain('--mode')
    expect(args).not.toContain('--dangerously-skip-permissions')
  })

  it('denies MCP, web, browser, command and write actions for the session only', async () => {
    const { h } = await geminiReview(agyFindings({ findings: [finding()] }))

    const settings = JSON.parse(argAfter(h.agy.launches[0]?.args ?? [], '--settings') ?? '{}') as {
      permissions?: { deny?: string[] }
    }
    expect(settings.permissions?.deny).toEqual(
      expect.arrayContaining(['mcp(*)', 'read_url(*)', 'execute_url(*)', 'command(*)', 'write_file(*)']),
    )
  })

  it('starts a fresh session every time', async () => {
    const { h } = await geminiReview(agyFindings({ findings: [finding()] }))

    const args = h.agy.launches[0]?.args ?? []
    for (const flag of ['-c', '--continue', '--conversation', '-i', '--prompt-interactive', '--project']) {
      expect(args).not.toContain(flag)
    }
  })

  it('reads findings from structured_output only', async () => {
    const prose = `Here is my answer.\n\n\`\`\`json\n${JSON.stringify({ findings: [finding({ id: 'PROSE' }), finding({ id: 'PROSE2' })] })}\n\`\`\`\n`
    const { result } = await geminiReview({
      stdout: agyStream(agyResult({ findings: [finding({ id: 'G1', severity: 'critical' })] }, { response: prose })),
    })

    expect(result.findings.map((f) => [f.critic_finding_id, f.severity])).toEqual([['G1', 'critical']])
    expect(result.critics).toEqual([
      expect.objectContaining({
        client: 'agy-gemini-flash-high',
        cost_usd: null,
        family: 'google',
        finding_count: 1,
        model: 'gemini-3.8-flash-high',
        status: 'ok',
      }),
    ])
  })

  it('accepts an empty findings list that says what was checked', async () => {
    const { result } = await geminiReview(agyFindings({ findings: [], no_findings_reason: 'Walked every failure path.' }))

    expect(result.critics[0]).toMatchObject({ finding_count: 0, no_findings_reason: 'Walked every failure path.', status: 'ok' })
  })

  it('accepts a review that only read, listed, searched and finished', async () => {
    const { result } = await geminiReview(
      agyFindings({ findings: [finding()] }, ['list_dir', 'view_file', 'read_file', 'grep_search', 'find_by_name', 'finish']),
    )

    expect(result.critics[0]?.status).toBe('ok')
  })

  it('keeps the result event next to the raw output', async () => {
    const { result } = await geminiReview(agyFindings({ findings: [finding()] }))

    const critic = result.critics[0]
    expect(JSON.parse(await readFile(critic?.export_path ?? '', 'utf8'))).toMatchObject({ status: 'SUCCESS' })
    expect(await readFile(critic?.stdout_path ?? '', 'utf8')).toContain('"event":"result"')
  })

  it.each<[string, AgyScript, string]>([
    ['an empty response', { stdout: agyStream(agyResult({ findings: [finding()] }, { response: '' })) }, 'empty-output'],
    [
      'a denied action',
      {
        stdout: agyStream(
          agyResult({ findings: [finding()] }, { denied_actions: [{ action: 'write_file', display_name: 'WriteFile' }] }),
        ),
      },
      'denied-action',
    ],
    ['a missing structured_output', { stdout: agyStream(agyResult(undefined)) }, 'missing-structured-output'],
    [
      'a print timeout',
      { stdout: agyStream(agyResult({ findings: [finding()] }, { finished_reason: 'print_timeout' })) },
      'print-timeout',
    ],
    ['a non-zero exit code', { exitCode: 1, stderr: 'agy: authentication required', stdout: '' }, 'exit-1'],
    ['empty stdout', { stdout: '' }, 'empty-output'],
    ['stdout that is not JSON lines', { stdout: 'panic: runtime error\n' }, 'unparseable-output'],
    ['a stream with no result event', { stdout: `${JSON.stringify({ event: 'init', init: {} })}\n` }, 'unparseable-output'],
    ['structured output with an unknown severity', agyFindings({ findings: [finding({ severity: 'blocker' })] }), 'invalid-findings'],
    ['structured output that is not an object', agyFindings(['F1']), 'invalid-findings'],
    ['an empty list without no_findings_reason', agyFindings({ findings: [] }), 'missing-no-findings-reason'],
  ])('records %s as a failed critic, whatever the exit code', async (_name, script, failure) => {
    const { result } = await geminiReview(script)

    expect(result.critics[0]).toMatchObject({ failure, finding_count: 0, status: 'failed' })
    expect(result.findings).toEqual([])
    expect(result.degraded).toBe(true)
  })

  it.each<[string, string, string]>([
    [
      "agy's own error line",
      'error: Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 115h35m37s.\n' +
        'AGY_ERROR: {"short_error":"RESOURCE_EXHAUSTED (code 429): Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 115h35m37s.","status":"RESOURCE_EXHAUSTED","error_code":429}\n',
      'agy exited 3: RESOURCE_EXHAUSTED (code 429): Individual quota reached. Please upgrade your subscription to increase your limits. Resets in 115h35m37s.',
    ],
    ['the first line of stderr', '\n  agy: authentication required  \nmore\n', 'agy exited 3: agy: authentication required'],
    ['nothing when stderr is empty', '', 'agy exited 3'],
    ['the first line when the error line is not JSON', 'boom\nAGY_ERROR: {not json\n', 'agy exited 3: boom'],
  ])('says why agy exited non-zero, from %s', async (_name, stderr, detail) => {
    const { result } = await geminiReview({ exitCode: 3, stderr, stdout: '' })

    expect(result.critics[0]).toMatchObject({ detail, failure: 'exit-3', status: 'failed' })
  })

  it.each([
    ['an MCP tool call', 'call_mcp_tool'],
    ['a web search', 'search_web'],
    ['a URL fetch', 'read_url_content'],
    ['a browser action', 'browser_click_element'],
    ['a workflow, which can start a skill', 'run_workflow'],
    ['a subagent', 'invoke_subagent'],
    ['a shell command', 'run_command'],
  ])('discards the findings of a critic that made %s', async (_name, tool) => {
    const { result } = await geminiReview(agyFindings({ findings: [finding()] }, ['read_file', tool]))

    expect(result.critics[0]).toMatchObject({ failure: 'unsafe-tool', finding_count: 0, status: 'failed' })
    expect(result.critics[0]?.detail).toContain(tool)
    expect(result.findings).toEqual([])
  })
})

describe('Antigravity readiness', () => {
  async function prepared(options: HarnessOptions, families: readonly string[]) {
    const h = await harness(options)
    open.push(h)
    await h.commands.allow.run({ families, repo: h.repo })
    const run = await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })
    if (run.status !== 'ready') throw new Error('not ready')
    return { h, run }
  }

  it('checks the Keychain for the Antigravity CLI item, as setup does', async () => {
    const { h } = await prepared({}, ['google'])

    expect(h.agy.commands).toEqual([
      expect.objectContaining({ args: ['find-generic-password', '-s', 'Antigravity CLI'], command: 'security' }),
    ])
  })

  it('replaces a Gemini critic that is not signed in with GLM, in setup words', async () => {
    const { run } = await prepared({ agySignedIn: false }, ['mistral', 'google', 'zhipu'])

    expect(run.critics.map((critic) => [critic.client, critic.lenses])).toEqual([
      ['vibe-mistral-large', ['fidelity', 'testability']],
      ['vibe-glm', ['failure-modes', 'security']],
    ])
    expect(run.dropped).toEqual([
      {
        client: 'agy-gemini-flash-high',
        detail: SIGN_IN,
        family: 'google',
        reason: 'unavailable',
        replaced_by: 'c2-vibe-glm',
      },
    ])
  })

  it('counts agy as not signed in when the Keychain cannot be asked', async () => {
    const { run } = await prepared({ agySignedIn: 'no-security-tool' }, ['google', 'zhipu'])

    expect(run.dropped).toContainEqual(expect.objectContaining({ client: 'agy-gemini-flash-high', detail: SIGN_IN }))
  })

  it('finds agy by an absolute path in routing', async () => {
    const routing = JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as {
      clients: Record<string, Record<string, unknown>>
    }
    routing.clients['agy-gemini-flash-high'] = { ...routing.clients['agy-gemini-flash-high'], binary: '/nowhere/agy' }
    const { run } = await prepared({ routing }, ['google'])

    expect(run.dropped).toContainEqual(expect.objectContaining({ detail: 'antigravity: /nowhere/agy is not on PATH' }))
  })

  it('counts agy as unavailable when it is not on PATH', async () => {
    const { h, run } = await prepared({ agyOnPath: false }, ['google', 'zhipu'])

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-glm'])
    expect(run.dropped).toContainEqual(
      expect.objectContaining({ client: 'agy-gemini-flash-high', detail: 'antigravity: agy is not on PATH', reason: 'unavailable' }),
    )
    expect(h.agy.commands).toEqual([])
  })

  it('counts agy as unavailable off macOS without asking the Keychain', async () => {
    const { h, run } = await prepared({ platform: 'linux' }, ['google', 'zhipu'])

    expect(run.critics.map((critic) => critic.client)).toEqual(['vibe-glm'])
    expect(run.dropped).toContainEqual(
      expect.objectContaining({
        client: 'agy-gemini-flash-high',
        detail: 'antigravity: macOS only; not set up on linux',
        reason: 'unavailable',
      }),
    )
    expect(h.agy.commands).toEqual([])
  })

  it('drops an unavailable Gemini critic and says so when the allowlist permits no fallback', async () => {
    const { h, run } = await prepared({ agySignedIn: false }, ['google'])

    expect(run.critics).toEqual([])
    expect(run.dropped).toContainEqual({ client: 'agy-gemini-flash-high', detail: SIGN_IN, family: 'google', reason: 'unavailable' })
    await h.commands.fanOut.run({ run_dir: run.run_dir })
    const { summary } = await h.commands.record.run({ run_dir: run.run_dir, verdicts: [] })
    expect(summary).toContain(`Not run: agy-gemini-flash-high (google): unavailable (${SIGN_IN}); no fallback left`)
  })

  it('asks nothing about agy when the allowlist does not permit Google', async () => {
    const { h } = await prepared({}, ['mistral'])

    expect(h.agy.commands).toEqual([])
  })
})

describe('agy models in routing', () => {
  function agyRouting(model: string): unknown {
    return {
      clients: { 'agy-x': { binary: 'agy', family: 'google', kind: 'agy', model } },
      stages: {
        spec: {
          base: [{ client: 'agy-x', lenses: ['security'], max_price_usd: 0, role: 'spec-redteam', time_limit_s: 60 }],
          distinct_families: true,
        },
      },
      version: 1,
    }
  }

  it.each(['claude-sonnet-4-6', 'claude-opus-4-6-thinking', 'gpt-oss-120b-medium'])(
    'never lets agy run %s as a critic',
    async (model) => {
      const h = await harness({ routing: agyRouting(model) })
      open.push(h)
      await h.commands.allow.run({ families: ['google'], repo: h.repo })

      await expect(
        h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }),
      ).rejects.toThrow(/routing\.json: .*model must be a Gemini model/)
    },
  )

  it('rejects an agy client of any family but Google', async () => {
    const routing = agyRouting('gemini-3.8-flash-high') as { clients: Record<string, Record<string, unknown>> }
    routing.clients['agy-x'] = { ...routing.clients['agy-x'], family: 'mistral' }
    const h = await harness({ routing })
    open.push(h)
    await h.commands.allow.run({ families: ['mistral'], repo: h.repo })

    await expect(
      h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' }),
    ).rejects.toThrow(/routing\.json: .*family must be "google"/)
  })
})
