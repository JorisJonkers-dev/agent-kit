#!/usr/bin/env node
// Writes the corpus the Rust test `verdicts` replays: replies, finding objects, verdict objects and runs, and what the
// node runner's reader and verdict rules make of them.
//
//   node scripts/crates/critique/tests/gen-verdicts-corpus.mjs [count] [seed] [out.json]
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const shipped = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique')
const [N = '600', SEED = '5', OUT = join(here, 'fixtures', 'verdicts.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(join(shipped, 'critique.mjs'), 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-verdicts-corpus-'))
writeFileSync(
  join(scratch, 'runner.mjs'),
  `${source.slice(0, source.indexOf(entry))}\nexport { parseFindings, readRunFinding, readVerdict, JsonFields, countVerdicts, secondRoundDue, stageVerdictError, candidateQuestions, severityOverrideProblem, openCriticalQuestions, verdictProblem, unfixedCritical }\n`,
)
const R = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })

let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const STAGES = ['decisions', 'spec', 'tickets', 'ticket-review', 'feature-review', 'adr-review', 'decision-recovery', 'architecture-review', 'plan']
const SEV = ['critical', 'major', 'minor']
const CAT = ['bug', 'omission', 'undocumented-decision', 'security', 'drift', 'nope']
const CODES = ['fabricated-evidence', 'no-evidence', 'no-check', 'unknown-check', 'single-site', 'bogus', 'no-plan-anchor']
const SLUG = ['test-fails', 'trace-walked', 'refuted', 'cross-ticket-trace', 'test-fails', 'cross-ticket-trace', 'trace-walked', 'refuted', 'Bad_Slug', '', 'a'.repeat(41), 'a'.repeat(40), '9x', 'ok-1']

// A value that is usually right and sometimes wrong, so every reader meets both.
let strict = false
const maybe = (good, bad) => (strict || rnd() < 0.985 ? good : bad)
const WRONG = () => pick(1, null, true, [], {}, '', '  ')
const evidence = () => ({ kind: maybe(pick('quote', 'input', 'failing-test', 'trace'), 'x'), value: maybe('v', WRONG()), ...(rnd() < 0.3 ? { file: maybe('tree/a.ts', WRONG()) } : {}) })
const body = () => ({
  severity: maybe(pick(...SEV), 'huge'), category: maybe(pick(...CAT.slice(0, 5)), 'weird'), location: maybe('spec#1', WRONG()), claim: maybe('c', WRONG()),
  evidence: maybe(Array.from({ length: Math.floor(rnd() * 3) }, evidence), WRONG()),
  ...(rnd() < 0.3 ? { suggestion: maybe(pick('s', 's', ''), 4) } : {}), ...(rnd() < 0.4 ? { check: maybe(pick('c1', 'c1', '  ', '', ' x '), 7) } : {}),
})
const criticFinding = (i) => ({ id: maybe(`f${i}`, WRONG()), ...body() })
const preVerdict = () => {
  if (!strict && rnd() < 0.1) return WRONG()
  const verdict = maybe(pick('rejected', 'hint'), 'maybe')
  const good = verdict === 'rejected' ? pick('fabricated-evidence', 'single-site') : pick('no-evidence', 'no-check', 'unknown-check', 'no-plan-anchor')
  return { verdict, reason_code: maybe(good, pick(...CODES)), reason: maybe('r', WRONG()) }
}
const runFinding = (i) => ({
  ...criticFinding(i), critic_finding_id: maybe('c-1', WRONG()), raised_by: maybe(['a', 'b'].slice(0, 1 + Math.floor(rnd() * 2)), WRONG()),
  ...(rnd() < 0.4 ? { declared_severity: maybe(pick(...SEV), 'x') } : {}),
  ...(rnd() < 0.5 ? { pre_verdict: preVerdict() } : {}),
})
const verdict = (id) => ({
  finding_id: id, verdict: maybe(pick('accepted', 'rejected', 'needs-human', 'hint'), 'later'), ...(rnd() < 0.6 ? { reason: pick('why', 'why', 'why', '', 3) } : {}),
  ...(rnd() < 0.6 ? { reason_code: pick(...SLUG) } : {}), ...(rnd() < 0.3 ? { answered: pick(true, true, false, "yes") } : {}), ...(rnd() < 0.3 ? { follow_up: pick('#9', '', 4) } : {}),
  ...(rnd() < 0.3 ? { trace_tickets: pick(['t/1.md', 't/2.md'], ['t/1.md', 't/1.md'], ['t/1.md'], ['zzz', 't/1.md'], 'x', []) } : {}),
  ...(rnd() < 0.3 ? { regression_test: pick('a.test.ts', '', 1) } : {}), ...(rnd() < 0.3 ? { fixed: pick(true, false, false, 0) } : {}),
  ...(rnd() < 0.3 ? { severity: pick(...SEV, 'huge') } : {}), ...(rnd() < 0.3 ? { severity_reason: pick(...SLUG) } : {}),
})
const settle = (fn) => { try { return { ok: fn() } } catch (error) { return { error: `${error.name}: ${error.message}` } } }
const parse = (value) => settle(() => R.readVerdict(R.JsonFields.of(value, '')))

const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('reply', 'reply', 'finding', 'verdict', 'verdict', 'rules', 'run')
  if (kind === 'reply') {
    const object = { findings: maybe(Array.from({ length: Math.floor(rnd() * 3) }, (_, n) => criticFinding(n)), WRONG()), ...(rnd() < 0.5 ? { no_findings_reason: pick('none', '  ', '', 4, ' trimmed ') } : {}) }
    const text = pick('', 'no block', `x\n\`\`\`json\n${JSON.stringify(object)}\n\`\`\`\n`, `\`\`\`json \r\n${JSON.stringify(object, null, 1)}\`\`\``, `\`\`\`json\n{"findings": [\n\`\`\``, `\`\`\`json\n${JSON.stringify(rnd() < 0.5 ? [] : 3)}\n\`\`\``, `\`\`\`json\n${JSON.stringify(object)}\n\`\`\`\n\`\`\`json\n{"findings":[],"no_findings_reason":"later"}\n\`\`\``)
    cases.push({ kind: 'reply', text, expected: R.parseFindings(text) })
  } else if (kind === 'finding') {
    const value = rnd() < 0.95 ? runFinding(i) : WRONG()
    cases.push({ kind: 'finding', value, expected: settle(() => R.readRunFinding(R.JsonFields.of(value, ''))) })
  } else if (kind === 'verdict') {
    const value = rnd() < 0.95 ? verdict(`f${i % 3}`) : WRONG()
    cases.push({ kind: 'verdict', value, expected: parse(value) })
  } else if (kind === 'rules') {
    const stage = pick(...STAGES)
    const parsed = parse(verdict('f1'))
    const v = parsed.ok ?? { finding_id: 'f1', verdict: 'accepted' }
    const files = Array.from({ length: Math.floor(rnd() * 4) }, (_, n) => ({ kind: pick('ticket', 'ticket', 'spec'), path: pick('t/1.md', 't/2.md', 'a') + (n > 2 ? 'x' : ''), source: 's', sha256: 'h' }))
    strict = true
    const finding = { ...runFinding(1), id: 'f1', severity: pick(...SEV), ...(rnd() < 0.5 ? { declared_severity: pick(...SEV) } : {}) }
    const category = pick(...CAT, undefined)
    strict = false
    cases.push({
      kind: 'rules', stage, verdict: v, packet_files: files, finding, category,
      stage_error: R.stageVerdictError(stage, v, category) ?? null, problem: R.verdictProblem(stage, v, files) ?? null, override: R.severityOverrideProblem(finding, v) ?? null,
    })
  } else {
    const stage = pick(...STAGES)
    strict = true
    const findings = Array.from({ length: Math.floor(rnd() * 16) }, (_, n) => ({ ...runFinding(n), id: `f${n}`, severity: pick(...SEV), verdict: { finding_id: `f${n}`, verdict: pick('accepted', 'accepted', 'rejected', 'needs-human', 'needs-human', 'hint'), ...(rnd() < 0.4 ? { answered: pick(true, false) } : {}), ...(rnd() < 0.4 ? { fixed: pick(true, false) } : {}) } }))
    strict = false
    const run = { stage, round: pick(1, 2), degraded: false, findings }
    const ids = (list) => list.map((f) => f.id)
    const questions = R.candidateQuestions(run)
    cases.push({
      kind: 'run', stage, round: run.round, findings,
      counts: R.countVerdicts(run), second_round: R.secondRoundDue(run), questions: questions === undefined ? null : { asked: ids(questions.asked), deferred: ids(questions.deferred) },
      open: ids(R.openCriticalQuestions(run)), unfixed: ids(R.unfixedCritical(run)),
    })
  }
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases -> ${OUT}`)
