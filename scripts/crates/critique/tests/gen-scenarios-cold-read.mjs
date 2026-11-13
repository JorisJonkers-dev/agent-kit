#!/usr/bin/env node
// Writes the scenario.json files of cold-read (the goldens are expected.json, from regenerate-fixtures.mjs).
// A scenario's `assets` flag makes the Rust replay read the node runner's real shipped folder, since the
// command reads roles/cold-reader.md from it.
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const step = (argv, extra = {}) => ({ argv, ...extra })
const coldRead = (input, extra) => step(['cold-read', '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const SB = '@SANDBOX@'
const run = (id, files) => ({ files, run_dir: `${SB}/runs/${id}` })
const scenarios = {}

const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
/** A run's manifest, as `prepare` writes one. */
const manifest = (id, extra = {}) =>
  `${JSON.stringify({
    allowed_families: ['mistral'],
    artifact: { labels: [], reference: 'o/r#1', tracker: 'github' },
    created_at: '2026-10-30T12:00:00.000Z',
    critics: [],
    depth: 'base',
    depth_choice: { chosen_by: 'argument', measures: {}, signals: [] },
    distinct_families: true,
    dropped: [],
    flow: 'flow-1',
    limits: LIMITS,
    orchestrator_family: 'anthropic',
    packet_dir: `${SB}/runs/${id}/packet`,
    packet_files: [],
    repo_root: `${SB}/repos/cr`,
    repository: 'o/r',
    run_dir: `${SB}/runs/${id}`,
    run_id: id,
    stage: 'spec',
    version: 1,
    ...extra,
  })}\n`
const runs = (ids, extra) => Object.fromEntries(ids.map((id) => [`runs/${id}/manifest.json`, manifest(id, extra?.[id])]))

const REPO = {
  'repos/cr/docs/adr/0001-db.md': '# 1. DB\n\nWe use Postgres.\n',
  'repos/cr/docs/adr/0002-events.md': '# 2. Events\n\nEvents are append-only. Revisit in 2027.\n',
  'repos/cr/docs/adr/sub dir/0003-ünï.md': '# 3. Ünï\n\nUnicode names work.\n',
}
const HOME = { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\n', 'home/.vibe/agents/critic-mistral-small.toml': '# agent\n' }

const GOOD_EXPORT = { config: { enable_mcp: false, enabled_tools: ['read_file'], include_project_context: false, outside_workdir: 'deny' }, cost_usd: 0.002, usage: { input_tokens: 1200, cached_input_tokens: 200, output_tokens: 300 } }
const journal = (text) => JSON.stringify([{ type: 'message', role: 'user', content: [{ text: 'q' }] }, { type: 'message', role: 'assistant', content: [{ text }] }])
const fence = (body) => `Here is what I read.\n\`\`\`json\n${JSON.stringify(body)}\n\`\`\`\nDone.\n`
const q = (text) => `'${text.split("'").join("'\\''")}'`
/** One fake `vibe` outcome: what it writes to its export, stdout and stderr, and how it exits. */
const outcome = ({ exported = GOOD_EXPORT, stdout = '', stderr = '', code = 0 }) =>
  `${exported === null ? '' : `printf '%s\\n' ${q(JSON.stringify(exported))} > "$out/export.json"\n`}printf '%s' ${q(stdout)}\nprintf '%s' ${q(stderr)} >&2\nexit ${code}`
/** A fake `vibe` program: it takes the folder its `--output-dir` names and answers by the run and record it was launched for. */
const fake = (cases) =>
  `#!/bin/sh\nout=''\nwhile [ $# -gt 0 ]; do\n  if [ "$1" = --output-dir ]; then out="$2"; fi\n  shift\ndone\nmkdir -p "$out"\ncase "$out" in\n${Object.entries(cases)
    .map(([where, body]) => `*${where}/export)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const fakeStep = (cases, write = {}) => ({ write: { ...write, 'bin/vibe': fake(cases) }, exec: ['bin/vibe'], shims: ['env', 'mkdir'], path: ['bin'] })

// Answers: complete (with white space to trim), partial (an empty and a missing answer), and the last block of two.
scenarios['cold-read-answered'] = [
  coldRead(run('r1', ['docs/adr/0001-db.md', 'docs/adr/0002-events.md', 'docs/adr/sub dir/0003-ünï.md']), {
    ...fakeStep({
      '/r1/cold-read/1': outcome({ stdout: journal(fence({ decided: '  Use Postgres.\n', rejected: '\tMongo, for joins.', revisit: 'Never  ' })) }),
      '/r1/cold-read/2': outcome({ stdout: journal(fence({ decided: 'Append-only events.', rejected: '', extra: 1 })), exported: { ...GOOD_EXPORT, usage: undefined } }),
      '/r1/cold-read/3': outcome({ stdout: journal(`${fence({ decided: 'x' })}${fence({ decided: 5 })}`), exported: { ...GOOD_EXPORT, cost_usd: undefined, usage: { input_tokens: 0, output_tokens: 0 } } }),
    }, { ...REPO, ...HOME, ...runs(['r1']) }),
    roots: ['runs'],
  }),
  coldRead(run('r1', ['docs/adr/0001-db.md']), { roots: ['runs'], path: ['bin'] }),
]

// What a run's launch can end in, and what the reply's fence may look like.
const FENCES = {
  'r-exit': outcome({ code: 3, stderr: 'boom\n', stdout: journal(fence({ decided: 'a' })) }),
  'r-timeoutish': outcome({ code: 124, stderr: '', exported: { ...GOOD_EXPORT, outcome: 'time-limit' } }),
  'r-unsafe': outcome({ exported: { ...GOOD_EXPORT, config: { ...GOOD_EXPORT.config, enable_mcp: true } }, stdout: journal(fence({ decided: 'a' })) }),
  'r-empty': outcome({ stdout: '  \n' }),
  'r-noexport': outcome({ exported: null, stdout: journal(fence({ decided: 'a' })) }),
  'r-nojournal': outcome({ stdout: 'not json' }),
  'r-crlf': outcome({ stdout: journal('Pre\n```json \t\r\n{"decided":"crlf","rejected":"r","revisit":"v"}\r\n```\n') }),
  'r-nonewline': outcome({ stdout: journal('```json\n{"decided":"tight","rejected":"r","revisit":"v"}```') }),
  'r-fenceless': outcome({ stdout: journal('{"decided":"no fence"}') }),
  'r-badjson': outcome({ stdout: journal('```json\n{decided}\n```') }),
  'r-array': outcome({ stdout: journal('```json\n["decided"]\n```') }),
  'r-unclosed-first': outcome({ stdout: journal('```json x\n{"decided":"no"}\n```\n```json\n{"decided":"yes"}\n```\n```json\n{"decided":"open"}') }),
  'r-nulls': outcome({ stdout: journal(fence({ decided: null })) }),
  'r-text-parts': outcome({ stdout: JSON.stringify([{ type: 'message', role: 'assistant', content: [{ text: '```json\n{"decided":"split ' }, { image: 1 }, { text: 'reply"}\n```' }] }]) }),
  'r-no-assistant': outcome({ stdout: JSON.stringify([{ type: 'message', role: 'user', content: [{ text: 'q' }] }]) }),
}
const ids = Object.keys(FENCES)
scenarios['cold-read-launches'] = ids.map((id, index) =>
  coldRead(run(id, ['docs/adr/0001-db.md']), {
    ...(index === 0 ? fakeStep(Object.fromEntries(ids.map((id) => [`/${id}/cold-read/1`, FENCES[id]])), { ...REPO, ...HOME, ...runs(ids) }) : { path: ['bin'] }),
    roots: ['runs'],
  }),
)

// Why a reader may not run: the allowlist, a missing client, a client not signed in, a missing agent.
scenarios['cold-read-blocked'] = [
  coldRead(run('b-family', ['docs/adr/0001-db.md', 'docs/adr/0002-events.md']), { write: { ...REPO, ...runs(['b-family', 'b-nobin', 'b-nokey', 'b-noagent', 'b-emptykey'], { 'b-family': { allowed_families: ['deepseek', 'zhipu'] } }) }, roots: ['runs'] }),
  coldRead(run('b-nobin', ['docs/adr/0001-db.md']), { roots: ['runs'], path: ['bin'] }),
  coldRead(run('b-nokey', ['docs/adr/0001-db.md']), { write: { 'bin/vibe': '#!/bin/sh\nexit 0\n' }, exec: ['bin/vibe'], path: ['bin'], roots: ['runs'] }),
  coldRead(run('b-noagent', ['docs/adr/0001-db.md']), { write: { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\n' }, path: ['bin'], roots: ['runs'] }),
  coldRead(run('b-emptykey', ['docs/adr/0001-db.md']), { write: { 'home/.vibe/.env': 'MISTRAL_API_KEY=\n' }, path: ['bin'], roots: ['runs'] }),
]

// The ledger: a gateway ledger that will not take a record, a spec-bearing ticket-review manifest, and a launch that cannot start.
const SPEC_FILE = { kind: 'spec', path: 'spec/spec.md', sha256: 'h', source: 'o/r#7' }
scenarios['cold-read-ledger'] = [
  coldRead(run('l-spec', ['docs/adr/0001-db.md']), {
    ...fakeStep({ '/l-spec/cold-read/1': outcome({ stdout: journal(fence({ decided: 'a', rejected: 'b', revisit: 'c' })) }), '/l-warn/cold-read/1': outcome({ stdout: journal(fence({ decided: 'a' })) }), '/l-warn/cold-read/2': outcome({ code: 1, stderr: 'late\n' }) }, { ...REPO, ...HOME, ...runs(['l-spec', 'l-warn', 'l-nolaunch'], { 'l-spec': { stage: 'ticket-review', packet_files: [SPEC_FILE], round: 1 }, 'l-nolaunch': { flow: 'flow-9', round: 2, previous_run_id: 'l-spec' } }) }),
    roots: ['runs'],
  }),
  coldRead(run('l-warn', ['docs/adr/0001-db.md', 'docs/adr/0002-events.md']), { remove: ['state'], write: { state: 'a file where the state directory belongs\n' }, path: ['bin'], roots: ['runs'] }),
  coldRead(run('l-nolaunch', ['docs/adr/0001-db.md']), { remove: ['bin/env', 'state'], path: ['bin'], roots: ['runs'] }),
]

// Input and run errors, in the order the command meets them.
const ok = ['docs/adr/0001-db.md']
const ESCAPE = { files: ['../escape.md'], run_dir: `${SB}/runs/e1` }
scenarios['cold-read-errors'] = [
  coldRead({}, { write: { ...REPO, ...HOME, ...runs(['e1']), 'runs/e-badjson/manifest.json': 'not json', 'runs/e-shape/manifest.json': '{}\n', 'runs/e-version/manifest.json': manifest('e-version').replace('"version":1', '"version":2') } }),
  coldRead([]), coldRead('null'), coldRead({ files: [] }), coldRead({ files: 'x', run_dir: 'y' }), coldRead({ files: [5], run_dir: 'y' }),
  coldRead({ files: ok }), coldRead({ files: ok, run_dir: '' }), coldRead({ files: ok, run_dir: '  ' }),
  coldRead({ files: [], run_dir: 'y' }),
  coldRead({ files: ok, run_dir: `${SB}/runs/missing` }),
  coldRead(run('e-badjson', ok)), coldRead(run('e-shape', ok)), coldRead(run('e-version', ok)),
  coldRead(ESCAPE),
  coldRead({ files: ['missing.md'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ['missing.md', '../escape.md'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ['../escape.md', 'missing.md'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ['docs'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ['/etc/passwd'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ['docs/adr/0001-db.md', 'docs/adr/missing.md'], run_dir: `${SB}/runs/e1` }),
  coldRead({ files: ok, run_dir: `${SB}/runs/e1`, ignored: 1 }, { write: { 'home/.config/agent-kit/critique-routing.json': '{"cold_read":null}\n' } }),
  coldRead({ files: ok, run_dir: `${SB}/runs/e1` }, { write: { 'home/.config/agent-kit/critique-routing.json': '{' } }),
  coldRead({ files: ok, run_dir: `${SB}/runs/e1` }, { write: { 'home/.config/agent-kit/critique-routing.json': '{"cold_read":{"client":"nope","max_price_usd":1,"time_limit_s":1}}' } }),
  coldRead({ files: ok, run_dir: `${SB}/runs/e1` }, { write: { 'home/.config/agent-kit/critique-routing.json': '[]' } }),
  step(['cold-read', '--json', JSON.stringify({ files: ok, run_dir: `${SB}/runs/e1` }), '--strict'], { remove: ['home/.config'], path: ['bin'] }),
]


// A plan limit reached by the first run: the second run's reader may not start, and says why.
const PLAN = '{"clients":{"vibe":{"plan_limits":[{"window":"day","limit_usd":0.0002}]}}}\n'
scenarios['cold-read-plan'] = [
  coldRead(run('p-1', ok), { ...fakeStep({ '/p-1/cold-read/1': outcome({ stdout: journal(fence({ decided: 'a' })) }) }, { ...REPO, ...HOME, ...runs(['p-1', 'p-2', 'p-3']) }), roots: ['runs'] }),
  coldRead(run('p-2', ok), { write: { 'home/.config/agent-kit/model-gateway.json': PLAN }, path: ['bin'], roots: ['runs'] }),
  coldRead(run('p-3', ok), { write: { 'home/.config/agent-kit/model-gateway.json': '{' }, path: ['bin'], roots: ['runs'] }),
]

// A reader pinned to a gateway client model (routing's override), another family, and a launch priced by its tokens.
const PINNED = '{"cold_read":{"client":"vibe-glm","max_price_usd":0.5,"time_limit_s":30}}\n'
scenarios['cold-read-pinned'] = [
  coldRead(run('g-1', ok), { ...fakeStep({ '/g-1/cold-read/1': outcome({ stdout: journal(fence({ decided: 'glm' })) }) }, { ...REPO, ...HOME, 'home/.vibe/agents/critic-glm.toml': '# agent\n', 'home/.config/agent-kit/critique-routing.json': PINNED, ...runs(['g-1', 'g-2'], { 'g-1': { allowed_families: ['zhipu'] }, 'g-2': { allowed_families: ['mistral'] } }) }), roots: ['runs'] }),
  coldRead(run('g-2', ok), { path: ['bin'], roots: ['runs'] }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ assets: true, steps }, null, 2)}\n`)
}
