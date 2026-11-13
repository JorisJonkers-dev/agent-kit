#!/usr/bin/env node
// Writes the scenario.json files of fan-out (the goldens are expected.json, from regenerate-fixtures.mjs). The critics
// are fake `agy` programs that answer by the critic whose directory they run in; the shipped assets (roles, lenses,
// prompts, schema) are the real ones.
import { createHash } from 'node:crypto'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const sha = (text) => createHash('sha256').update(text).digest('hex')
const q = (text) => `'${text.split("'").join("'\\''")}'`
const step = (argv, extra = {}) => ({ argv, ...extra })
const fanOut = (input, extra) => step(['fan-out', '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const go = (id, extra) => fanOut({ run_dir: `${SB}/runs/${id}` }, extra)
const scenarios = {}

const SPEC = 'The system retries failed requests without a limit.\nThe retry budget is unbounded.\nUsers see a spinner while it retries.\n'
const ADR = '# 1. Use Postgres\n\nWe use Postgres for the ledger.\n\nStatus: accepted\n'
const REFERENCE = 'Decision: retries are capped at three.\n'
const QUOTE = 'The retry budget is unbounded.'
const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
const AGY = { binary: 'agy', family: 'google', kind: 'agy', model: 'gemini-3.8-flash-high' }
const AGY_B = { ...AGY, model: 'gemini-3.8-pro' }
const critic = (id, extra = {}) => ({
  client: 'agy-main', client_config: AGY, critic_id: id, fallbacks: [{ client: 'agy-b', client_config: AGY_B }], lenses: ['failure-modes'], max_price_usd: 2, role: 'spec-redteam', time_limit_s: 300, ...extra,
})
const FILES = {
  spec: { 'artifact/spec.md': SPEC, 'references/decision.md': REFERENCE },
  adr: { 'artifact/adr/0001.md': ADR },
}
const manifest = (id, stage, critics, extra = {}, files = FILES.spec) =>
  `${JSON.stringify({
    allowed_families: ['google', 'mistral'],
    artifact: { labels: [], reference: 'o/r#1', tracker: 'github' },
    created_at: '2026-10-30T12:00:00.000Z',
    critics,
    depth: 'base',
    depth_choice: { chosen_by: 'argument', measures: {}, signals: [] },
    distinct_families: true,
    dropped: [],
    flow: 'flow-1',
    limits: LIMITS,
    orchestrator_family: 'anthropic',
    packet_dir: `${SB}/runs/${id}/packet`,
    packet_files: Object.entries(files).map(([path, text]) => ({ kind: path.startsWith('artifact') ? 'artifact' : 'reference', path, sha256: sha(text), source: 's' })),
    repo_root: `${SB}/repos/r`,
    repository: 'o/r',
    run_dir: `${SB}/runs/${id}`,
    run_id: id,
    stage,
    version: 1,
    ...extra,
  })}\n`
const run = (id, critics, { stage = 'spec', extra, files = FILES.spec } = {}) => ({
  [`runs/${id}/manifest.json`]: manifest(id, stage, critics, extra, files),
  ...Object.fromEntries(Object.entries(files).map(([path, text]) => [`runs/${id}/packet/${path}`, text])),
})

const agyStream = (result) => `printf '%s\\n' ${q(JSON.stringify({ event: 'result', result }))}`
const answers = (findings, { sleep = 0, extra = '' } = {}) =>
  `${sleep > 0 ? `sleep ${sleep}\n` : ''}${extra}${agyStream({ response: 'done', structured_output: findings.length === 0 ? { findings, no_findings_reason: 'nothing found' } : { findings } })}\nexit 0`
const found = (id, extra = {}) => ({
  category: 'failure-mode', check: 'dependency-fails', claim: `claim ${id}`, evidence: [{ kind: 'quote', value: QUOTE }, { kind: 'input', value: 'the endpoint is down' }], id, location: 'artifact/spec.md', severity: 'minor', ...extra,
})
const fakeAgy = (cases) =>
  `#!/bin/sh\nif [ "$1" = models ]; then\nprintf '%s\\n' 'Fetching available models...' 'gemini-3.8-flash-high\tGemini 3.8 Flash' 'gemini-3.8-pro\tGemini 3.8 Pro'\nexit 0\nfi\ncase "$PWD" in\n${Object.entries(cases)
    .map(([id, body]) => `*/critics/${id}/packet)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const agyStep = (cases, write = {}, extra = {}) => ({ write: { ...write, 'bin/agy': fakeAgy(cases), 'repos/r/README.md': '# r\n' }, exec: ['bin/agy'], shims: ['env', 'git', 'sleep'], path: ['bin'], git: GIT, ...extra })
const GIT = [['init', '-q', '--template=', 'repos/r'], ['-C', 'repos/r', 'remote', 'add', 'origin', 'git@github.com:o/r.git'], ['-C', 'repos/r', 'add', '-A'], ['-C', 'repos/r', 'commit', '-q', '-m', 'x']]
const stateless = (extra = {}) => ({ path: ['bin'], ...extra })

// Two critics side by side: the same defect raised twice merges, and every finding gets the evidence check's word.
scenarios['fan-out-merge'] = [
  go('m1', {
    ...agyStep(
      {
        'c1-agy': answers([
          found('a', { severity: 'minor' }),
          found('b', { severity: 'critical', suggestion: 'cap the retries' }),
          found('c', { check: 'made-up-check', location: 'artifact/spec.md:2', evidence: [{ kind: 'quote', value: 'Users see a spinner while it retries.' }, { kind: 'input', value: 'x' }] }),
          found('d', { check: undefined, location: 'a/b', evidence: [{ kind: 'quote', value: 'without a limit.' }, { kind: 'input', value: 'y' }] }),
          found('e', { location: 'elsewhere', evidence: [{ kind: 'quote', value: 'text nobody wrote' }] }),
          found('f', { location: 'again', evidence: [] }),
          found('g', { check: 'unbounded-call', location: 'x', evidence: [{ kind: 'quote', value: 'references-only' }, { kind: 'input', value: 'z' }], suggestion: 'first' }),
        ]),
        'c2-agy': answers([found('h', { claim: 'Claim C again', location: 'ARTIFACT/spec.md:2 ', severity: 'major', suggestion: 'second', evidence: [{ kind: 'quote', value: 'a spinner while it' }, { kind: 'input', value: 'x' }] }), found('i', { location: 'else', claim: 'A NEW ONE.', evidence: [] })], { sleep: 1 }),
      },
      { ...run('m1', [critic('c1-agy'), critic('c2-agy', { lenses: [] })], { files: { ...FILES.spec, 'references/other.md': 'references-only text\n' } }) },
    ),
    roots: ['runs'],
  }),
  go('m1', stateless({ roots: ['runs'] })),
]

// What a failed critic does: it is retried through a fallback, unless the quota is spent, it wrote to its packet, it has
// none, or the family it would bring already reviewed.
const FAIL = `printf '%s' 'boom\\n' >&2\nexit 3`
const OK = answers([found('a')])
const todayLines = (count) => Array.from({ length: count }, (_, index) => JSON.stringify({ charged_usd: 0, client: 'agy-main', critic_id: `old-${index}`, family: 'google', flow: 'old', kind: 'agy', model: 'm', recorded_at: '2026-11-03T08:00:00.000Z', repository: 'o/r', round: 1, run_id: 'old', stage: 'spec', status: 'ok', type: 'invocation' })).join('\n')
scenarios['fan-out-retry'] = [
  go('t1', {
    ...agyStep(
      { 'c1-agy': FAIL, 'c1-agy-retry-agy-b': OK, 't2c': FAIL, 't3c': answers([found('a')], { extra: 'echo planted > planted.txt\n' }), 't4c': FAIL, 't5a': FAIL, 't5b': OK, 't6c': FAIL, 't6c-retry-agy-b': FAIL, 't7a': FAIL, 't7b': `sleep 1\n${FAIL}`, 't7a-retry-agy-b': OK, 't7b-retry-agy-b': 'sleep 1\nexit 9' },
      {
        ...run('t1', [critic('c1-agy')]),
        ...run('t2', [critic('t2c')]),
        ...run('t3', [critic('t3c')]),
        ...run('t4', [critic('t4c', { fallbacks: [] })]),
        ...run('t5', [critic('t5a'), critic('t5b')]),
        ...run('t6', [critic('t6c')]),
        ...run('t7', [critic('t7a'), critic('t7b')], { extra: { distinct_families: false } }),
      },
    ),
    roots: ['runs'],
  }),
    go('t3', stateless({ roots: ['runs'] })),
  go('t4', stateless({ roots: ['runs'] })),
  go('t5', stateless({ roots: ['runs'] })),
  go('t6', stateless({ roots: ['runs'] })),
  go('t7', stateless({ roots: ['runs'] })),
  go('t2', { write: { 'home/.local/share/agent-kit/critique/ledger.jsonl': `${todayLines(20)}\n` }, path: ['bin'], roots: ['runs'] }),
]

// The quota is spent (last, since the ledger keeps it), so agy may not retry.
// A run whose packet holds no code: the checks that need it are skipped, and the report says so.
const NO_CONFIRM = found('a', { category: 'drift', check: 'no-confirmation', claim: 'drifts', evidence: [{ kind: 'quote', value: 'We use Postgres for the ledger.' }], location: 'artifact/adr/0001.md' })
scenarios['fan-out-skipped'] = [
  go('s1', {
    ...agyStep(
      { 's1c': answers([NO_CONFIRM, found('b', { category: 'unasked-question', check: 'no-decision-sentence', evidence: [{ kind: 'quote', value: 'We use Postgres for the ledger.' }, { kind: 'input', value: 'x' }], location: 'artifact/adr/0001.md', severity: 'minor' })]) },
      run('s1', [critic('s1c', { lenses: ['adr-record'], role: 'adr-reviewer' })], { stage: 'adr-review', files: FILES.adr }),
    ),
    roots: ['runs'],
  }),
  // Every critic failed: the run is degraded.
  go('s2', { write: run('s2', [critic('s2c', { fallbacks: [] })]), path: ['bin'], roots: ['runs'] }),
  // No critic at all.
  go('s3', { write: run('s3', []), path: ['bin'], roots: ['runs'] }),
]

// A nedap repository's run: its critics are the work clients, and Codex answers in its own event format.
const codexSays = (structured) => `printf '%s\\n' ${q(JSON.stringify({ type: 'item.completed', item: { type: 'agent_message', text: `\`\`\`json\n${JSON.stringify(structured)}\n\`\`\`` } }))}\nexit 0`
scenarios['fan-out-work'] = [
  go('w1', {
    write: { ...run('w1', [critic('w1c', { ...{ client: 'codex-work', client_config: { binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' }, fallbacks: [] } })], { extra: { repository: 'nedap/x', work: { grant_workflow: 'wf-1' } } }), 'bin/codex': `#!/bin/sh\n${codexSays({ findings: [found('a')] })}\n`, 'repos/r/README.md': '# r\n' },
    exec: ['bin/codex'], shims: ['env', 'git'], path: ['bin'], git: GIT, roots: ['runs'],
  }),
]

// What a run must be before it is fanned out, and the input errors, in the order the command meets them.
const WORK = { client: 'codex-work', client_config: { binary: 'codex', family: 'openai', kind: 'codex', model: 'gpt-5.5' }, fallbacks: [] }
scenarios['fan-out-errors'] = [
  fanOut({}, { write: { ...run('e1', [critic('c1')]), 'runs/e-badjson/manifest.json': 'not json', 'runs/e-shape/manifest.json': '{}\n', ...run('e-work', [critic('c1', WORK)]), ...run('e-mismatch', [critic('c1')]), 'runs/e-mismatch/packet/artifact/spec.md': 'changed after prepare\n', ...run('e-norole', [critic('c1', { role: 'no-such-role' })]) } }),
  fanOut([]), fanOut('null'), fanOut({ run_dir: 5 }), fanOut({}), fanOut({ run_dir: '' }),
  fanOut({ run_dir: `${SB}/runs/missing` }),
  go('e-badjson'), go('e-shape'), go('e-work'), go('e-mismatch'),
  go('e-norole', { roots: ['runs'] }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ assets: true, steps }, null, 2)}\n`)
}
console.log(`${Object.keys(scenarios).length} scenarios`)
