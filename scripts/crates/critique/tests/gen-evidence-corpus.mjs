#!/usr/bin/env node
// Writes the corpus the Rust test `evidence_corpus` replays: generated packets and findings, and what the node runner's
// evidence check (domain/evidence.ts), line references, declared checks and ticket and plan targets make of them.
//
//   node scripts/crates/critique/tests/gen-evidence-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so its pieces are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '1500', SEED = '5', OUT = join(here, 'fixtures', 'evidence-corpus.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-evidence-corpus-'))
const names = ['checkEvidence', 'indexPacket', 'lineReferences', 'declaredChecks', 'checkNamed', 'checkRunnable', 'withDeclaredSeverity', 'skippedChecks', 'planTarget', 'ticketTarget', 'planStepIds', 'diffShownLines', 'CRITIQUE_STAGES']
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { ${names.join(', ')} }\n`)
const node = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })

let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const int = (lo, hi) => lo + Math.floor(rnd() * (hi - lo + 1))
const chance = (p) => rnd() < p
const some = (make, max) => Array.from({ length: int(0, max) }, make)
const STAGES = node.CRITIQUE_STAGES
const CATEGORIES = ['unasked-question', 'contradiction', 'omission', 'invention', 'untestable', 'failure-mode', 'security', 'structure', 'bug', 'integration', 'drift', 'stale', 'undocumented-decision']
const WORDS = ['the', 'spec', 'retry', 'budget', 'is', 'unbounded', 'cohesive', 'leaky', 'clean', 'God class', 'observable', 'no rationale found', 'é', '→', 'a\tb', 'x y', ' ', 'line\nbreak', '  spaced  ']
const sentence = (n = 8) => Array.from({ length: int(1, n) }, () => pick(...WORDS)).join(pick(' ', ' ', '  ', '\n'))
const PATHS = ['src/a.ts', 'src/b/c.rs', 'README.md', 'docs/x.md', 'a.b/c-d.ts', 'Makefile', 'src/ü.ts']
const SPECIAL = ['ticket-1', '#12', 'T1', 'S01', 'S02', 'a.b']

const body = (n) => Array.from({ length: n }, (_, i) => `line ${i + 1} ${sentence(4)}`.replace(/\n/g, ' ')).join('\n') + pick('', '\n')
const hunk = (start, count) => `@@ -${start},${count} +${start},${count} @@\n${Array.from({ length: count }, (_, i) => `+added ${i}`).join('\n')}\n`
const diffFor = (paths) => paths.map((p) => `diff --git a/${p} b/${p}\n--- a/${p}\n+++ b/${p}\n${hunk(int(1, 30), int(1, 6))}${chance(0.4) ? hunk(int(40, 80), int(1, 3)) : ''}`).join('')
const removalDiff = (p) => `diff --git a/${p} b/${p}\n--- a/${p}\n+++ b/${p}\n@@ -5,2 +4,0 @@\n-a\n-b\n`
const deletedDiff = (p) => `diff --git a/${p} b/${p}\ndeleted file mode 100644\n--- a/${p}\n+++ /dev/null\n@@ -1,3 +0,0 @@\n-a\n-b\n-c\n`

const packetFile = () => {
  const kind = pick('artifact', 'artifact', 'reference', 'reference', 'tree', 'tree', 'diff', 'fact', 'history', 'check', 'spec')
  const path = pick(...PATHS)
  if (kind === 'diff') return { kind, path: 'diff.patch', text: chance(0.1) ? removalDiff(pick(...PATHS)) : chance(0.2) ? deletedDiff(pick(...PATHS)) : diffFor(some(() => pick(...PATHS), 3).concat([pick(...PATHS)])) }
  if (kind === 'check') return chance(0.7) ? { kind, path: 'check/plan.json', text: JSON.stringify({ steps: some(() => pick({ id: pick('S01', 'S02', 'S3') }, { id: 7 }, null, { x: 1 }), 4).concat([{ id: 'S01' }, { id: 'S02' }]) }) } : { kind, path: 'check/plan.json', text: pick('not json', '{"steps":3}', '[]', '') }
  const prefix = { artifact: 'artifact/', reference: 'references/', tree: 'tree/', fact: 'facts/', history: 'history/', spec: 'spec/' }[kind]
  const file = { kind, path: prefix + (kind === 'reference' && chance(0.3) ? pick('dependency-cruiser.json', '.eslintrc.json', 'layers.yml', 'layers.json') : path), text: body(int(0, 40)) }
  if (kind === 'tree' && chance(0.3)) {
    const last = int(1, 30)
    file.window = { last, lines: last + int(1, 40), ...(chance(0.5) ? { ranges: [{ first: 1, last: int(1, 10) }, { first: int(12, 20), last: int(21, 30) }] } : {}) }
  }
  return file
}

const quoteFrom = (packet) => {
  const files = packet.filter((f) => f.text.length > 0)
  if (files.length === 0 || chance(0.04)) return sentence(3)
  const f = pick(...files)
  const lines = f.text.split('\n')
  const a = int(0, lines.length - 1)
  const q = lines.slice(a, a + int(1, 2)).join(pick('\n', ' ', '  '))
  return chance(0.03) ? q + ' zz' : q
}
let PACKET = []
const heldCite = () => {
  const f = pick(...PACKET.filter((x) => x.path.startsWith('tree/') || x.path.startsWith('artifact/') || x.path === 'diff.patch'), { path: 'tree/src/a.ts', text: '' })
  const n = Math.max(1, f.text.split('\n').length)
  return `${f.path.startsWith('diff') ? 'tree/src/a.ts' : f.path}:${int(1, n)}${chance(0.3) ? `-${int(1, n)}` : ''}`
}
const cite = () => chance(0.5) ? heldCite() : `${pick('', './', 'tree/', 'a/', 'b/', 'artifact/', 'references/')}${pick(...PATHS)}:${int(0, 60)}${pick('', '', `-${int(1, 70)}`, ` – ${int(1, 70)}`, ` - ${int(1, 70)}`)}`
const trace = () => pick(`${cite()} then ${cite()}`, `see https://example.com:443/x and ${cite()}`, `10:30 ${cite()}`, `no citation`, `${cite()}`, 'file.ts:3\nfile.ts:4', 'v1.2:3 x', 'localhost:8080/x.ts:4')
const evidenceItem = (packet) => {
  const kind = pick('quote', 'quote', 'quote', 'input', 'failing-test', 'trace', 'trace')
  return { kind, value: kind === 'quote' ? quoteFrom(packet) : kind === 'trace' ? trace() : sentence(4) }
}
const location = () => pick(cite(), `${cite()}, ${cite()}`, 'S01: mixed-change', 'S02 no-rollback', 'S9: no-fake', 'ticket-1: too-big', '#12 no-seam', 'T1 uncovered-requirement', 'uncovered-requirement', 'S01', 'too-big', 'x S01x', 'Mixed-Change at s01', '(S01)#S02', 'S01/ no-fake-x', 'whole spec', `${pick(...SPECIAL)} ${pick('too-big', 'no-seam', 'untestable-criterion')}`)
const finding = (packet) => ({
  category: pick(...CATEGORIES), claim: sentence(10), evidence: some(() => evidenceItem(packet), 4), location: location(), severity: pick('critical', 'major', 'minor'),
  ...(chance(0.6) ? { suggestion: pick(sentence(6), 'say the observable', 'Observable: x') } : {}), ...(chance(0.5) ? { check: pick('inward-import', 'port-in-adapter', 'some-check', 'refute', 'downgrade', 'no-fake') } : {}),
})

const cases = []
for (let i = 0; i < Number(N); i++) {
  const packet = some(packetFile, 7)
  PACKET = packet
  const stage = pick(...STAGES)
  const f = finding(packet)
  const tickets = some(() => pick(...SPECIAL), 3)
  const role = chance(0.3) ? pick('adr-reviewer', 'decision-recoverer', 'bug-hunter') : undefined
  const index = node.indexPacket(packet, tickets)
  const result = node.checkEvidence(f, stage, index, role)
  cases.push({ kind: 'evidence', stage, ...(role === undefined ? {} : { role }), packet, tickets, finding: f, expected: result ?? null })
}
for (let i = 0; i < 700; i++) {
  const packet = [
    { kind: 'artifact', path: 'artifact/spec.md', text: body(8) }, { kind: 'artifact', path: 'artifact/spec2.md', text: body(8) }, { kind: 'reference', path: pick('references/adr.md', 'references/layers.yml'), text: body(8) },
    ...(chance(0.6) ? [{ kind: 'reference', path: 'references/adr2.md', text: body(6) }] : []), ...(chance(0.5) ? [{ kind: 'fact', path: 'facts/INDEX.md', text: body(5) }] : []), ...(chance(0.5) ? [{ kind: 'history', path: 'history/log.md', text: body(5) }] : []),
    { kind: 'check', path: 'check/plan.json', text: JSON.stringify({ steps: [{ id: 'S01' }, { id: 'S02' }] }) }, ...(chance(0.6) ? [{ kind: 'tree', path: 'tree/src/a.ts', text: body(20) }, { kind: 'tree', path: 'tree/src/b/c.rs', text: body(20) }] : []),
  ]
  PACKET = packet
  const quotes = some(() => { const f = pick(...packet.filter((x) => x.kind !== 'check')); const lines = f.text.split('\n'); return { kind: 'quote', value: lines[int(0, Math.min(7, lines.length - 1))] } }, 3)
  const extra = some(() => pick({ kind: 'input', value: 'x' }, { kind: 'failing-test', value: 't' }, { kind: 'trace', value: `tree/src/a.ts:${int(1, 22)} and tree/src/b/c.rs:${int(1, 20)}` }, { kind: 'trace', value: 'tree/src/a.ts:3' }, { kind: 'trace', value: 'no cite' }), 2)
  const stage = pick('spec', 'plan', 'adr-review', 'decisions', 'tickets', 'architecture-review', 'decision-recovery', 'decision-recovery', 'decision-recovery', 'feature-review')
  const f = { category: chance(0.6) ? 'undocumented-decision' : pick(...CATEGORIES), claim: pick('plain claim', 'leaky design', 'no rationale found here', 'No Rationale Found', sentence(5)), evidence: [...(chance(0.4) ? [] : quotes), ...extra, ...(chance(0.5) ? [{ kind: 'trace', value: 'tree/src/a.ts:2 then tree/src/b/c.rs:3' }] : [])], location: pick('S01: mixed-change', 'S02 no-fake', 'S01', 'tree/src/a.ts:4', 'T1 too-big', 'T1', 'uncovered-requirement', 'x'), severity: 'major', ...(chance(0.6) ? { suggestion: pick('the observable is x', 'none') } : {}), ...(chance(0.4) ? { check: pick('inward-import', 'rule-in-adapter', 'z') } : {}) }
  const tickets = pick([], ['T1'], ['T1', 'T2'])
  const role = chance(0.2) ? pick('adr-reviewer', 'decision-recoverer') : undefined
  const index = node.indexPacket(packet, tickets)
  cases.push({ kind: 'evidence', stage, ...(role === undefined ? {} : { role }), packet, tickets, finding: f, expected: node.checkEvidence(f, stage, index, role) ?? null })
}
for (let i = 0; i < 300; i++) {
  const packet = [{ kind: 'artifact', path: 'artifact/a.md', text: 'alpha one\nshared line\nalpha two' }, { kind: 'artifact', path: 'artifact/b.md', text: 'beta one\nshared line\nbeta two' }, { kind: 'reference', path: 'references/r.md', text: 'ref one\nbeta two' }, ...(chance(0.5) ? [{ kind: 'tree', path: 'tree/x.ts', text: 'x' }] : [])]
  const stage = pick('adr-review', 'spec', 'decisions')
  const f = { category: pick('contradiction', 'contradiction', 'omission', 'invention', 'unasked-question', 'drift', 'bug'), claim: 'c', evidence: some(() => pick({ kind: 'quote', value: pick('alpha one', 'beta one', 'shared line', 'ref one', 'beta two') }, { kind: 'input', value: 'i' }), 3), location: 'x', severity: 'minor' }
  cases.push({ kind: 'evidence', stage, packet, tickets: [], finding: f, expected: node.checkEvidence(f, stage, node.indexPacket(packet, []), undefined) ?? null })
}
for (let i = 0; i < 100; i++) {
  const packet = [{ kind: 'tree', path: 'tree/src/a.ts', text: body(30), window: { last: 20, lines: 60, ranges: [{ first: 1, last: 5 }, { first: 12, last: 20 }] } }]
  const f = { category: 'bug', claim: 'c', evidence: [], location: `src/a.ts:${int(1, 25)}-${int(1, 25)}`, severity: 'minor' }
  cases.push({ kind: 'evidence', stage: 'feature-review', packet, tickets: [], finding: f, expected: node.checkEvidence(f, 'feature-review', node.indexPacket(packet, []), undefined) ?? null })
}
for (let i = 0; i < 400; i++) {
  const text = Array.from({ length: int(0, 3) }, () => pick(trace(), location(), 'x: 1', 'https://a.b:80/c.ts:9', 'a.ts:1-2-3', 'a/b.ts:12 – 14', '1:2', '.:3', '..:3', './a/b:4')).join(pick(' ', '\n', '; '))
  cases.push({ kind: 'refs', text, expected: node.lineReferences(text) })
}
const bullet = () => `${pick('-', '-', '*', ' -', '- ')}${pick(' ', ' ')}\`${pick('no-rollback', 'a1', 'a-b-c', 'Bad', 'x--y', 'dup', 'tree-check', 'ok-1')}\`${pick('', ' (critical)', ' (major)', ' (minor)', ' (nope)', ' (critical')}${pick(':', ':', '')}${pick('', ' Needs `tree/`.', ' Needs `tree/`', ' text')} body`
for (let i = 0; i < 300; i++) {
  const texts = some(() => some(() => pick(bullet(), 'prose', ''), 5).join(pick('\n', '\r\n', '\n\n')), 3)
  if (chance(0.4)) texts.unshift('- `tree-check` (critical): Needs `tree/`. reads code')
  const declared = node.declaredChecks(texts)
  const probe = { check: pick(undefined, 'no-rollback', 'dup', 'zzz', 'Bad', 'ok-1', 'tree-check'), severity: pick('critical', 'minor') }
  const full = { id: 'f1', category: 'bug', claim: 'c', evidence: [], location: 'l', severity: probe.severity, critic_finding_id: 'x', raised_by: ['c1'], ...(probe.check === undefined ? {} : { check: probe.check }) }
  cases.push({
    kind: 'checks', texts, probe, tree: chance(0.5), stage: pick(...STAGES), declared,
    named: node.checkNamed(probe, declared) ?? null, runnable: node.checkRunnable(probe, declared, false) ?? null, runnableTree: node.checkRunnable(probe, declared, true) ?? null,
    severity: node.withDeclaredSeverity(full, declared),
    skipped: ['adr-review', 'spec', 'plan'].map((stage) => [stage, node.skippedChecks(stage, { tree: false }, [{ from: 'role a', checks: declared }, { from: 'lens b', checks: declared.slice(1) }])]),
    skippedTree: node.skippedChecks('adr-review', { tree: true }, [{ from: 'role a', checks: declared }]),
  })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases, ${cases.filter((c) => c.kind === 'evidence' && c.expected).length} with a verdict`)
