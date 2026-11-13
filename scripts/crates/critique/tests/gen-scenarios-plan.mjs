#!/usr/bin/env node
// Writes the scenario.json files of check-plan (the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, readFileSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, 'fixtures')
const shipped = JSON.parse(readFileSync(join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'routing.json'), 'utf8'))
const step = (argv, extra = {}) => ({ argv, ...extra })
const cmd = (name, input, extra) => step([name, '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const plan = (input, extra) => cmd('check-plan', input, extra)
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const R = (dir) => `@SANDBOX@/repos/${dir}`
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]
const commit = (dir) => [['-C', `repos/${dir}`, 'add', '-A'], ['-C', `repos/${dir}`, 'commit', '-q', '-m', 'x']]
const files = (dir, text, { fresh = false, commits = true, ...extra } = {}) => ({
  write: Object.fromEntries(Object.entries(text).map(([path, body]) => [`repos/${dir}/${path}`, body])),
  git: [...(fresh ? init(dir) : []), ...(commits ? commit(dir) : [])],
  ...extra,
})
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const override = (text, extra = {}) => ({ write: { [OVERRIDE]: typeof text === 'string' ? text : JSON.stringify(text) }, ...extra })
const scenarios = {}

const stepText = (id, fields = {}, title = 'Do the thing') => {
  const base = {
    Kind: 'tidy', Technique: 'extract function', Anchor: '`src/a.ts#fooBar`', Touches: 'src/a.ts', 'Safety net': 'the unit tests of a',
    'Main stays green because': 'nothing observable changes', Rollback: 'revert the commit', 'Transitional element': 'none', 'Blocked by': 'none', 'Done when': 'the tests pass',
  }
  const all = { ...base, ...fields }
  return `### ${id}: ${title}\n${Object.entries(all).filter(([, value]) => value !== null).map(([name, value]) => `- **${name}**: ${value}`).join('\n')}\n`
}
const doc = ({ goal = 'Tidy the parser. Behaviour does not change.', steps = [stepText('S01')], extra = '' } = {}) =>
  `# Plan\n\n## Goal\n${goal}\n\n## Target\nThe parser has its own module.\n\n## Safety net\nThe unit tests.\n\n## Steps\n\n${steps.join('\n')}\n## Not doing\nNothing else.\n\n## Open decisions\nNone.\n${extra}`

const adr = (n, body, status = 'Accepted') => `# ${n}. Decision ${n}\n\nStatus: ${status}\n\n${body}\n`
const repoFiles = files('p', {
  'docs/adr/0001-first.md': adr(1, 'First.'),
  'docs/adr/0002-second.md': adr(2, 'Second.', 'Superseded by [0003](0003-third.md)'),
  'docs/adr/0003-third.md': adr(3, 'Replaces ADR-0002.'),
  'docs/adr/README.md': '# Index\n',
  'src/a.ts': 'export function fooBar() {}\n',
  'plans/ok.md': doc({ steps: [stepText('S01', { Anchor: 'ADR-0001' }), stepText('S02', { Anchor: 'ADR 3', 'Blocked by': 'S01' })] }),
  'plans/absent.md': doc({ steps: [stepText('S01', { Anchor: 'ADR-0001, adr-9, ADR-0002, `src/a.ts`' })] }),
  'plans/dup.md': doc({ steps: [stepText('S01', { Anchor: 'ADR-0002' }), stepText('S02', { Anchor: 'ADR-0003 and ADR-0003' })] }),
}, { fresh: true })

scenarios['plan-basic'] = [
  plan({ plan: 'plans/ok.md', repo: R('p') }, repoFiles),
  plan({ plan: 'plans/absent.md', repo: R('p') }),
  plan({ plan: 'plans/dup.md', repo: R('p') }),
  plan({ plan: `${R('p')}/plans/ok.md`, repo: R('p') }),
  plan({ plan: 'plans/ok.md', repo: R('p/docs') }),
  plan({ plan: '../p/plans/ok.md', repo: R('p/plans') }),
  plan({ plan: 'plans/ok.md' }, { write: { 'work/plans/ok.md': doc() } }),
  plan({ plan: 'plans/ok.md' }),
  plan({ plan: `${R('p')}/plans/ok.md` }),
  step(['check-plan', '--json', JSON.stringify({ plan: 'plans/absent.md', repo: R('p') }), '--strict']),
  step(['check-plan', '--json', JSON.stringify({ plan: 'plans/ok.md', repo: R('p') }), '--strict']),
]

scenarios['plan-refusals'] = [
  plan({}), plan({ plan: '' }), plan({ plan: 5 }), plan({ plan: 'x', repo: '' }), plan({ plan: 'x', repo: 5 }), plan([]), plan('null'), plan({ plan: 'x', repo: null }),
  plan({ plan: 'x', repo: R('none') }, { git: init('none').slice(0, 1) }),
  plan({ plan: 'x', repo: R('nedap') }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  plan({ plan: 'x', repo: R('gone') }),
  plan({ plan: 'missing.md', repo: R('bare') }, { git: init('bare') }),
  plan({ plan: 'missing.md' }),
  plan({ plan: 'dir' }, { mkdir: ['work/dir'] }),
  plan({ plan: 'extra.md', ignored: true }, { write: { 'work/extra.md': doc() } }),
  plan({ plan: 'p.md' }, { write: { 'work/p.md': '' } }),
  step(['check-plan', '--json', JSON.stringify({ plan: 'x', repo: R('none') }), '--strict']),
]

const check = (name, text, extra = {}) => plan({ plan: `${name}.md` }, { ...extra, write: { [`work/${name}.md`]: text, ...extra.write } })
scenarios['plan-structure'] = [
  check('empty', ''),
  check('sections', '## Goal\nSomething.\n\n## goal\nTwice.\n\n## Steps\n\n## Appendix\nnotes\n\n## STEPS\nagain\n'),
  check('nosteps', '## Goal\nx\n## Target\nx\n## Safety net\nx\n## Steps\nnot a step\n## Not doing\nx\n## Open decisions\nx\n'),
  check('headings', doc({ steps: ['### S1 Missing colon\n', '### Step: nope\n', stepText('S02'), stepText('S02', {}, 'Again'), stepText('S03', {}, '')] })),
  check('fields', doc({ steps: [stepText('S01', { Kind: null, Anchor: null, 'Done when': null }), '### S02: Dupes\n- **Kind**: tidy\n- **Kind**: tidy\n- **Technique**: t\nprose line\n- **anchor:** `src/a.ts`\n- **Touches**: a,\n  b\n  - c\n'] })),
  check('fences', doc({ extra: '\n```\n## Appendix\n### S09: hidden\n```\n~~~\n## Other\n~~~\n' })),
  check('crlf', doc().replaceAll('\n', '\r\n')),
  check('kinds', doc({ steps: [stepText('S01', { Kind: 'tidy, structure' }), stepText('S02', { Kind: 'tidy/behaviour' }), stepText('S03', { Kind: 'bogus' }), stepText('S04', { Kind: 'characterize and tidy' }), stepText('S05', { Kind: 'Tidy' })] })),
  check('blocked', doc({ steps: [stepText('S01', { 'Blocked by': 'S09' }), stepText('S02', { 'Blocked by': 'the database' }), stepText('S03', { 'Blocked by': 'N/A' }), stepText('S04', { 'Blocked by': 'S01, s02 and S03' }), stepText('S05', { 'Blocked by': 'Nothing.' })] })),
  check('cycles', doc({ steps: [stepText('S01', { 'Blocked by': 'S01' }), stepText('S02', { 'Blocked by': 'S03' }), stepText('S03', { 'Blocked by': 'S02' }), stepText('S04', { 'Blocked by': 'S02' }), stepText('S05', { 'Blocked by': 'S06' }), stepText('S06', { 'Blocked by': 'S07' }), stepText('S07', { 'Blocked by': 'S05, S01' })] })),
  check('transitional', doc({ steps: [stepText('S01', { 'Transitional element': 'a shim, removed in S03' }), stepText('S02', { 'Transitional element': 'a flag' }), stepText('S03', { Kind: 'remove-transitional' }), stepText('S04', { 'Transitional element': 'old table, see S09' })] })),
  check('anchors', doc({ steps: [stepText('S01', { Anchor: 'somewhere' }), stepText('S02', { Anchor: 'prerequisite-of: S01' }), stepText('S03', { Anchor: 'prerequisite-of:S09, `src/b.ts#x`' }), stepText('S04', { Anchor: '`notapath`, `src/a.ts#fooBar` and ADR-0001' }), stepText('S05', { Anchor: 'ADR-12, adr 3, ADR_4, ADR-00001' }), stepText('S06', { Anchor: '`https://example.com/x/y.ts`' })] })),
]

scenarios['plan-rules'] = [
  check('goals', doc({ goal: 'Behaviour changes for callers.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('goal-denied', doc({ goal: 'Behaviour does not change. Nothing alters behaviour here.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('goal-never', doc({ goal: 'The behaviour will never change.\nBut we change the behaviour of the parser.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('goal-mixed', doc({ goal: 'No change to the log. The behaviour of the cache differs.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('goal-without', doc({ goal: 'We alter things without a behaviour change.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('goal-not', doc({ goal: 'Behaviour is not changed anywhere.', steps: [stepText('S01', { Kind: 'behaviour' })] })),
  check('untested', doc({ steps: [stepText('S01', { Kind: 'structure', 'Safety net': 'none' }), stepText('S02', { Kind: 'characterize' }), stepText('S03', { Kind: 'structure', 'Safety net': 'untested code', 'Blocked by': 'S02' }), stepText('S04', { Kind: 'structure', 'Safety net': 'No tests', 'Blocked by': 'S03' }), stepText('S05', { Kind: 'structure', 'Safety net': 'Nothing covers it', 'Blocked by': 'S09' })] })),
  check('size', doc({ steps: [stepText('S01', { Touches: 'a, b, c, d, e, f', 'Done when': 'a; b; c; d; e; f; g; h; i' }), stepText('S02', { Touches: '\n  - a\n  - b\n  - c\n  - d\n  - e\n  - f\n  - g' })] })),
  check('size-sub', '## Goal\nx\n## Target\nx\n## Safety net\nx\n## Steps\n### S01: Sub\n- **Kind**: tidy\n- **Technique**: t\n- **Anchor**: ADR-1\n- **Touches**: a, b,, c ; d\n  - e\n  - f\n  - g\n  - h\n  - i\n  - j\n- **Safety net**: s\n- **Main stays green because**: m\n- **Rollback**: r\n- **Transitional element**: none\n- **Blocked by**: none\n- **Done when**: x\n  - 1\n  - 2\n  - 3\n  - 4\n  - 5\n  - 6\n  - 7\n  - 8\n  - 9\n## Not doing\nx\n## Open decisions\nx\n'),
  check('expand', doc({ steps: [stepText('S01', { Kind: 'expand' }), stepText('S02', { Kind: 'migrate' })] })),
  check('expand-ok', doc({ steps: [stepText('S01', { Kind: 'expand' }), stepText('S02', { Kind: 'migrate' }), stepText('S03', { Kind: 'contract' })] })),
  check('expand-contract', doc({ steps: [stepText('S01', { Kind: 'expand' }), stepText('S02', { Kind: 'contract' })] })),
  check('paths', doc({ goal: 'Move src/parser/index.ts and see https://example.com/a/b/c.ts, plus lib.rs.', steps: [stepText('S01', { Technique: 'move docs/guide/intro.md and docs/x.md', Rollback: 'revert `a/b/c`', 'Done when': 'x.json parses; no a.b.c', 'Main stays green because': 'foo.test.ts and @scope/pkg/index.mjs and .hidden/file.yml' })], extra: '' }).replace('The parser has its own module.', 'The parser in src/parser/mod.ts.').replace('The unit tests.', 'Tests in tests/a/b.rs and a_b-c.sh.')),
  check('paths-prose', '## Goal\nx\n## Target\nx\n## Safety net\nx\n## Steps\n### S01: Prose\nSee src/a.ts for details.\n- **Kind**: tidy\n- **Technique**: t\n  continued in lib/x.ts\n- **Anchor**: `src/a.ts`\n  - `src/a.ts` also\n- **Touches**: src/a.ts\n- **Safety net**: s\n- **Main stays green because**: m\n- **Rollback**: r\n- **Transitional element**: none\n- **Blocked by**: none\n- **Done when**: x\n## Not doing\nx\n## Open decisions\nx\n'),
  check('stored', doc({ steps: [stepText('S01', { Kind: 'migrate', Touches: 'the users table' }), stepText('S02', { Kind: 'contract', Technique: 'drop the old column' }), stepText('S03', { Kind: 'migrate', Touches: 'config' }), stepText('S04', { Kind: 'tidy', Touches: 'schema' }), stepText('S05', { Kind: 'contract', Touches: '\n  - the database' })] })),
  check('many', doc({ steps: Array.from({ length: 12 }, (_, index) => stepText(`S${String(index + 1).padStart(2, '0')}`, { 'Blocked by': index === 0 ? 'none' : `S${String(index).padStart(2, '0')}` })) })),
  check('unicode', doc({ goal: 'Änderung: behaviour changes — ünïcode   line sep.', steps: [stepText('S01', { Technique: 'naïve extract — ok', Kind: 'behaviour' })] })),
]

const patch = (value) => override(JSON.stringify(value))
const sizePatch = (max_modules, max_criteria) => ({ stages: { tickets: { ticket_size: { max_modules, max_criteria } } } })
const sized = doc({ steps: [stepText('S01', { Touches: 'a, b, c', 'Done when': 'a; b; c' })] })
scenarios['plan-routing'] = [
  check('sized', sized),
  check('sized', sized, patch(sizePatch(2, 2))),
  check('sized', sized, patch(sizePatch(3, 3))),
  check('sized', sized, patch({ stages: { tickets: { ticket_size: null } } })),
  check('sized', sized, patch({})),
  check('sized', sized, override('not json')),
  check('sized', sized, override('')),
  check('sized', sized, override('[1]')),
  check('sized', sized, override('null')),
  check('sized', sized, override('"x"')),
  check('sized', sized, override('{"stages":')),
  check('sized', sized, override('{"version":2}')),
  check('sized', sized, override({ version: null })),
  check('sized', sized, override({ stages: { tickets: null } })),
  check('sized', sized, override({ stages: null })),
  check('sized', sized, override({ clients: null })),
  check('sized', sized, override({ stages: { tickets: { ticket_size: { max_modules: 'x' } } } })),
  check('sized', sized, override({ stages: { tickets: { ticket_size: 5 } } })),
  check('sized', sized, { remove: [OVERRIDE], mkdir: [OVERRIDE] }),
  check('sized', sized, { remove: [OVERRIDE] }),
  check('sized', sized, { env: { HOME: null } }),
]

const slot = (client) => ({ client, role: 'r', lenses: [], max_price_usd: 1, time_limit_s: 1 })
const vibe = { kind: 'vibe', binary: 'vibe', agent: 'a', model: 'm', family: 'mistral' }
const routings = {
  'defaults': { antigravity_quota: null, search: null, packet: null, fallbacks: null, stats: null, gate: null, triage: null, cold_read: null, stages: { tickets: { depth_signals: null, ticket_size: null } } },
  'gate-default': { gate: { merge_commits: null } },
  'new-client': { clients: { 'vibe-new': vibe } },
  'new-client-full': { clients: { 'vibe-new': { ...vibe, prices: { input: 1, cached_input: 0, output: 1 }, gateway: { client: 'vibe', model: 'x' } } } },
  'client-kind': { clients: { 'vibe-new': { ...vibe, kind: 'bogus' } } },
  'client-family': { clients: { 'codex-work': { family: 'mistral' } } },
  'client-claude-family': { clients: { 'claude-work': { family: 'openai' } } },
  'client-agy-model': { clients: { 'agy-gemini-flash-high': { model: 'claude-x' } } },
  'client-vibe-family': { clients: { 'vibe-glm': { family: 'anthropic' } } },
  'client-gateway': { clients: { 'vibe-glm': { gateway: { client: 'agy' } } } },
  'price-input': { clients: { 'vibe-glm': { prices: { input: 0 } } } },
  'price-cached': { clients: { 'vibe-glm': { prices: { cached_input: -1 } } } },
  'depth-bogus': { stages: { spec: { depth_signals: { thresholds: { bogus: 1 } } } } },
  'depth-none': { stages: { 'feature-review': { depth_signals: { thresholds: { bogus: 1 } } } } },
  'depth-unknown-stage': { stages: { extra: { base: [], distinct_families: true, depth_signals: { thresholds: { x: 1 } } } } },
  'depth-extra-ok': { stages: { extra: { base: [slot('vibe-glm')], deep: [slot('vibe-glm')], adr: [slot('vibe-glm')], work: [slot('claude-work')], distinct_families: false } } },
  'base-work': { stages: { spec: { base: [slot('codex-work')] } } },
  'adr-work': { stages: { spec: { adr: [slot('claude-work')] } } },
  'deep-work': { stages: { spec: { deep: [slot('codex-work')] } } },
  'work-open': { stages: { spec: { work: [slot('vibe-glm')] } } },
  'work-unknown': { stages: { spec: { base: [slot('unknown-client')] } } },
  'slot-shape': { stages: { spec: { base: [{ client: 'vibe-glm' }] } } },
  'fallback-work': { fallbacks: { 'codex-work': ['vibe-glm'] } },
  'fallback-to-work': { fallbacks: { 'vibe-glm': ['claude-work'] } },
  'fallback-shape': { fallbacks: { 'vibe-glm': 'x' } },
  'triage-agy': { triage: { model: { client: 'agy-gemini-flash-high' } } },
  'triage-unknown': { triage: { model: { client: 'nope' } } },
  'triage-shape': { triage: { labels: 'x' } },
  'cold-codex': { cold_read: { client: 'codex-work' } },
  'cold-agy': { cold_read: { client: 'agy-gemini-flash-high' } },
  'cold-unknown': { cold_read: { client: 'nope' } },
  'stats-zero': { stats: { removal_features: 0 } },
  'quota-shape': { antigravity_quota: { daily_cap: 'x' } },
  'quota-stage': { antigravity_quota: { reserved_stages: ['nope'] } },
  'quota-ok': { antigravity_quota: { daily_cap: 3, reserve_threshold: 1, reserved_stages: ['plan'] } },
  'search-zero': { search: { max_bytes: 0 } },
  'packet-shape': { packet: { cap_bytes: 'x' } },
  'packet-exclude': { packet: { exclude: { vendored: 'x' } } },
  'gate-zero': { gate: { merge_commits: 0 } },
  'gate-shape': { gate: { feature_branches: 5 } },
  'stage-distinct': { stages: { spec: { distinct_families: 'yes' } } },
  'stage-base': { stages: { spec: { base: 'x' } } },
  'stage-ticket-size': { stages: { spec: { ticket_size: { max_modules: 1, max_criteria: 2 } } } },
  'fallbacks-clients-shape': { clients: { 'vibe-glm': 5 } },
}
scenarios['plan-routing-readers'] = Object.values(routings).map((value) => check('sized', sized, patch(value)))

const bare = (inner) => `## Goal\nx\n## Target\n${inner}\n## Safety net\nx\n## Steps\n### S01: S\n- **Kind**: tidy\n## Not doing\nx\n## Open decisions\nx\n`
const edgy = (name, text) => check(name, text)
scenarios['plan-edges'] = [
  edgy('heading-blank', '## Goal\nx\n## Steps\n###S01: no blank\n### S02: ok\n### S2:\n###  S03 :  spaced  \n### S04: a\u2028b\n### S05: tab\t\n### Sx: letter\n### S: none\n'),
  edgy('field-forms', '## Steps\n### S01: F\n-**Kind**: tidy\n- **Kind: tidy\n- **Kind**:tidy\n- **Kind**: ti\u2028dy\n- **Kind** :  tidy \n-  **Done when:** x\n- **Touches**: a\n  -x\n  - \n  - y\u2028z\n  continued\n   - deep\n\u00a0nbsp\n'),
  edgy('ids', '## Steps\n### S01: A\n- **Blocked by**: S and S1 and Sx and S1x and xS2\n- **Kind**: structure\n- **Safety net**: none\n### S02: B\n- **Kind**: tidy\n- **Blocked by**: S01\n### S03: C\n- **Kind**: tidy\n- **Blocked by**: S01\n### S04: D\n- **Kind**: structure\n- **Safety net**: untested\n- **Blocked by**: S02, S03\n'),
  edgy('anchor-spans', '## Steps\n### S01: A\n- **Anchor**: `` and `src/a.ts\n### S02: B\n- **Anchor**: ``, `a/b.ts`, ` `, `x\n### S03: C\n- **Anchor**: prerequisite-of: foo and prerequisite-of:S and PREREQUISITE-OF:s7 and prerequisite-of:S1x and xprerequisite-of:S1\n'),
  edgy('urls', bare('See https://example.com/a/b/c.ts then http://x.y/z more and ftp:// empty and a+b.c-d://q/r.ts and ://x and 1http://a/b.c')),
  edgy('paths-1', bare('a/b. a/b.1 a/b.x a/b.abcdefg a/b.abcdef a/b.abcde a/b/c/ a/b/ a/b/c d/e.f/g @s/p.ts x.ts .y.ts z_.json q-w.yml ../a/b ./c/d.rs é/ü.md a b/c.sh, w/x.y.z, v/u.t8')),
  edgy('paths-2', bare('lib.rs lib.rsx LIB.RS lib.R foo.tsx.bak a-b.md a_b.md _a.md 9a.md ab.mdd ab.md. x/./y a//b //c/d e/f/ g/h//i')),
  edgy('anchor-solo', '## Steps\n### S01: A\n- **Anchor**: `abc\n'),
  edgy('goal-near', '## Goal\nBehaviourx changes. Behaviors differ. Behaviou changes. Misbehaviour changes. Behaviour\nx\n## Steps\n### S01: B\n- **Kind**: behaviour\n'),
  edgy('goal-us', '## Goal\nThe behavior changes. The behavior is not changed. Behaviours differ. Behavioural changes. behaviour\nnot changed\n## Steps\n### S01: B\n- **Kind**: behaviour\n'),
  edgy('goal-1', '## Goal\nBehaviour never changes here, nothing alters. Not a thing alters behaviour. No change to behaviour is planned. Behaviour: is not (really) changed. Behaviour was not at all altered. Behaviour is never differing.\n## Steps\n### S01: B\n- **Kind**: behaviour\n'),
  edgy('goal-2', '## Goal\nWe change the behaviour of x, and never touch it. Without changes, behaviour not altered. This will alter the behaviour not at all. It does not change behaviour. A behaviour that did not change.\n## Steps\n### S01: B\n- **Kind**: behaviour\n'),
]
void shipped

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
