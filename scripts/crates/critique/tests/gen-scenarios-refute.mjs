#!/usr/bin/env node
// Writes the scenario.json files of refute (the goldens are expected.json, from regenerate-fixtures.mjs). The refuters
// are fake `agy` programs that answer by the finding whose refute directory they run in; the shipped assets (the
// refuter role, the prompts, the schema, routing) are the real ones.
import { createHash } from 'node:crypto'
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, 'fixtures')
const shipped = JSON.parse(readFileSync(join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'routing.json'), 'utf8'))
const SB = '@SANDBOX@'
const sha = (text) => createHash('sha256').update(text).digest('hex')
const q = (text) => `'${text.split("'").join("'\\''")}'`
const step = (argv, extra = {}) => ({ argv, ...extra })
const refute = (input, extra) => step(['refute', '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const scenarios = {}

const SPEC = 'The system retries failed requests without a limit.\nThe retry budget is unbounded.\n'
const QUOTE = 'The retry budget is unbounded.'
const LIMITS = { antigravity_quota: { daily_cap: 20, reserve_threshold: 10, reserved_stages: ['spec'] }, search: { max_bytes: 8192, max_matches: 50 } }
const HASHES = { contract: 'c', preamble: 'p', role: 'r', schema: 's' }

const manifest = (id, extra = {}) =>
  `${JSON.stringify({
    allowed_families: ['mistral', 'google'],
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
    packet_files: [{ kind: 'artifact', path: 'artifact/spec.md', sha256: sha(SPEC), source: 'o/r#1' }],
    repo_root: `${SB}/repos/r`,
    repository: 'o/r',
    run_dir: `${SB}/runs/${id}`,
    run_id: id,
    stage: 'spec',
    version: 1,
    ...extra,
  })}\n`

const criticRun = (id, family, role = 'spec-redteam') => ({
  charged_usd: 0, client: `client-${id}`, cost_usd: null, critic_id: id, duration_ms: 0, exit_code: 0, family, finding_count: 1, lens_hashes: {}, lenses: [], model: 'm',
  prompt_hashes: HASHES, prompt_path: `${SB}/x/prompt.md`, role, status: 'ok',
})
const finding = (id, extra = {}) => ({
  category: 'bug', claim: `claim ${id}`, critic_finding_id: id, evidence: [{ kind: 'quote', value: QUOTE }, { kind: 'input', value: 'a case' }], id, location: 'artifact/spec.md', raised_by: ['c1'], severity: 'critical', ...extra,
})
const findings = (id, list, critics = [criticRun('c1', 'mistral')]) =>
  `${JSON.stringify({ critics, degraded: false, findings: list, findings_path: `${SB}/runs/${id}/findings.json`, run_id: id, stage: 'spec', status: 'reviewed' })}\n`

/** One run: its manifest, its findings and its frozen packet. */
const run = (id, list, { manifestExtra, critics } = {}) => ({
  [`runs/${id}/manifest.json`]: manifest(id, manifestExtra),
  [`runs/${id}/findings.json`]: findings(id, list, critics),
  [`runs/${id}/packet/artifact/spec.md`]: SPEC,
})
const accept = (...ids) => ids.map((finding_id) => ({ finding_id, verdict: 'accepted' }))
const go = (id, verdicts, extra) => refute({ run_dir: `${SB}/runs/${id}`, verdicts }, extra)

const agyStream = (result, tools = []) => `${tools.map((tool) => `printf '%s\\n' ${q(JSON.stringify({ step_update: { step_type: 'tool', tool_name: tool } }))}`).join('\n')}\nprintf '%s\\n' ${q(JSON.stringify({ event: 'result', result }))}`
const answers = (structured, { tools, extra = '', result = {} } = {}) => `${extra}${agyStream({ response: 'done', structured_output: structured, ...result }, tools)}\nexit 0`
const objection = (check, extra = {}) => ({
  category: 'omission', check, claim: 'It does not hold.', evidence: [{ kind: 'quote', value: QUOTE }, { kind: 'input', value: 'x' }], id: 'o1', location: 'elsewhere', severity: 'major', suggestion: 'drop it', ...extra,
})
const fakeAgy = (cases) =>
  `#!/bin/sh\nif [ "$1" = models ]; then\nprintf '%s\\n' 'Fetching available models...' 'gemini-3.8-flash-high\tGemini 3.8 Flash'\nexit 0\nfi\ncase "$PWD" in\n${Object.entries(cases)
    .map(([id, body]) => `*/refute/${id}/packet)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const agyStep = (cases, write = {}, extra = {}) => ({ write: { ...write, 'bin/agy': fakeAgy(cases) }, exec: ['bin/agy'], shims: ['env', 'git'], path: ['bin'], ...extra })

const REPO = { git: [['init', '-q', '--template=', 'repos/r'], ['-C', 'repos/r', 'remote', 'add', 'origin', 'git@github.com:o/r.git']], write: { 'repos/r/README.md': '# r\n' } }
const repoCommit = [['-C', 'repos/r', 'add', '-A'], ['-C', 'repos/r', 'commit', '-q', '-m', 'x']]
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const slot = (client) => shipped.stages.spec.base.find((entry) => entry.client === client)
// Only the base roster, so the candidates are the base clients and their fallbacks.
const routing = (over = {}) => `${JSON.stringify({ stages: { spec: { base: [slot('vibe-mistral-large'), slot('agy-gemini-flash-high')], deep: [], ...over } } })}\n`

// What a refuter may answer, and how each answer is judged.
const CASES = {
  f1: answers({ findings: [], no_findings_reason: 'The finding holds up.' }),
  f2: answers({ findings: [objection('refute')] }),
  f3: answers({ findings: [objection('downgrade')] }),
  f4: answers({ findings: [objection('downgrade', { severity: 'critical' })] }),
  f5: answers({ findings: [objection('refute'), objection('refute', { id: 'o2' })] }),
  f6: answers({ findings: [objection('other-check')] }),
  f7: answers({ findings: [objection('refute', { evidence: [{ kind: 'quote', value: 'text the packet does not hold' }] })] }),
  f8: `printf '%s' 'late\\n' >&2\nexit 3`,
  f9: answers({ findings: [objection('refute')] }, { tools: ['run_command', 'view_file'] }),
  f10: answers({ findings: [objection('refute')] }, { extra: 'echo planted > planted.txt\n' }),
  f11: answers({ findings: [objection(undefined)].map(({ check, ...rest }) => rest) }),
  f12: `printf '%s\\n' ${q(JSON.stringify({ event: 'result', result: { response: '' } }))}\nexit 0`,
  f13: answers({ findings: [objection('refute', { evidence: [] })] }),
  f14: answers({ findings: [objection('refute', { suggestion: undefined, check: ' refute ' })] }),
  f15: answers({ findings: [objection('refute', { evidence: [{ kind: 'quote', value: QUOTE, file: 'artifact/spec.md' }, { kind: 'input', value: 'x' }] })] }),
}
const ids = Object.keys(CASES)
// One run per answer: refuters of one round run side by side, so the order of their ledger lines is not fixed, and one
// refuter per run keeps the goldens exact.
const allRuns = Object.assign({}, ...ids.map((id) => run(`a-${id}`, [finding(id)])))
scenarios['refute-answers'] = [
  ...ids.map((id, index) =>
    go(`a-${id}`, accept(id), {
      ...(index === 0 ? agyStep(CASES, { ...allRuns, [OVERRIDE]: routing(), ...REPO.write }, { git: [...REPO.git, ...repoCommit] }) : { path: ['bin'] }),
      roots: ['runs'],
    }),
  ),
  go('a-f1', accept('f1'), { roots: ['runs'], path: ['bin'] }),
]

// Which findings go to a refuter: only an accepted critical one, as the verdict's severity override has it.
const DECLARED = { check: 'no-rollback', declared_severity: 'major', severity: 'critical' }
scenarios['refute-targets'] = [
  go('t1', [{ finding_id: 'g1', reason: 'not a defect', verdict: 'rejected' }, { finding_id: 'g2', verdict: 'accepted' }, { finding_id: 'g3', verdict: 'hint' }, { finding_id: 'g4', verdict: 'needs-human' }], {
    ...agyStep({}, { ...run('t1', [finding('g1'), finding('g2', { severity: 'major' }), finding('g3'), finding('g4', { severity: 'minor' })]), [OVERRIDE]: routing(), ...REPO.write }, { git: [...REPO.git, ...repoCommit] }),
    roots: ['runs'],
  }),
  // A severity override that makes it critical, and one that takes it out of the round.
  go('t2', [{ finding_id: 'h1', severity: 'critical', severity_reason: 'loses-data', verdict: 'accepted' }, { finding_id: 'h2', severity: 'minor', severity_reason: 'cosmetic', verdict: 'accepted' }], {
    ...agyStep({ h1: answers({ findings: [], no_findings_reason: 'fine' }) }, run('t2', [finding('h1', { ...DECLARED, severity: 'major', declared_severity: 'major', check: 'no-rollback' }), finding('h2', DECLARED)])),
    roots: ['runs'],
  }),
  // A finding fan-out rejected for good cannot be accepted; nor can a bad override be given.
  go('t3', [{ finding_id: 'j1', verdict: 'accepted' }], { write: run('t3', [finding('j1', { pre_verdict: { reason: 'quote not in packet', reason_code: 'fabricated-evidence', verdict: 'rejected' } })]), roots: ['runs'] }),
  go('t3', [{ finding_id: 'j1', reason: 'a', verdict: 'rejected' }, { finding_id: 'j1', verdict: 'accepted' }], { roots: ['runs'] }),
  go('t3', [{ finding_id: 'nope', verdict: 'accepted' }], { roots: ['runs'] }),
  go('t3', [{ finding_id: 'j1', reason: 'a', severity: 'major', severity_reason: 'x', verdict: 'rejected' }], { roots: ['runs'] }),
  go('t3', [{ finding_id: 'j1', reason: 'a', severity_reason: 'x', verdict: 'rejected' }], { roots: ['runs'] }),
  go('t4', [{ finding_id: 'k1', severity: 'major', verdict: 'accepted' }], { write: run('t4', [finding('k1', DECLARED)]), roots: ['runs'] }),
  go('t4', [{ finding_id: 'k1', severity: 'minor', severity_reason: 'x', verdict: 'accepted' }], { roots: ['runs'] }),
  go('t4', [{ finding_id: 'k1', severity: 'minor', verdict: 'accepted' }], { roots: ['runs'] }),
  // No finding to refute: the file is written and the ledger is left alone.
  go('t5', [], { write: run('t5', []), roots: ['runs', 'home'] }),
]

// Why a client may not refute: the family, a missing CLI, the quota, the work roster's grant.
const TODAY = '2026-11-03T08:00:00.000Z'
const agyLines = (count) => Array.from({ length: count }, (_, index) => JSON.stringify({ charged_usd: 0, client: 'agy-gemini-flash-high', critic_id: `old-${index}`, family: 'google', flow: 'old', kind: 'agy', model: 'm', recorded_at: TODAY, repository: 'o/r', round: 1, run_id: 'old', stage: 'spec', status: 'ok', type: 'invocation' })).join('\n')
const LEDGER = 'home/.local/share/agent-kit/critique/ledger.jsonl'
scenarios['refute-blocked'] = [
  // Everything barred by family: mistral raised it, google is not allowed, zhipu neither.
  go('b1', accept('m1'), { write: { ...run('b1', [finding('m1')], { manifestExtra: { allowed_families: ['mistral'] } }), [OVERRIDE]: routing(), ...REPO.write }, git: [...REPO.git, ...repoCommit], roots: ['runs', 'home'] }),
  // The orchestrator's own family, and both raisers.
  go('b2', accept('m1'), { write: run('b2', [finding('m1', { raised_by: ['c1', 'c2'] })], { manifestExtra: { allowed_families: ['mistral', 'google', 'zhipu'], orchestrator_family: 'zhipu' }, critics: [criticRun('c1', 'mistral'), criticRun('c2', 'google')] }), roots: ['runs'] }),
  // Vibe is not on PATH, so the first candidate cannot run; agy is the next, and answers.
  go('b3', accept('m1'), { ...agyStep({ m1: CASES.f1 }, { ...run('b3', [finding('m1', { raised_by: ['c1'] })], { critics: [criticRun('c1', 'deepseek')] }) }, {}), roots: ['runs'] }),
  // The agy quota is spent today: no client is left.
  go('b4', accept('m1'), { write: { ...run('b4', [finding('m1')], { critics: [criticRun('c1', 'deepseek')] }), [LEDGER]: `${agyLines(20)}\n` }, roots: ['runs'] }),
  // Past the reserve threshold, a stage that is not reserved may not use agy; the reserved ones may.
  go('b5', accept('m1'), { write: { ...run('b5', [finding('m1')], { critics: [criticRun('c1', 'deepseek')], manifestExtra: { stage: 'decisions' } }), [LEDGER]: `${agyLines(12)}\n` }, roots: ['runs'] }),
  // A CLI that is installed but cannot list its models.
  go('b6', accept('m1'), { write: { ...run('b6', [finding('m1')], { critics: [criticRun('c1', 'deepseek')] }), 'bin/agy': '#!/bin/sh\nprintf "%s\\n" "Error: Please sign in to view available models." >&2\nexit 1\n' }, exec: ['bin/agy'], shims: ['env'], path: ['bin'], roots: ['runs'] }),
  // A nedap repository's work roster: Codex needs a live grant, and the work Claude agent is the orchestrator's family.
  go('b7', accept('m1'), { write: run('b7', [finding('m1')], { critics: [criticRun('c1', 'deepseek')], manifestExtra: { allowed_families: ['openai', 'anthropic'], work: {}, repository: 'nedap/x' } }), roots: ['runs'] }),
  go('b8', accept('m1'), { write: run('b8', [finding('m1')], { critics: [criticRun('c1', 'deepseek')], manifestExtra: { allowed_families: ['openai'], work: { grant_workflow: 'wf-1' }, repository: 'nedap/x' } }), roots: ['runs'] }),
]

// A routing that names no such client, and a stage routing does not define.
scenarios['refute-routing'] = [
  go('r1', accept('m1'), { write: { ...run('r1', [finding('m1')]), [OVERRIDE]: `${JSON.stringify({ stages: { spec: { base: [{ ...slot('vibe-mistral-large'), client: 'nope' }] } } })}\n` }, roots: ['runs'] }),
  go('r2', accept('m1'), { write: { ...run('r2', [finding('m1')], { manifestExtra: { stage: 'plan' } }), [OVERRIDE]: `${JSON.stringify({ stages: { plan: null } })}\n` }, roots: ['runs'] }),
]

// What a run must be before it is refuted, and the input errors, in the order the command meets them.
scenarios['refute-errors'] = [
  refute({}, { write: { ...run('e1', [finding('m1')]), 'runs/e-badjson/manifest.json': 'not json', 'runs/e-shape/manifest.json': '{}\n', 'runs/e-nofind/manifest.json': manifest('e-nofind'), 'runs/e-badfind/manifest.json': manifest('e-badfind'), 'runs/e-badfind/findings.json': 'not json', 'runs/e-recorded/manifest.json': manifest('e-recorded'), 'runs/e-recorded/findings.json': findings('e-recorded', []), 'runs/e-recorded/report.md': '# report\n', 'runs/e-done/manifest.json': manifest('e-done'), 'runs/e-done/findings.json': findings('e-done', []), 'runs/e-done/refute.json': '{}\n' } }),
  refute([]), refute('null'), refute({ run_dir: 5, verdicts: [] }), refute({ run_dir: 'x' }), refute({ run_dir: 'x', verdicts: 'y' }), refute({ run_dir: 'x', verdicts: [5] }), refute({ run_dir: 'x', verdicts: [{ finding_id: 'a' }] }),
  refute({ run_dir: `${SB}/runs/missing`, verdicts: [] }),
  go('e-badjson', []), go('e-shape', []), go('e-nofind', []), go('e-badfind', []),
  go('e-recorded', []), go('e-done', []),
  go('e1', accept('m1', 'm1')),
  go('e1', accept('zz')),
]

// A launch that exits non-zero, and a gateway ledger that will not take the record of one.
scenarios['refute-launch'] = [
  go('l1', accept('m1'), { write: { ...run('l1', [finding('m1')], { critics: [criticRun('c1', 'deepseek')] }), ...run('l2', [finding('m1')], { critics: [criticRun('c1', 'deepseek')] }), 'bin/agy': '#!/bin/sh\nif [ "$1" = models ]; then printf "%s\\n" "gemini-3.8-flash-high"; exit 0; fi\nexit 5\n', [OVERRIDE]: routing(), ...REPO.write }, git: [...REPO.git, ...repoCommit], exec: ['bin/agy'], shims: ['env', 'git'], path: ['bin'], roots: ['runs'] }),
  go('l2', accept('m1'), { remove: ['state'], write: { state: 'a file where the state directory belongs\n' }, path: ['bin'], roots: ['runs'] }),
]

// A code stage: the prompt lists the repo map, the diff and the context slices, and the refuter reads the packet as prepare froze it.
const DIFF = 'diff --git a/src/a.ts b/src/a.ts\n--- a/src/a.ts\n+++ b/src/a.ts\n@@ -1,2 +1,3 @@\n const a = 1\n+const b = 2\n const c = 3\n'
const CODE = { base: 'main', base_commit: 'b'.repeat(40), changed_files: ['tree/src/a.ts'], head: 'HEAD', head_commit: 'c'.repeat(40), slices: [{ first: 1, last: 3, path: 'tree/src/a.ts', reason: 'the changed function' }] }
const codePacket = { 'diff/change.diff': DIFF, 'tree/src/a.ts': 'const a = 1\nconst b = 2\nconst c = 3\n', 'facts/INDEX.md': '# index\n', 'tree/REPO-MAP.md': '# map\n' }
scenarios['refute-code'] = [
  go('c1', accept('m1'), {
    ...agyStep({ m1: CASES.f1 }, { ...run('c1', [finding('m1')], { critics: [criticRun('c1', 'deepseek')], manifestExtra: { stage: 'feature-review', code: CODE, packet_files: Object.entries(codePacket).map(([path, text]) => ({ kind: path.startsWith('diff') ? 'diff' : path.startsWith('tree/src') ? 'tree' : 'fact', path, sha256: sha(text), source: 's' })) } }), ...Object.fromEntries(Object.entries(codePacket).map(([path, text]) => [`runs/c1/packet/${path}`, text])), [OVERRIDE]: JSON.stringify({ stages: { 'feature-review': { base: [slot('vibe-mistral-large'), slot('agy-gemini-flash-high')], deep: [] } } }), ...REPO.write }, { git: [...REPO.git, ...repoCommit] }),
    roots: ['runs'],
  }),
]

// A Pi refuter over a LiteLLM route: started directly with the prompt on stdin, its JSON event stream read as a critic's
// is, and logged as a pi launch. The fake `pi` keeps its argv and stdin beside the refuter's packet.
const PI = { family: 'zhipu', kind: 'pi', model: 'glm-5-3', provider: 'litellm-vibe-study' }
const piRouting = `${JSON.stringify({ clients: { 'pi-vibe-study-glm': PI }, stages: { spec: { base: [{ ...slot('vibe-mistral-large'), client: 'pi-vibe-study-glm' }], deep: [] } } })}\n`
const piSays = (structured, calls = []) =>
  `printf '%s\\n' ${q(
    [
      ...calls.map(([tool, args]) => ({ args, toolName: tool, type: 'tool_execution_start' })),
      { message: { content: [{ text: `\`\`\`json\n${JSON.stringify(structured)}\n\`\`\``, type: 'text' }], role: 'assistant', stopReason: 'stop', usage: { cost: { total: 0.001 }, input: 400, output: 40 } }, type: 'message_end' },
    ].map((event) => JSON.stringify(event)).join('\n'),
  )}\nexit 0`
const PI_CASES = {
  p1: piSays({ findings: [], no_findings_reason: 'The finding holds up.' }, [['read', { path: 'artifact/spec.md' }]]),
  p2: piSays({ findings: [objection('refute')] }),
  p3: piSays({ findings: [objection('refute')] }, [['read', { path: '../../../packet/artifact/spec.md' }]]),
}
const fakePi = (cases) =>
  `#!/bin/sh\nif [ "$1" = auth ]; then printf '%s\\n' '{"status":"ready"}'; exit 0; fi\nif [ "$1" = --offline ]; then printf '%s\\n' 'litellm-vibe-study glm-5-3 128K'; exit 0; fi\nprintf '%s\\n' "$@" > "$PWD/../argv.txt"\ncat > "$PWD/../stdin.txt"\ncase "$PWD" in\n${Object.entries(cases)
    .map(([id, body]) => `*/refute/${id}/packet)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const piRuns = Object.assign({}, ...Object.keys(PI_CASES).map((id) => run(`pi-${id}`, [finding(id)], { manifestExtra: { allowed_families: ['mistral', 'zhipu'] } })))
scenarios['refute-pi'] = Object.keys(PI_CASES).map((id, index) =>
  go(`pi-${id}`, accept(id), {
    ...(index === 0
      ? { write: { ...piRuns, 'bin/pi': fakePi(PI_CASES), [OVERRIDE]: piRouting, ...REPO.write }, exec: ['bin/pi'], shims: ['env', 'git', 'cat'], path: ['bin'], git: [...REPO.git, ...repoCommit] }
      : { path: ['bin'] }),
    roots: ['runs'],
  }),
)

// A step names its PATH: the sandbox's bin/ with the host's env and git linked in, so a CLI the host happens to have
// installed (vibe, agy) never changes what the step sees.
const hermetic = (step) => (step.path !== undefined ? step : { ...step, path: ['bin'], shims: [...new Set([...(step.shims ?? []), 'env', 'git'])] })
for (const [name, steps] of Object.entries(scenarios)) {
  scenarios[name] = steps.map(hermetic)
}
for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ assets: true, steps }, null, 2)}\n`)
}
console.log(`${Object.keys(scenarios).length} scenarios`)
