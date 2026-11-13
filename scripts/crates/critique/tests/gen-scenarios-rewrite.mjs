#!/usr/bin/env node
// Writes the scenario.json files of check-rewrite (the goldens are expected.json, from regenerate-fixtures.mjs).
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const step = (argv, extra = {}) => ({ argv, ...extra })
const rewrite = (input, extra) => step(['check-rewrite', '--json', typeof input === 'string' ? input : JSON.stringify(input)], extra)
const GITHUB = 'git@github.com:JorisJonkers-dev/example.git'
const R = (dir) => `@SANDBOX@/repos/${dir}`
const init = (dir, remote = GITHUB) => [['init', '-q', '--template=', `repos/${dir}`], ['-C', `repos/${dir}`, 'remote', 'add', 'origin', remote]]
const tree = (dir, text, extra = {}) => ({ write: Object.fromEntries(Object.entries(text).map(([path, body]) => [`repos/${dir}/${path}`, body])), git: init(dir), ...extra })
const scenarios = {}

const A = 'We use Postgres.'
const B = 'Events are append-only.'
const files = {
  'docs/adr/0001-db.md': `# 1. DB\n\n${A}\n`,
  'docs/adr/0002-events.md': `# 2. Events\n\n${B}\n\nSuperseded by [0003](0003-new.md).\n`,
  'docs/adr/0003-new.md': '# 3. New\n\nReplaces ADR-0002.\n',
  'docs/adr/0004-twice.md': `# 4. Twice\n\n${A} ${A}\n`,
  'docs/adr/0005-overlap.md': '# 5. Overlap\n\naaaa\n',
  'docs/adr/0006-split.md': '# 6. Split\n\nStatus: Superseded\n\nSuperseded by ADR-0007, [eight](0008-eight.md#x) and replaced by `0009`.\n\n```\nSuperseded by [ten](0010-ten.md)\n```\n',
  'docs/adr/0007-seven.md': '# 7. Seven\n', 'docs/adr/0008-eight.md': '# 8. Eight\n', 'docs/adr/0009-nine.md': '# 9. Nine\n', 'docs/adr/0010-ten.md': '# 10. Ten\n',
  'docs/adr/..dots.md': `# Dots\n\n${B}\n`,
}
const base = tree('rw', files)

scenarios['rewrite-checks'] = [
  rewrite({ repo: R('rw'), original_sentences: [A, B], rewritten: [{ path: 'docs/adr/0001-db.md', decision_sentences: [A] }, { path: 'docs/adr/0002-events.md', decision_sentences: [B] }] }, base),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'docs/adr/0002-events.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [A, B], rewritten: [{ path: 'docs/adr/0001-db.md', decision_sentences: [A, 'New one.'] }, { path: 'docs/adr/0004-twice.md', decision_sentences: ['Another.', A] }] }),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'docs/adr/0004-twice.md', decision_sentences: [A] }] }),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'docs/adr/0001-db.md', decision_sentences: [A] }, { path: 'docs/adr/0004-twice.md', decision_sentences: [A] }] }),
  rewrite({ repo: R('rw'), original_sentences: ['aa'], rewritten: [{ path: 'docs/adr/0005-overlap.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [] }),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: R('rw') + '/docs/adr/0001-db.md', decision_sentences: [A] }, { path: './docs/../docs/adr/0002-events.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [B], rewritten: [{ path: 'docs/adr/..dots.md', decision_sentences: [B] }] }),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'docs/adr/0001-db.md', decision_sentences: [A] }], ignored: 1 }),
  step(['check-rewrite', '--json', JSON.stringify({ repo: R('rw'), original_sentences: ['nope'], rewritten: [] }), '--strict']),
]

const one = { repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'docs/adr/0001-db.md', decision_sentences: [A] }] }
scenarios['rewrite-split'] = [
  rewrite({ ...one, split: { original: 'docs/adr/0006-split.md', files: ['docs/adr/0007-seven.md', 'docs/adr/0008-eight.md', 'docs/adr/0009-nine.md'] } }, base),
  rewrite({ ...one, split: { original: 'docs/adr/0006-split.md', files: ['docs/adr/0010-ten.md', 'docs/adr/0007-seven.md', 'docs/adr/nothing.md', '0007-seven.md'] } }),
  rewrite({ ...one, split: { original: 'docs/adr/0002-events.md', files: ['docs/adr/0003-new.md'] } }),
  rewrite({ ...one, split: { original: 'docs/adr/0003-new.md', files: ['docs/adr/0002-events.md'] } }),
  rewrite({ ...one, split: { original: 'docs/adr/0001-db.md', files: [] } }),
  rewrite({ ...one, split: { original: 'docs/adr/missing.md', files: [] } }),
  rewrite({ ...one, split: { original: '../escape.md', files: [] } }),
]

scenarios['rewrite-input'] = [
  rewrite({}, base), rewrite([]), rewrite('null'), rewrite({ repo: R('rw') }),
  rewrite({ repo: R('rw'), original_sentences: 'x', rewritten: [] }),
  rewrite({ repo: R('rw'), original_sentences: [5], rewritten: [] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: 'x' }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [5] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: 'a.md' }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: '', decision_sentences: [] }] }),
  rewrite({ original_sentences: [], rewritten: [] }),
  rewrite({ repo: '', original_sentences: [], rewritten: [] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [], split: 5 }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [], split: { files: [] } }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [], split: { original: 'a.md' } }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [], split: null }),
  rewrite({ repo: R('rw'), original_sentences: [''], rewritten: [] }),
  rewrite({ repo: R('rw'), original_sentences: [A], rewritten: [{ path: 'a.md', decision_sentences: [''] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: '../escape.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: '/etc/passwd', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: 'missing.md', decision_sentences: [] }, { path: '../escape.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: 'missing.md', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: 'docs', decision_sentences: [] }] }),
  rewrite({ repo: R('rw'), original_sentences: [], rewritten: [{ path: '.', decision_sentences: [] }] }),
  rewrite({ repo: R('none'), original_sentences: [], rewritten: [] }, { git: init('none').slice(0, 1) }),
  rewrite({ repo: R('nedap'), original_sentences: [], rewritten: [] }, { git: init('nedap', 'git@github.com:nedap/secret.git') }),
  rewrite({ repo: R('gone'), original_sentences: [], rewritten: [] }),
  step(['check-rewrite', '--json', JSON.stringify({ repo: R('none'), original_sentences: [], rewritten: [] }), '--strict']),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
}
