#!/usr/bin/env node
// Writes the corpus the Rust test `tickets_corpus` replays: generated local ticket files, ticket graphs and specs, and
// what the node runner's ticket checker makes of them (domain/tickets.ts and localTicketOpen).
//
//   node scripts/crates/critique/tests/gen-tickets-corpus.mjs [count] [seed] [out.json]
//
// The node runner is a bundle, so the functions are reached by loading a copy of it without its entry point.
import { mkdtempSync, readFileSync, rmSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const bundle = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
const [N = '400', SEED = '11', OUT = join(here, 'fixtures', 'tickets-check.json')] = process.argv.slice(2)
const entry = '// critique-runner-entry.ts\nasync function readStdin'
const source = readFileSync(bundle, 'utf8')
const scratch = mkdtempSync(join(tmpdir(), 'critique-tickets-corpus-'))
writeFileSync(join(scratch, 'runner.mjs'), `${source.slice(0, source.indexOf(entry))}\nexport { checkTickets, localTickets, localTicketOpen, sizeReport, summariseCheck }\n`)
const { checkTickets, localTickets, localTicketOpen, sizeReport, summariseCheck } = await import(join(scratch, 'runner.mjs'))
rmSync(scratch, { recursive: true })
let s = Number(SEED)
const rnd = () => ((s = (s * 1664525 + 1013904223) % 4294967296) / 4294967296)
const pick = (...x) => x[Math.floor(rnd() * x.length)]
const some = (make, max) => Array.from({ length: Math.floor(rnd() * (max + 1)) }, make)

const EOL = () => pick('\n', '\n', '\n', '\r\n', '\r', ' ')
const TITLES = ['Schema', 'schema', 'Add the API', 'UI', 'Ship it', 'Ünïcode títle', 'Σίσυφος', 'two  spaces']
const heading = () =>
  pick(
    `# ${pick(...TITLES)}`,
    `# 01: ${pick(...TITLES)}`,
    `# #2 - ${pick(...TITLES)}`,
    `# 3. ${pick(...TITLES)}  `,
    `#\t04 – ${pick(...TITLES)}\t`,
    `## ${pick(...TITLES)}`,
    `#${pick(...TITLES)}`,
    `# 12`,
    '',
  )
const NAMES = () => pick('01', '1', '#1', '02', '#2', '2', '03', '10', '#10', '01-schema.md', '2 (later)', '3: x', '4.', '99', 'Schema', 'schema', 'UI', 'Ship it', 'nope', '#x')
const listOf = () => some(NAMES, 4)
const blockedBy = () =>
  pick(
    () => `**Blocked by:** ${listOf().join(pick(', ', '; ', ' and ', ' & ', ' ', ','))}`,
    () => `Blocked by: ${pick('None (can start immediately)', 'none', 'N/A', 'nothing yet', '-', '--', '—', '`01`', '"02"', "'#1'", '_03_', '*1* and *2*')}`,
    () => `## Blocked by${EOL()}${listOf().map((name) => `${pick('-', '*', '+', '1.', '2)')} ${name}`).join(EOL())}${EOL()}${pick('', 'after', '- late')}`,
    () => `> blocked BY${pick(':', '', '**:')}${EOL()}${EOL()}- ${NAMES()}${EOL()}  - ${NAMES()}`,
    () => `- Blocked by ${NAMES()}`,
    () => `Blockedby ${NAMES()}`,
    () => `blocked by - 01`,
    () => '',
  )()
const criteria = () =>
  pick(
    () => `## Acceptance criteria${EOL()}${some(() => pick('- [ ] a', '- [x] b', '* [X] c', '+ [ ] d', '  - [ ] nested', '- [] no', '- [y] no', '-[ ] no', '1. [ ] no', '- [ ]\tt'), 6).join(EOL())}`,
    () => `### ACCEPTANCE CRITERIA ###${EOL()}- [ ] one${EOL()}#### sub${EOL()}- [x] two${EOL()}## next${EOL()}- [ ] three`,
    () => `- [ ] loose${EOL()}- [x] loose`,
    () => '```\n- [ ] fenced\n```\n- [ ] after',
    () => '~~~\n## Acceptance criteria\n~~~',
    () => '',
  )()
const STORY = () => pick('3', '#3', '5', '12', '3-5', '3 – 5', '5-3', '1-2000', '#4–#6', '0', '007')
const citation = () =>
  pick(
    () => `${pick('user story', 'User Story', 'user stories', 'USER STORIES', 'user  stories', 'user story:', 'User stories: ', 'User stories **', 'userstory', 'super user story')} ${some(STORY, 3).join(pick(', ', ', and ', ', or ', ' and ', ' or ', ' & ', ',', ' x '))}`,
    () => `Covers user story ${STORY()}.`,
    () => '```\nuser story 9\n```',
    () => '',
  )()
const comments = () => pick('', `## Comments${EOL()}- [ ] in comments${EOL()}user story 8`, '## comments\n- [x] c')
const status = () => pick('', '', 'Status: open', '**Status:** Done', '> *Status*: closed', 'status:wontfix', 'Status: resolved ', 'Status: `done`', 'Status: in progress', '  Status : done', 'Status: Done\nStatus: open')
const ticketText = () => [heading(), status(), blockedBy(), criteria(), citation(), comments()].filter(() => rnd() < 0.85).join(EOL())
const FILE = () => pick('01-schema.md', '02-api.md', '2-api.md', '10-ui.md', '3.md', 'readme.md', 'x-01.md', '04-done.MD', '005-z.md', 'Ω.md')
const files = () => {
  const names = [...new Set(some(FILE, 6))].sort()
  return names.map((name) => ({ reference: pick(`.scratch/f/issues/${name}`, `issues/${name}`), text: ticketText() }))
}
const spec = () =>
  [
    pick('# Spec', ''),
    pick('## User Stories', '## user stories', '# User stories', '### User Stories', '## User Story', ''),
    some(() => pick('1. one', '2) two', '3. three', '10. ten', '  4. indented', '5.no', '3. again', '007. lead', '#### sub', '6. six'), 6).join(EOL()),
    pick('', '## Next', '# Top', '```\n7. fenced\n```', '## User Stories\n99. second section'),
    pick('', '8. after'),
  ].join(EOL())
const ID = () => pick('#1', '#2', '#3', '#4', '#5', '#6', 'github.com/o/r#7', '#9')
const graph = () =>
  some(() => ({ blockedBy: some(ID, 4), id: ID(), reference: 'github.com/o/r#1', text: ticketText(), title: pick(...TITLES) }), 7)
const dense = () => {
  const ids = ['#1', '#2', '#3', '#4', '#5', '#6']
  return ids.map((id) => ({ blockedBy: ids.filter(() => rnd() < 0.5), id, reference: id, text: '', title: id }))
}

const cases = []
for (let i = 0; i < Number(N); i += 1) {
  const kind = pick('local', 'local', 'graph', 'dense')
  const spec_text = spec()
  const input = kind === 'local' ? { files: files() } : { tickets: kind === 'graph' ? graph() : dense() }
  const tickets = kind === 'local' ? localTickets(input.files) : input.tickets
  const check = checkTickets(spec_text, tickets)
  const size = { max_criteria: pick(0, 1, 2, 8), max_modules: 5 }
  cases.push({
    ...input,
    expected: { check, local: kind === 'local' ? tickets : null, open: tickets.map((ticket) => localTicketOpen(ticket.text)), size: sizeReport(check, size), summary: summariseCheck(check) },
    size,
    spec: spec_text,
  })
}
writeFileSync(OUT, `${JSON.stringify(cases)}\n`)
console.log(`${cases.length} cases written to ${OUT}`)
