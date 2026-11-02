import { readdir, readFile, stat } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { ASSETS_ROOT, codeStageRouting, fenced, finding, harness, isolatedExport, vibeStdout, type Harness } from '../../../test/critique/harness.js'
import { parseDiff } from './domain/diff-filter.js'
import { DIFF_WINDOW_NOTICE, diffDropLine, diffShownLines, fileKind, windowDiff } from './domain/diff-window.js'
import type { PrepareResult } from './index.js'

/**
 * A code packet whose diff alone passes the cap is sent with its diff in
 * windows by file, not refused (#257); and a diff's file is named by its
 * `+++` or `---` line, never by splitting the `diff --git` line (#254 F2).
 */

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

const TICKET = '.scratch/widgets/issues/01-frobnicate.md'
const SPEC = '.scratch/widgets/spec.md'

async function cappedRouting(limits: Record<string, number>): Promise<unknown> {
  const shipped = JSON.parse(await readFile(join(ASSETS_ROOT, 'routing.json'), 'utf8')) as { readonly packet: object }
  return { ...(codeStageRouting() as object), packet: { ...shipped.packet, ...limits } }
}

async function repository(limits: Record<string, number> = { cap_bytes: 60000 }): Promise<Harness> {
  const h = await harness({ routing: await cappedRouting(limits) })
  open.push(h)
  await h.commands.allow.run({ code: true, families: ['mistral'], repo: h.repo })
  await h.writeText(join(h.repo, TICKET), '# 01: Frobnicate widgets\n')
  return h
}

async function prepareCode(h: Harness, input: Record<string, unknown> = {}): Promise<PrepareResult> {
  const result = await h.commands.prepare.run({
    base: 'HEAD~1',
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: SPEC,
    stage: 'ticket-review',
    ticket: TICKET,
    ...input,
  })
  if (result.status !== 'ready') throw new Error(`prepare refused: ${result.message}`)
  return result
}

const diffOf = (run: PrepareResult) => readFile(join(run.packet_dir, 'diff/change.diff'), 'utf8')
const indexOf = (run: PrepareResult) => readFile(join(run.packet_dir, 'facts/INDEX.md'), 'utf8')

const CAP = 9000

/** A file of `count` rows of about 60 bytes inside one function, the rows `changed` names rewritten. */
function rows(name: string, count: number, changed: (row: number) => boolean = () => false): string {
  const body = Array.from({ length: count }, (_, i) => `    '${changed(i + 3) ? 'ROW' : 'row'} ${String(i + 3)} ${'x'.repeat(40)}',\n`).join('')
  return `export function ${name}(): string[] {\n  return [\n${body}  ]\n}\n`
}

const ENGINE_CHANGED = (line: number) => line % 10 === 0
const TEST_CHANGED = (line: number) => line % 20 === 0

/**
 * A change whose diff is twice the 9,000-byte cap: it rewrites one row in
 * ten of `src/engine.ts` (40 changed rows), one in twenty of
 * `src/util.ts` and `test/engine.test.ts`, and adds `docs/guide.md`, 250
 * lines, more changed lines than any other file.
 */
async function oversizedChange(limits: Record<string, number> = { cap_bytes: CAP }): Promise<Harness> {
  const h = await repository({ facts_cap_bytes: 1000, window_lines: 400, ...limits })
  h.git.hunks = 'by-line'
  await h.writeText(join(h.repo, 'src/engine.ts'), rows('engine', 400))
  await h.writeText(join(h.repo, 'src/util.ts'), rows('util', 200))
  await h.writeText(join(h.repo, 'test/engine.test.ts'), rows('engineTest', 200))
  await h.git.commit()
  await h.writeText(join(h.repo, 'src/engine.ts'), rows('engine', 400, ENGINE_CHANGED))
  await h.writeText(join(h.repo, 'src/util.ts'), rows('util', 200, TEST_CHANGED))
  await h.writeText(join(h.repo, 'test/engine.test.ts'), rows('engineTest', 200, TEST_CHANGED))
  await h.writeText(join(h.repo, 'docs/guide.md'), Array.from({ length: 250 }, (_, i) => `Guide line ${String(i + 1)} ${'y'.repeat(40)}\n`).join(''))
  await h.git.commit()
  return h
}

/** The lines from 3 to `last` that `changed` picks. */
function changedRows(last: number, changed: (row: number) => boolean): readonly number[] {
  return Array.from({ length: last - 2 }, (_, i) => i + 3).filter(changed)
}

async function packetBytes(dir: string): Promise<number> {
  const entries = await readdir(dir, { recursive: true, withFileTypes: true })
  const sizes = await Promise.all(entries.filter((entry) => !entry.isDirectory()).map(async (entry) => (await stat(join(entry.parentPath, entry.name))).size))
  return sizes.reduce((sum, size) => sum + size, 0)
}

const FAILING_TEST = { kind: 'failing-test', value: 'expect(engine()[7]).toMatch(/^row/)' }

function bug(location: string, ...evidence: readonly { kind: string; value: string }[]): Record<string, unknown> {
  return finding({ category: 'bug', claim: 'engine shouts one row in ten.', evidence, location })
}

/** The pre-verdict fan-out's mechanical check gives a critic's one finding. */
async function preVerdictOf(h: Harness, run: PrepareResult, found: Record<string, unknown>): Promise<unknown> {
  h.process.queue({ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [found] })) })
  return (await h.commands.fanOut.run({ run_dir: run.run_dir })).findings[0]?.pre_verdict
}

/** The line the facts index gives a file's left-out hunks. */
function leftOutLine(index: string, path: string): string | undefined {
  return index.split('\n').find((line) => line.startsWith(`- \`${path}\`: lines `))
}

describe('a code packet whose diff alone passes the cap', () => {
  it('prepares ready, with the most changed source files first, and lists every hunk it left out by file and line', async () => {
    const h = await oversizedChange()
    const full = await h.git.diffCommits(h.repo, 'c2', 'c3')
    expect(Buffer.byteLength(full)).toBeGreaterThan(2 * CAP)

    const run = await prepareCode(h)

    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(CAP)
    const diff = await diffOf(run)
    const index = await indexOf(run)
    expect(diff).toContain('+++ b/src/engine.ts\n')
    expect(diff).not.toContain('+++ b/docs/guide.md')
    const engine = changedRows(402, ENGINE_CHANGED)
    const sent = engine.filter((line) => diff.includes(`@@ -${String(line)} +${String(line)} @@\n`))
    expect(sent.length).toBeGreaterThan(0)
    expect(sent).toEqual(engine.slice(0, sent.length))
    expect(leftOutLine(index, 'src/engine.ts')).toBe(`- \`src/engine.ts\`: lines ${engine.slice(sent.length).join(', ')}`)
    expect(leftOutLine(index, 'src/util.ts')).toBe(`- \`src/util.ts\`: lines ${changedRows(202, TEST_CHANGED).join(', ')}`)
    expect(leftOutLine(index, 'test/engine.test.ts')).toBe(`- \`test/engine.test.ts\`: lines ${changedRows(202, TEST_CHANGED).join(', ')}`)
    expect(leftOutLine(index, 'docs/guide.md')).toBe('- `docs/guide.md`: lines 1-250')
    expect(diff.split('\n')[0]).toBe('The packet cap left hunks out of this diff; facts/INDEX.md lists each one by file and line.')
  })

  it('lets a citation into a hunk it sent resolve, though the tree does not hold that line', async () => {
    const h = await oversizedChange()
    const run = await prepareCode(h)
    expect(await readdir(join(run.packet_dir, 'tree/src')).catch(() => [])).not.toContain('engine.ts')
    const trace = { kind: 'trace', value: 'tree/src/engine.ts:10 returns ROW 10, which tree/src/engine.ts:20 repeats.' }

    expect(await preVerdictOf(h, run, bug('tree/src/engine.ts:10', trace))).toBeUndefined()
  })

  it('tells the critic how to cite a line a hunk shows and where the left-out hunks are listed', async () => {
    const h = await oversizedChange()
    const run = await prepareCode(h)
    h.process.queue({ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [], no_findings_reason: 'I read the diff.' })) })
    await h.commands.fanOut.run({ run_dir: run.run_dir })

    const prompt = await h.readText(join(run.run_dir, 'critics', 'c1-vibe-mistral-large', 'prompt.md'))

    expect(prompt).toContain('A line a hunk of `diff/change.diff` shows is cited the same way, at the number its `@@` line gives it.')
    expect(prompt).toContain('When the diff did not fit the packet cap, its first line says so and `facts/INDEX.md` lists each hunk it left out by file and line.')
  })

  // #257 review F1.
  it('lets a citation into a sent hunk that only removes lines resolve', async () => {
    const h = await repository()
    await h.writeText(join(h.repo, 'src/gone.ts'), 'a\nb\nc\n')
    await h.git.commit()
    await h.writeText(join(h.repo, 'src/gone.ts'), '')
    await h.writeText(join(h.repo, 'src/keep.ts'), 'export const keep = 1\n')
    await h.git.commit()
    const run = await prepareCode(h)
    expect(await preVerdictOf(h, run, bug('tree/src/gone.ts:2', FAILING_TEST))).toBeUndefined()
  })

  // #257 review F2. It writes 800 files, so it gets more than the default five seconds.
  it('prepares ready when the documents alone fit, however many files the diff touches', async () => {
    const h = await repository({ cap_bytes: 9000, repo_map_cap_bytes: 500 })
    for (let i = 0; i < 400; i++) await h.writeText(join(h.repo, `src/f${String(i)}.ts`), `export const f${String(i)} = ${String(i)}\n`)
    await h.git.commit()
    for (let i = 0; i < 400; i++) await h.writeText(join(h.repo, `src/f${String(i)}.ts`), `export const f${String(i)} = ${String(i + 1)}\n`)
    await h.git.commit()
    const run = await prepareCode(h)
    expect(await packetBytes(run.packet_dir)).toBeLessThanOrEqual(9000)
  }, 30_000)

  it('caps a facts index that cannot list every drop, counting what each list leaves unlisted', async () => {
    const h = await repository({ cap_bytes: 9000, repo_map_cap_bytes: 500 })
    for (let i = 0; i < 400; i++) await h.writeText(join(h.repo, `src/f${String(i)}.ts`), `export const f${String(i)} = ${String(i)}\n`)
    await h.git.commit()
    for (let i = 0; i < 400; i++) await h.writeText(join(h.repo, `src/f${String(i)}.ts`), `export const f${String(i)} = ${String(i + 1)}\n`)
    await h.git.commit()

    const run = await prepareCode(h)

    const index = await indexOf(run)
    expect(Buffer.byteLength(index)).toBeLessThanOrEqual(Math.max(2500, (9000 - 1000) / 4))
    expect(index).toMatch(/\n- … and \d+ more, which this index's cap of \d+ bytes leaves unlisted\n/)
    const diff = await diffOf(run)
    expect(diff.split('\n')[0]).toBe('The packet cap left hunks out of this diff; facts/INDEX.md lists each one by file and line.')
    expect(diff).toContain('+export const f')
  }, 30_000)

  it('rejects a citation into a hunk it left out mechanically', async () => {
    const h = await oversizedChange()
    const run = await prepareCode(h)

    expect(await preVerdictOf(h, run, bug('tree/src/engine.ts:390', FAILING_TEST))).toEqual({
      reason:
        'the location cites line 390 of tree/src/engine.ts, outside the lines the packet holds (10-10, 20-20, 30-30, 40-40, 50-50, 60-60, 70-70, 80-80, 90-90, 100-100, 110-110, 120-120, 130-130, 140-140, 150-150, 160-160, 170-170, 180-180, 190-190)',
      reason_code: 'fabricated-evidence',
      verdict: 'rejected',
    })
    expect(await preVerdictOf(h, run, bug('tree/docs/guide.md:5', FAILING_TEST))).toMatchObject({
      reason: 'the location cites tree/docs/guide.md, which is not in the packet',
    })
  })
})

describe('a diff in windows', () => {
  const MODE = 'diff --git a/run.sh b/run.sh\nold mode 100644\nnew mode 100755\n'
  const GONE = 'diff --git a/src/gone.ts b/src/gone.ts\ndeleted file mode 100644\n--- a/src/gone.ts\n+++ /dev/null\n@@ -1,3 +0,0 @@\n-a\n-b\n-c\n'
  const EDIT = 'diff --git a/src/edit.ts b/src/edit.ts\n--- a/src/edit.ts\n+++ b/src/edit.ts\n@@ -4 +4 @@\n-x\n+y\n@@ -9,2 +8,0 @@\n-p\n-q\n'
  const STANDING = 'Left out of this diff: package-lock.json, a lock file; its 9 changed lines are not shown.\n'

  it('lists a hunk that only removes lines by its lines at the base, and a section with no hunks as a whole', () => {
    const window = windowDiff(MODE + GONE + EDIT + STANDING, 0)

    expect(window.diff).toBe(DIFF_WINDOW_NOTICE + STANDING)
    expect(window.dropped.map(diffDropLine)).toEqual([
      '- `src/edit.ts`: lines 4; base lines 9-10',
      '- `src/gone.ts`: base lines 1-3',
      '- `run.sh`: its whole section, which has no hunks',
    ])
  })

  it('shows a file with no lines at the head at its base lines, but no line of another hunk that only removes lines', () => {
    const shown = diffShownLines(GONE + EDIT)

    expect(shown.get('src/gone.ts')).toEqual([{ first: 1, last: 3 }])
    expect(shown.get('src/edit.ts')).toEqual([{ first: 4, last: 4 }])
  })

  // #257 review F3.
  it('ranks source files by changed lines, not context lines', () => {
    const big = 'diff --git a/src/big.ts b/src/big.ts\n--- a/src/big.ts\n+++ b/src/big.ts\n@@ -1,6 +1,6 @@\n x\n x\n-old\n+x\n x\n x\n x\n x\n'
    const small = 'diff --git a/src/small.ts b/src/small.ts\n--- a/src/small.ts\n+++ b/src/small.ts\n@@ -1,2 +1,2 @@\n-a\n-b\n+c\n+d\n'
    const window = windowDiff(big + small, 100000)
    expect(window.diff.indexOf('src/small.ts')).toBeLessThan(window.diff.indexOf('src/big.ts'))
  })

  it('still sends a later, smaller section when an earlier one does not fit', () => {
    const window = windowDiff(GONE + MODE, Buffer.byteLength(MODE))

    expect(window.diff).toBe(DIFF_WINDOW_NOTICE + MODE)
    expect(window.dropped.map((drop) => drop.path)).toEqual(['src/gone.ts'])
  })

  it.each([
    ['src/engine.ts', 'source'],
    ['src/engine.test.ts', 'test'],
    ['pkg/engine_test.go', 'test'],
    ['tests/test_engine.py', 'test'],
    ['src/__tests__/engine.ts', 'test'],
    ['README.md', 'doc'],
    ['docs/adr/0001.txt', 'doc'],
    ['routing.json', 'source'],
  ])('ranks %s as a %s file', (path, kind) => {
    expect(fileKind(path)).toBe(kind)
  })
})

describe('a code packet whose documents alone pass the cap', () => {
  it('is still refused with packet-over-cap, however small its diff', async () => {
    const h = await repository({ cap_bytes: CAP })
    await h.writeText(join(h.repo, SPEC), `# Widgets\n\n${'1. As the owner, I want widgets frobnicated.\n'.repeat(250)}`)
    await h.writeText(join(h.repo, 'src/widget.ts'), 'export const widget = 1\n')
    await h.git.commit()

    const result = await h.commands.prepare.run({ base: 'HEAD~1', orchestrator_family: 'anthropic', repo: h.repo, spec: SPEC, stage: 'ticket-review', ticket: TICKET })

    expect(result).toMatchObject({ reason: 'packet-over-cap', status: 'refused' })
    expect((result as { message: string }).message).toMatch(
      /^the tickets, the spec, tree\/REPO-MAP\.md and facts\/INDEX\.md come to \d+ bytes before any hunk of the diff, past the packet cap of 9000; review a smaller change or raise packet\.cap_bytes in routing$/,
    )
  })
})

describe('the file a diff section names', () => {
  it('is read from its +++ line, so a vendored path holding " b/" is still left out', async () => {
    const h = await repository()
    await h.writeText(join(h.repo, 'node_modules/a b/index.js'), 'module.exports = 1\n')
    await h.git.commit()

    const run = await prepareCode(h)

    const diff = await diffOf(run)
    expect(diff).not.toContain('module.exports')
    expect(diff).toContain('Left out of this diff: node_modules/a b/index.js, a vendored file: it matches the routing glob `**/node_modules/**`')
  })

  // Headers as git prints them, quoting a path that holds a quote, a backslash or a control character.
  it.each([
    ['a rename, by its rename to line', 'diff --git "a/we\\"ird.ts" "b/n\\"ew.ts"\nsimilarity index 100%\nrename from "we\\"ird.ts"\nrename to "n\\"ew.ts"\n', 'n"ew.ts'],
    ['an added file, by its quoted +++ line', 'diff --git "a/n\\"ew.ts" "b/n\\"ew.ts"\nnew file mode 100644\n--- /dev/null\n+++ "b/n\\"ew.ts"\n@@ -0,0 +1 @@\n+q\n', 'n"ew.ts'],
    ['a deleted file, by its --- line', 'diff --git a/gone.ts b/gone.ts\ndeleted file mode 100644\n--- a/gone.ts\n+++ /dev/null\n@@ -1,2 +0,0 @@\n-a\n-b\n', 'gone.ts'],
    ['a mode change, by the one path its diff --git line names twice', 'diff --git a/run b/x.sh b/run b/x.sh\nold mode 100644\nnew mode 100755\n', 'run b/x.sh'],
    ['a binary file with a quoted path', 'diff --git "a/q\\\\b" "b/q\\\\b"\nBinary files "a/q\\\\b" and "b/q\\\\b" differ\n', 'q\\b'],
    ['a path git escapes in octal', 'diff --git "a/tab\\there\\303\\251.ts" "b/tab\\there\\303\\251.ts"\n--- "a/tab\\there\\303\\251.ts"\n+++ "b/tab\\there\\303\\251.ts"\n@@ -1 +1 @@\n-a\n+b\n', 'tab\thereé.ts'],
    ['nothing, for a diff --git line naming two paths and no other line', 'diff --git a/one b/two\n', undefined],
    ['nothing, for a quoted diff --git line with no b/ side', 'diff --git "a/one" "c/one"\n', undefined],
  ])('names %s', (_case, diff, path) => {
    expect(parseDiff(diff).map((section) => section.path)).toEqual([path])
  })

  it('cuts a section into its header and hunks, each hunk with both sides of its @@ line', () => {
    const [section] = parseDiff('diff --git a/w.ts b/w.ts\n--- a/w.ts\n+++ b/w.ts\n@@ -2 +2 @@\n-a\n+b\n@@ -9,3 +9,0 @@\n-c\n-d\n-e\n')

    expect(section?.header).toBe('diff --git a/w.ts b/w.ts\n--- a/w.ts\n+++ b/w.ts\n')
    expect(section?.hunks).toEqual([
      { new: { count: 1, start: 2 }, old: { count: 1, start: 2 }, text: '@@ -2 +2 @@\n-a\n+b\n' },
      { new: { count: 0, start: 9 }, old: { count: 3, start: 9 }, text: '@@ -9,3 +9,0 @@\n-c\n-d\n-e\n' },
    ])
  })
})
