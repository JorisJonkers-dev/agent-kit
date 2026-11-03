import { createHash } from 'node:crypto'
import { cp, mkdir, mkdtemp, readdir, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { dirname, join, relative, resolve } from 'node:path'
import { Readable } from 'node:stream'
import { fileURLToPath } from 'node:url'

import type {
  ClockPort,
  EnvPort,
  GhIssue,
  GhIssueLocator,
  GhIssueRequest,
  GhLinkedIssue,
  GitCommitLink,
  GitMergeTree,
  GitPorcelainEntry,
  GitRemote,
  ProcessCommand,
  ProcessPort,
  ProcessResult,
} from '../../src/ports/index.js'
import {
  AgyCriticClient,
  createCritiqueCommands,
  NodeCritiqueFiles,
  ProcessSearchClient,
  runSearchServer,
  VibeCriticClient,
  type CritiqueCommands,
  type CritiqueGhPort,
  type CritiqueGitPort,
} from '../../src/contexts/critique/index.js'

export const ASSETS_ROOT = resolve(dirname(fileURLToPath(import.meta.url)), '../../../../skills/workstation/critique')

/** The critic agent files agent-kit's generator renders, which setup installs into `~/.vibe/agents/`. */
export const GENERATED_AGENTS = resolve(dirname(fileURLToPath(import.meta.url)), '../../../../registry/generated/vibe/agents')

export const SPEC_CANARY = 'Every widget must be frobnicated within the canary window of seven seconds.'
export const DECISION_CANARY = 'Decided: widgets are frobnicated by the nightly job only.'

export class FakeEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.values[name]
    if (value === undefined) throw new Error(`missing ${name}`)
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

export class FixedClock implements ClockPort {
  private ticks = 0
  private at: number

  constructor(iso: string) {
    this.at = Date.parse(iso)
  }

  now(): Date {
    return new Date(this.at)
  }

  /** Moves the clock on, so the next run gets a later run id and record time. */
  advance(minutes: number): void {
    this.at += minutes * 60_000
  }

  monotonicMs(): number {
    this.ticks += 1500
    return this.ticks
  }

  private awake = false
  private readonly sleepers: (() => void)[] = []

  /**
   * Sleeps until `wake()`, so a runner's deadline never fires on its own in
   * a test: the work under test always finishes first unless the test says
   * time ran out.
   */
  async sleep(): Promise<void> {
    if (this.awake) return
    await new Promise<void>((resolve) => this.sleepers.push(resolve))
  }

  /** Ends every sleep, now and from now on: each deadline the runner set has passed. */
  wake(): void {
    this.awake = true
    this.sleepers.splice(0).forEach((resolve) => {
      resolve()
    })
  }
}

/**
 * A repository without git: `commit()` snapshots the files on disk, outside
 * `.git/`, as a new commit on one line of history, and `porcelainStatus`
 * compares the files on disk with the last commit, the way git compares the
 * working tree with HEAD. A commit's id is `c<n>`; `HEAD` and `HEAD~<n>`
 * name commits too. The index holds what `commit()` committed until
 * `stage()` copies the files on disk into it, as `git add -A` would; a
 * commit made with `commitTree` (`s<n>`) is off that line and moves no HEAD.
 * Branches name commits too: `checkout` switches HEAD's line to one, and
 * `merge` starts a merge the next `commit()` concludes with two parents.
 */
export class FakeGit implements CritiqueGitPort {
  private readonly history: { readonly id: string; readonly tree: ReadonlyMap<string, string>; readonly parents: readonly string[] }[] = []
  /** Each branch's tip, as `checkout`, `setBranch` and `commit()` left it. */
  private readonly branches = new Map<string, string>()
  /** The commit a merge in progress merges, as `MERGE_HEAD` names it. */
  private mergeHead: string | undefined
  /** HEAD's line of history, oldest first; commits made with `commitTree` are not on it. */
  private readonly line: string[] = []
  private index: ReadonlyMap<string, string> = new Map()
  private readonly trees = new Map<string, ReadonlyMap<string, string>>()
  /** Every revision the runner asked about, so a test can see what it resolved. */
  readonly resolved: string[] = []
  /** How many times the index was written as a tree. */
  indexTreeWrites = 0
  /** How many times a commit's tree was read as index entries. */
  treeEntryReads = 0
  /** The branch HEAD is on; empty for a detached HEAD, as `git branch --show-current` prints it. */
  branch = 'main'
  /** What reading the remotes does: answer, never return (a git that hangs), or throw this error (a git that crashes). */
  remotesBehaviour: 'answer' | 'hang' | Error = 'answer'
  /** How a diff prints a changed file. */
  hunks: FakeHunks = 'whole-file'

  constructor(
    private readonly repoRoot: string,
    public remoteList: readonly GitRemote[],
  ) {}

  async root(): Promise<string> {
    return Promise.resolve(this.repoRoot)
  }

  async commonDir(): Promise<string> {
    return Promise.resolve(join(this.repoRoot, '.git'))
  }

  async remotes(): Promise<readonly GitRemote[]> {
    const behaviour = this.remotesBehaviour
    if (behaviour === 'hang') return new Promise<readonly GitRemote[]>(() => undefined)
    if (behaviour instanceof Error) throw behaviour
    return Promise.resolve(this.remoteList)
  }

  async currentBranch(): Promise<string> {
    return Promise.resolve(this.branch)
  }

  /** Records the working tree as a new commit on HEAD's line, stages it as `git commit -a` leaves it, and returns its id. */
  async commit(): Promise<string> {
    const id = `c${String(this.history.filter((entry) => entry.id.startsWith('c')).length + 1)}`
    const tree = await this.workingTree()
    const parent = this.line.at(-1)
    const parents = [...(parent === undefined ? [] : [parent]), ...(this.mergeHead === undefined ? [] : [this.mergeHead])]
    this.history.push({ id, parents, tree })
    this.line.push(id)
    this.index = tree
    this.mergeHead = undefined
    if (this.branch !== '') this.branches.set(this.branch, id)
    return id
  }

  /** Points a branch at a commit without moving HEAD, as a fetch moves a remote-tracking branch. */
  setBranch(name: string, commit: string): void {
    this.branches.set(name, commit)
  }

  /**
   * Switches HEAD to a branch, as `git checkout` does: the index and the
   * files on disk become its tip's. With `from`, the branch is made there
   * first, as `git checkout -b` makes it.
   */
  async checkout(name: string, from?: string): Promise<void> {
    if (this.branch !== '' && this.line.length > 0) this.branches.set(this.branch, this.line.at(-1) ?? '')
    const tip = from === undefined ? this.branches.get(name) : await this.resolveCommit('', from)
    if (tip === undefined) throw new Error(`error: pathspec '${name}' did not match any file(s) known to git`)
    this.branches.set(name, tip)
    this.branch = name
    this.line.splice(0, this.line.length, ...this.firstParents(tip).reverse())
    await this.become(this.treeOfCommit(tip))
  }

  /** Starts a branch with no commits and no files, as `git checkout --orphan` and clearing the index leave it. */
  async orphan(name: string): Promise<void> {
    this.branch = name
    this.line.splice(0)
    await this.become(new Map())
  }

  /**
   * Starts merging a branch into HEAD, as `git merge --no-ff --no-commit`
   * does: `MERGE_HEAD` names its tip, and the index and the files on disk
   * hold the merge, conflicted files with their conflict markers.
   */
  async merge(name: string): Promise<void> {
    const theirs = await this.resolveCommit('', name)
    const merged = await this.mergeTree('', this.line.at(-1) ?? '', theirs)
    this.mergeHead = theirs
    await this.become(this.trees.get(merged.tree) ?? new Map())
  }

  /** Copies the files on disk into the index, as `git add -A` does. */
  async stage(): Promise<void> {
    this.index = await this.workingTree()
  }

  async porcelainStatus(): Promise<readonly GitPorcelainEntry[]> {
    const tree = await this.workingTree()
    const committed = this.headTree()
    const entries: GitPorcelainEntry[] = []
    for (const [path, text] of tree) {
      const before = committed.get(path)
      if (before === undefined) entries.push({ code: '??', path })
      else if (before !== text) entries.push({ code: ' M', path })
    }
    for (const path of committed.keys()) if (!tree.has(path)) entries.push({ code: ' D', path })
    return entries.sort((a, b) => a.path.localeCompare(b.path))
  }

  async resolveCommit(_cwd: string, revision: string): Promise<string> {
    this.resolved.push(revision)
    if (revision === 'MERGE_HEAD') {
      if (this.mergeHead === undefined) throw new Error("fatal: ambiguous argument 'MERGE_HEAD': unknown revision or path not in the working tree.")
      return Promise.resolve(this.mergeHead)
    }
    const tip = revision === this.branch ? undefined : this.branches.get(revision)
    if (tip !== undefined) return Promise.resolve(tip)
    const back = /^HEAD(?:~(\d+))?$/.exec(revision === this.branch && revision !== '' ? 'HEAD' : revision)
    const id = back === null ? revision : this.line[this.line.length - 1 - Number(back[1] ?? 0)]
    const commit = this.history.find((entry) => entry.id === id)
    if (commit === undefined) throw new Error(`fatal: ambiguous argument '${revision}': unknown revision or path not in the working tree.`)
    return Promise.resolve(commit.id)
  }

  /** Every commit reachable from `head` and not from `base`, newest first, with its tree and parents. */
  async commitsBetween(_cwd: string, base: string, head: string): Promise<readonly GitCommitLink[]> {
    const ofBase = new Set(this.ancestors(base))
    return Promise.resolve(
      this.ancestors(head)
        .filter((id) => !ofBase.has(id))
        .map((commit) => ({ commit, parents: this.history.find((entry) => entry.id === commit)?.parents ?? [], tree: treeId(this.treeOfCommit(commit)) })),
    )
  }

  /** A commit's tree in the form `stagedEntries` prints the index. */
  async treeEntries(_cwd: string, revision: string): Promise<string> {
    this.treeEntryReads += 1
    return Promise.resolve(entriesOf(this.treeOfCommit(revision)))
  }

  /**
   * A three-way merge of two commits on their merge base, file by file: a
   * file only one side changed takes that side; one both changed alike
   * keeps it; one both changed differently holds both sides between
   * conflict markers, as git's default conflict style writes them.
   */
  async mergeTree(_cwd: string, ours: string, theirs: string): Promise<GitMergeTree> {
    const base = this.treeOfCommit(await this.mergeBase('', ours, theirs))
    const [mine, other] = [this.treeOfCommit(ours), this.treeOfCommit(theirs)]
    const merged = new Map<string, string>()
    let conflicts = false
    for (const path of [...new Set([...base.keys(), ...mine.keys(), ...other.keys()])].sort()) {
      const [b, o, t] = [base.get(path), mine.get(path), other.get(path)]
      let text: string | undefined
      if (o === t || t === b) text = o
      else if (o === b) text = t
      else {
        conflicts = true
        text = `<<<<<<< ${ours}\n${o ?? ''}=======\n${t ?? ''}>>>>>>> ${theirs}\n`
      }
      if (text !== undefined) merged.set(path, text)
    }
    const tree = treeId(merged)
    this.trees.set(tree, merged)
    return Promise.resolve({ conflicts, tree })
  }

  /** The first ancestor of `b`, itself included, that is also an ancestor of `a`. */
  async mergeBase(_cwd: string, a: string, b: string): Promise<string> {
    const ofA = new Set(this.ancestors(a))
    return Promise.resolve(this.ancestors(b).find((id) => ofA.has(id)) ?? '')
  }

  async treeFiles(_cwd: string, commit: string): Promise<readonly string[]> {
    return Promise.resolve([...this.treeOfCommit(commit).keys()].sort())
  }

  async fileAt(_cwd: string, commit: string, path: string): Promise<string> {
    const text = this.treeOfCommit(commit).get(path)
    if (text === undefined) throw new Error(`fatal: path '${path}' does not exist in '${commit}'`)
    return Promise.resolve(text)
  }

  /** Each changed file's hunks, printed the way `hunks` says. */
  async diffCommits(_cwd: string, base: string, head: string): Promise<string> {
    return Promise.resolve(fileDiff(this.treeOfCommit(base), this.treeOfCommit(head), this.hunks))
  }

  async changedPaths(_cwd: string, base: string, head: string): Promise<readonly string[]> {
    return Promise.resolve(this.changed(this.treeOfCommit(base), this.treeOfCommit(head)))
  }

  /** `<mode> <blob> 0\t<path>\0` per staged file, sorted by path, as `git ls-files --stage -z` prints it. */
  async stagedEntries(): Promise<string> {
    return Promise.resolve(entriesOf(this.index))
  }

  /** HEAD against the index, or against the working tree for every path HEAD or the index tracks, in `diffCommits`' form. */
  async diffAgainstHead(_cwd: string, from: 'index' | 'working-tree'): Promise<string> {
    return Promise.resolve(fileDiff(this.headTree(), await this.against(from), this.hunks))
  }

  async pathsAgainstHead(_cwd: string, from: 'index' | 'working-tree'): Promise<readonly string[]> {
    return this.changed(this.headTree(), await this.against(from))
  }

  /** Files on disk the index does not hold. */
  async untrackedPaths(): Promise<readonly string[]> {
    return [...(await this.workingTree()).keys()].filter((path) => !this.index.has(path)).sort()
  }

  async treeOf(cwd: string, revision: string): Promise<string> {
    return treeId(this.treeOfCommit(await this.resolveCommit(cwd, revision)))
  }

  async writeIndexTree(): Promise<string> {
    this.indexTreeWrites += 1
    const id = treeId(this.index)
    this.trees.set(id, this.index)
    return Promise.resolve(id)
  }

  /** A commit off HEAD's line; the same tree on the same parent is the same commit, as with git's fixed identity and time. */
  async commitTree(_cwd: string, tree: string, parent: string): Promise<string> {
    const files = this.trees.get(tree)
    if (files === undefined) throw new Error(`fatal: ${tree} is not a valid 'tree' object`)
    const same = this.history.find((entry) => entry.parents[0] === parent && entry.id.startsWith('s') && treeId(entry.tree) === tree)
    if (same !== undefined) return Promise.resolve(same.id)
    const id = `s${String(this.history.filter((entry) => entry.id.startsWith('s')).length + 1)}`
    this.history.push({ id, parents: [parent], tree: files })
    return Promise.resolve(id)
  }

  private changed(before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>): readonly string[] {
    return changedBetween(before, after)
  }

  /** The index, or the working tree narrowed to the paths HEAD or the index tracks. */
  private async against(from: 'index' | 'working-tree'): Promise<ReadonlyMap<string, string>> {
    if (from === 'index') return this.index
    const disk = await this.workingTree()
    const tracked = new Map<string, string>()
    for (const path of new Set([...this.headTree().keys(), ...this.index.keys()])) {
      const text = disk.get(path)
      if (text !== undefined) tracked.set(path, text)
    }
    return tracked
  }

  /** A commit, then its parents, theirs, and so on back to the first commit, nearest first. */
  private ancestors(commit: string): readonly string[] {
    const seen: string[] = []
    const queue = [commit]
    for (let id = queue.shift(); id !== undefined; id = queue.shift()) {
      if (seen.includes(id)) continue
      seen.push(id)
      queue.push(...(this.history.find((entry) => entry.id === id)?.parents ?? []))
    }
    return seen
  }

  /** A commit, its first parent, and so on back to the first commit. */
  private firstParents(commit: string): string[] {
    const chain: string[] = []
    for (let id: string | undefined = commit; id !== undefined; id = this.history.find((entry) => entry.id === id)?.parents[0]) chain.push(id)
    return chain
  }

  /** Makes the index and the files on disk outside `.git/` hold `tree`. */
  private async become(tree: ReadonlyMap<string, string>): Promise<void> {
    for (const path of (await this.workingTree()).keys()) await rm(join(this.repoRoot, path))
    for (const [path, text] of tree) {
      await mkdir(dirname(join(this.repoRoot, path)), { recursive: true })
      await writeFile(join(this.repoRoot, path), text)
    }
    this.index = tree
  }

  private headTree(): ReadonlyMap<string, string> {
    const head = this.line.at(-1)
    return head === undefined ? new Map<string, string>() : this.treeOfCommit(head)
  }

  /** A commit's tree, or a tree a merge or the index was written as. */
  private treeOfCommit(commit: string): ReadonlyMap<string, string> {
    const tree = this.history.find((entry) => entry.id === commit)?.tree ?? this.trees.get(commit)
    if (tree === undefined) throw new Error(`fatal: not a valid object name ${commit}`)
    return tree
  }

  private async workingTree(): Promise<Map<string, string>> {
    const tree = new Map<string, string>()
    for (const entry of await readdir(this.repoRoot, { recursive: true, withFileTypes: true })) {
      const path = relative(this.repoRoot, join(entry.parentPath, entry.name))
      if (entry.isDirectory() || path === '.git' || path.startsWith('.git/')) continue
      tree.set(path, await readFile(join(this.repoRoot, path), 'utf8'))
    }
    return tree
  }
}

function changedBetween(before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>): readonly string[] {
  return [...new Set([...before.keys(), ...after.keys()])].filter((path) => before.get(path) !== after.get(path)).sort()
}

/**
 * How the fake prints a changed file: as one hunk that removes every old
 * line and adds every new one, or, `by-line`, for a file whose line count
 * did not change, as one hunk per run of changed lines with no context.
 */
export type FakeHunks = 'whole-file' | 'by-line'

/** Each changed file's hunks under git's headers; an added file comes from `/dev/null` and a deleted one goes to it. */
function fileDiff(before: ReadonlyMap<string, string>, after: ReadonlyMap<string, string>, hunks: FakeHunks = 'whole-file'): string {
  const lines = (text: string | undefined) => (text === undefined || text === '' ? [] : text.replace(/\n$/, '').split('\n'))
  return changedBetween(before, after)
    .map((path) => {
      const old = before.get(path)
      const now = after.get(path)
      // git ends a `---` or `+++` path that holds a space with a tab.
      const tab = path.includes(' ') ? '\t' : ''
      const header = [
        `diff --git a/${path} b/${path}`,
        `--- ${old === undefined ? '/dev/null' : `a/${path}${tab}`}`,
        `+++ ${now === undefined ? '/dev/null' : `b/${path}${tab}`}`,
      ]
      return [...header, ...hunksOf(lines(old), lines(now), hunks)].join('\n')
    })
    .map((section) => `${section}\n`)
    .join('')
}

function hunksOf(old: readonly string[], now: readonly string[], mode: FakeHunks): readonly string[] {
  // git prints no hunk for a file that has no lines on either side.
  if (old.length === 0 && now.length === 0) return []
  if (mode === 'whole-file' || old.length !== now.length) {
    const header = hunkHeader(old.length === 0 ? 0 : 1, old.length, now.length === 0 ? 0 : 1, now.length)
    return [header, ...old.map((line) => `-${line}`), ...now.map((line) => `+${line}`)]
  }
  const out: string[] = []
  for (let i = 0; i < old.length; i += 1) {
    if (old[i] === now[i]) continue
    let end = i
    while (end < old.length && old[end] !== now[end]) end += 1
    out.push(hunkHeader(i + 1, end - i, i + 1, end - i), ...old.slice(i, end).map((line) => `-${line}`), ...now.slice(i, end).map((line) => `+${line}`))
    i = end
  }
  return out
}

/** `@@ -<start>[,<count>] +<start>[,<count>] @@`, a count of one left out the way git leaves it out. */
function hunkHeader(oldStart: number, oldCount: number, newStart: number, newCount: number): string {
  const side = (start: number, count: number) => (count === 1 ? String(start) : `${String(start)},${String(count)}`)
  return `@@ -${side(oldStart, oldCount)} +${side(newStart, newCount)} @@`
}

function entriesOf(files: ReadonlyMap<string, string>): string {
  return [...files.keys()]
    .sort()
    .map((path) => `100644 ${createHash('sha1').update(files.get(path) ?? '').digest('hex')} 0\t${path}\0`)
    .join('')
}

function treeId(files: ReadonlyMap<string, string>): string {
  return `t${createHash('sha1').update(entriesOf(files)).digest('hex').slice(0, 12)}`
}

/**
 * How the fake's issues link to each other, keyed by `<repo>#<number>` like
 * the issues: each spec's sub-issues, by number in the spec's repository,
 * and each issue's native blocked-by links, by `<repo>#<number>`. A string in
 * place of a list is the stderr of a gh failure.
 */
export interface FakeGhLinks {
  readonly subIssues?: Readonly<Record<string, readonly number[] | string>>
  readonly blockedBy?: Readonly<Record<string, readonly string[] | string>>
}

/**
 * Serves GitHub issues by `<repo>#<number>`, where `repo` is what the runner
 * passes as `--repo`. A string in place of an issue is the stderr of a gh
 * failure. An issue it does not hold fails the way gh does.
 */
export class FakeGh implements CritiqueGhPort {
  readonly requests: GhIssueRequest[] = []
  /** Every sub-issue and blocked-by read, as `<kind> <repo>#<number>`. */
  readonly linkRequests: string[] = []

  private readonly issues: Record<string, GhIssue | string>

  constructor(
    issues: Readonly<Record<string, GhIssue | string>>,
    private readonly links: FakeGhLinks = {},
  ) {
    this.issues = { ...issues }
  }

  /** Serves `issue` as `<repo>#<number>` from now on. */
  serve(key: string, issue: GhIssue | string): void {
    this.issues[key] = issue
  }

  async viewIssue(request: GhIssueRequest): Promise<GhIssue> {
    this.requests.push(request)
    const issue = this.issue(request.repo, request.number)
    return Promise.resolve(request.comments ? issue : { ...issue, comments: [] })
  }

  async subIssues(request: GhIssueLocator): Promise<readonly GhIssue[]> {
    const numbers = this.linked('sub-issues', this.links.subIssues, request)
    return Promise.resolve(numbers.map((number) => ({ ...this.issue(request.repo, number), comments: [] })))
  }

  async blockedBy(request: GhIssueLocator): Promise<readonly GhLinkedIssue[]> {
    const references = this.linked('blocked-by', this.links.blockedBy, request)
    return Promise.resolve(
      references.map((reference) => {
        const [repo = '', number = ''] = reference.split('#')
        const [host, ...path] = repo.split('/')
        return { number: Number(number), url: `https://${host ?? ''}/${path.join('/')}/issues/${number}` }
      }),
    )
  }

  private issue(repo: string, number: number): GhIssue {
    const issue = this.issues[`${repo}#${String(number)}`]
    if (issue === undefined) {
      throw new Error(
        `gh issue view ${String(number)} failed: GraphQL: Could not resolve to an issue or pull request with the number of ${String(number)}. (repository.issue)`,
      )
    }
    if (typeof issue === 'string') throw new Error(issue)
    return issue
  }

  private linked<T>(
    kind: string,
    table: Readonly<Record<string, readonly T[] | string>> | undefined,
    request: GhIssueLocator,
  ): readonly T[] {
    const key = `${request.repo}#${String(request.number)}`
    this.linkRequests.push(`${kind} ${key}`)
    const entry = table?.[key] ?? []
    if (typeof entry === 'string') throw new Error(entry)
    return entry
  }
}

/** A ticket issue: a sub-issue of a spec, with its acceptance criteria. */
export function ticketIssue(number: number, title: string, body: string): GhIssue {
  return {
    body,
    comments: [],
    labels: ['type:feature'],
    number,
    title,
    url: `https://github.com/JorisJonkers-dev/agent-kit/issues/${String(number)}`,
  }
}

/** A spec issue in agent-kit's tracker, as gh reports it. */
export function specIssue(overrides: Partial<GhIssue> = {}): GhIssue {
  return {
    body: `## User Stories\r\n\r\n1. As the owner, I want widgets.\r\n\r\n3. ${SPEC_CANARY}\r\n`,
    comments: [{ author: 'joris', body: 'Amended: the window is seven seconds.', createdAt: '2026-10-30T09:00:00Z' }],
    labels: ['type:feature'],
    number: 167,
    title: 'Widgets',
    url: 'https://github.com/JorisJonkers-dev/agent-kit/issues/167',
    ...overrides,
  }
}

/** The tracker headings Pocock's setup writes into `docs/agents/issue-tracker.md`. */
export const TRACKER_DOCS = {
  github: '# Issue tracker: GitHub\n\nIssues and specs for this repo live as GitHub issues.\n',
  gitlab: '# Issue tracker: GitLab\n\nIssues and specs for this repo live as GitLab issues.\n',
  'local-markdown': '# Issue tracker: Local Markdown\n\nIssues and specs for this repo live as markdown files in `.scratch/`.\n',
} as const

export interface VibeScript {
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode: number
  /** The export Vibe writes; a function builds it from the launch, as Vibe reports its effective config from the launch's settings. */
  readonly exportJson?: unknown
  /** Plays this run only for the critic whose `--agent` this is; otherwise for the next launch. */
  readonly agent?: string
  /**
   * What the critic does while it runs: `cwd` is its working directory,
   * `repo` the repository, and `search` calls the MCP servers its launch
   * configured, as Vibe would start them (none when it has none).
   */
  readonly effect?: (where: { readonly cwd: string; readonly repo: string; readonly search: SearchHelper }) => Promise<void>
  /** Do not return until this many critics have started, so a test can see critics overlap. */
  readonly waitForStarts?: number
  /** The run never returned and the outer timer killed it: no export, exit 124, `timedOut`. */
  readonly timedOut?: boolean
}

/**
 * The export as Vibe would write it. `JSON.stringify` writes Infinity as
 * null; here it is the literal `1e999`, which JSON.parse reads back as
 * Infinity, as it would from a corrupt export.
 */
function exportText(exported: unknown): string {
  const infinite = '\u0000infinity\u0000'
  return JSON.stringify(exported, (_key, value: unknown) => (value === Infinity ? infinite : value)).replaceAll(JSON.stringify(infinite), '1e999')
}

/** Calls one tool of a server the critic's launch configured, by server name, and returns its text answer. */
export type SearchHelper = (call: { readonly server?: string; readonly tool: string; readonly arguments: Readonly<Record<string, unknown>> }) => Promise<string>

/** The MCP servers a Vibe launch configured, as its `VIBE_MCP_SERVERS` lists them. */
export function launchServers(command: ProcessCommand | undefined): readonly Record<string, unknown>[] {
  const text = command?.env?.VIBE_MCP_SERVERS
  return text === undefined ? [] : (JSON.parse(text) as Record<string, unknown>[])
}

/** The command a harness with search starts the runner's search server with. */
export const SEARCH_COMMAND = { args: ['/opt/agent-kit/skills/critique/critique.mjs'], command: '/usr/local/bin/node' } as const

/**
 * The export of a searching critic whose Vibe honoured its launch: MCP on,
 * the launch's servers as configured, and the critic agent's tool list.
 */
export function searchExport(overrides: Record<string, unknown> = {}): (command: ProcessCommand) => Record<string, unknown> {
  return (command) => {
    const base = isolatedExport({
      enable_mcp: command.env?.VIBE_ENABLE_MCP === 'true',
      enabled_tools: ['read_file', 'packet_grep', 'packet_list', 'packet_symbol'],
      mcp_servers: launchServers(command).map((server) => ({ disabled: false, ...server, args: ['<redacted>'] })),
      ...overrides,
    })
    return base
  }
}

/** Every critic launch and return, in the order they happened, across all critic clients. */
export class LaunchLog {
  readonly events: string[] = []

  /**
   * Records a critic's start, waits until `waitForStarts` critics have
   * started (for at most two seconds, so critics run one after another make
   * the test fail rather than hang), then records its end.
   */
  async play(name: string, waitForStarts = 0): Promise<void> {
    this.events.push(`start ${name}`)
    const deadline = Date.now() + 2000
    while (this.events.filter((event) => event.startsWith('start')).length < waitForStarts && Date.now() < deadline) {
      await new Promise((resolve) => setTimeout(resolve, 5))
    }
    this.events.push(`end ${name}`)
  }
}

/**
 * How the personal Vibe home is signed in: Vibe's Keychain item, its
 * `~/.vibe/.env`, the `.env` on a machine with no `security` tool to ask
 * (Linux), or not at all.
 */
export type VibeSignIn = 'keychain' | 'env-file' | 'env-file-no-keychain' | 'none'

/**
 * Plays canned Vibe runs: writes export.json into --output-dir and returns
 * stdout and exit code. It also answers the Keychain lookup the Vibe
 * readiness check makes with `security`; `commands` holds the critic
 * launches only, `checks` those lookups.
 */
export class FakeVibeProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []
  readonly checks: ProcessCommand[] = []
  /** Every session the runner opened with its search server, and every `pkill` that stopped one. */
  readonly searches: ProcessCommand[] = []
  readonly stops: ProcessCommand[] = []
  /**
   * How the search server starts: as the runner's own code, in process, or
   * not at all (`broken`), or only for the reach proof's packet and never
   * on a critic's copy (`broken-on-copies`).
   */
  searchBehaviour: 'serve' | 'broken' | 'broken-on-copies' = 'serve'

  constructor(
    private readonly repo: string,
    private readonly scripts: VibeScript[],
    private readonly log: LaunchLog = new LaunchLog(),
    private readonly signIn: VibeSignIn = 'keychain',
    /** What `vibe --version` prints; null when vibe is not installed, so the call fails. */
    public version: string | null = 'vibe 3.0.4',
  ) {}

  /** Adds runs to play, for a test that needs the prepared run's paths to write them. */
  queue(...scripts: VibeScript[]): void {
    this.scripts.push(...scripts)
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    if (command.command === 'pkill') {
      this.stops.push(command)
      return { exitCode: 1, stderr: '', stdout: '' }
    }
    if (command.args.includes('search-server')) {
      this.searches.push(command)
      const proof = command.args[command.args.indexOf('--root') + 1]?.endsWith(join('search-proof', 'packet')) === true
      if (this.searchBehaviour === 'broken' || (this.searchBehaviour === 'broken-on-copies' && !proof)) return { exitCode: 1, stderr: 'Error: Cannot find module critique.mjs\n', stdout: '' }
      return serveInProcess(command.args, command.stdin ?? '')
    }
    if (command.command === 'security') {
      this.checks.push(command)
      if (this.signIn === 'env-file-no-keychain') throw new Error('spawn security ENOENT')
      return this.signIn === 'keychain'
        ? { exitCode: 0, stderr: '', stdout: 'keychain: "/Users/me/Library/Keychains/login.keychain-db"\n' }
        : { exitCode: 44, stderr: 'security: SecKeychainSearchCopyNext: The specified item could not be found in the keychain.\n', stdout: '' }
    }
    if (command.args.length === 1 && command.args[0] === '--version') {
      this.checks.push(command)
      if (this.version === null) throw new Error(`spawn ${command.command} ENOENT`)
      return { exitCode: 0, stderr: '', stdout: `${this.version}\n` }
    }
    this.commands.push(command)
    const agent = command.args[command.args.indexOf('--agent') + 1]
    const named = this.scripts.findIndex((script) => script.agent === agent)
    const index = named === -1 ? this.scripts.findIndex((script) => script.agent === undefined) : named
    const [script] = index === -1 ? [] : this.scripts.splice(index, 1)
    if (script === undefined) throw new Error('unexpected process launch')
    await this.log.play('vibe', script.waitForStarts)
    const outputDirIndex = command.args.indexOf('--output-dir')
    const outputDir = command.args[outputDirIndex + 1]
    if (script.exportJson !== undefined && outputDir !== undefined) {
      await mkdir(outputDir, { recursive: true })
      const exported: unknown = typeof script.exportJson === 'function' ? (script.exportJson as (launch: ProcessCommand) => unknown)(command) : script.exportJson
      await writeFile(join(outputDir, 'export.json'), exportText(exported))
    }
    await script.effect?.({ cwd: command.cwd ?? '', repo: this.repo, search: searchHelper(command) })
    if (script.timedOut === true) return { exitCode: 124, stderr: script.stderr ?? '', stdout: script.stdout, timedOut: true }
    return { exitCode: script.exitCode, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

/** Runs the runner's search server in this process on the given stdin, as `critique.mjs search-server` would run. */
async function serveInProcess(args: readonly string[], stdin: string): Promise<ProcessResult> {
  let stdout = ''
  let stderr = ''
  const exitCode = await runSearchServer(args.slice(args.indexOf('search-server') + 1), {
    stdin: Readable.from([stdin]),
    writeStderr: (text) => {
      stderr += text
    },
    writeStdout: (line) => {
      stdout += line
    },
  })
  return { exitCode, stderr, stdout }
}

/** Calls a server of the launch the way Vibe's MCP client would: initialize, then the one call. */
function searchHelper(command: ProcessCommand): SearchHelper {
  return async (call) => {
    const server = launchServers(command).find((entry) => entry.name === (call.server ?? 'packet') && entry.disabled !== true)
    if (server === undefined) throw new Error(`the launch configured no live MCP server named ${call.server ?? 'packet'}`)
    const requests = [
      { id: 1, jsonrpc: '2.0', method: 'initialize', params: { protocolVersion: '2025-11-25' } },
      { jsonrpc: '2.0', method: 'notifications/initialized' },
      { id: 2, jsonrpc: '2.0', method: 'tools/call', params: { arguments: call.arguments, name: call.tool } },
    ]
    const result = await serveInProcess(server.args as string[], requests.map((request) => `${JSON.stringify(request)}\n`).join(''))
    const reply = result.stdout.split('\n').map((line) => (line === '' ? undefined : (JSON.parse(line) as Record<string, unknown>))).find((line) => line?.id === 2)
    const content = (reply?.result as { content?: { text: string }[] } | undefined)?.content
    return content?.map((part) => part.text).join('') ?? JSON.stringify(reply?.error)
  }
}

export interface AgyScript {
  readonly stdout: string
  readonly stderr?: string
  readonly exitCode?: number
  /** Do not return until this many critics have started. */
  readonly waitForStarts?: number
  /** What the critic does while it runs: `cwd` is its working directory, `repo` the repository. */
  readonly effect?: (where: { readonly cwd: string; readonly repo: string }) => Promise<void>
  /** The run never returned and the outer timer killed it. */
  readonly timedOut?: boolean
}

/**
 * What `agy models` answers, as agy 1.4.12 does: `signed-in` lists the
 * models (`gemini-3.8-flash-high` among them), `signed-out` exits 1 asking for
 * a sign-in, `spawn-error` cannot start, and a result is returned as it is.
 */
export type AgyModelsReply = 'signed-in' | 'signed-out' | 'spawn-error' | ProcessResult

const AGY_MODELS_FETCHING = 'Fetching available models...\n'

const AGY_MODELS: Readonly<Record<'signed-in' | 'signed-out', ProcessResult>> = {
  'signed-in': {
    exitCode: 0,
    stderr: AGY_MODELS_FETCHING,
    stdout: [
      'gemini-3.8-flash-high\tGemini 3.8 Flash (High)',
      'gemini-3.8-flash-medium\tGemini 3.8 Flash (Medium)',
      'gemini-3.8-flash-low\tGemini 3.8 Flash (Low)',
      'claude-sonnet-4-6\tClaude Sonnet 4.6 (Thinking)',
      '',
    ].join('\n'),
  },
  'signed-out': {
    exitCode: 1,
    stderr: `${AGY_MODELS_FETCHING}Error: Please sign in to view available models. Launch the CLI without arguments to sign in.\n`,
    stdout: '',
  },
}

/**
 * Plays canned agy runs, and answers the `agy models` call the Antigravity
 * readiness check makes.
 */
export class FakeAgyProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(
    private readonly repo: string,
    private readonly scripts: AgyScript[],
    private readonly models: AgyModelsReply,
    private readonly log: LaunchLog = new LaunchLog(),
    /** What `agy --version` prints; null when agy is not installed, so the call fails. */
    public version: string | null = '1.4.12',
  ) {}

  /** The critic launches only, without the readiness checks and version reads. */
  get launches(): readonly ProcessCommand[] {
    return this.commands.filter((command) => !isAgyProbe(command))
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    if (command.args.length === 1 && command.args[0] === '--version') {
      if (this.version === null) throw new Error(`spawn ${command.command} ENOENT`)
      return { exitCode: 0, stderr: '', stdout: `${this.version}\n` }
    }
    if (command.args.length === 1 && command.args[0] === 'models') {
      if (this.models === 'spawn-error') throw new Error(`spawn ${command.command} EACCES`)
      return typeof this.models === 'string' ? AGY_MODELS[this.models] : this.models
    }
    const script = this.scripts.shift()
    if (script === undefined) throw new Error('unexpected agy launch')
    await this.log.play('agy', script.waitForStarts)
    await script.effect?.({ cwd: command.cwd ?? '', repo: this.repo })
    if (script.timedOut === true) return { exitCode: 124, stderr: script.stderr ?? '', stdout: script.stdout, timedOut: true }
    return { exitCode: script.exitCode ?? 0, stderr: script.stderr ?? '', stdout: script.stdout }
  }
}

/** `agy --version` and `agy models`: the calls that read agy without starting a critic. */
function isAgyProbe(command: ProcessCommand): boolean {
  return command.args.length === 1 && (command.args[0] === '--version' || command.args[0] === 'models')
}

/** The `result` event agy prints last, for a run that returned the given structured output. */
export function agyResult(structured: unknown, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    conversation_id: 'c0ffee00-0000-4000-8000-000000000000',
    status: 'SUCCESS',
    response: 'I read both files and report what I found.\n',
    duration_seconds: 21.5,
    num_turns: 2,
    usage: { input_tokens: 9370, output_tokens: 812, thinking_tokens: 300, cache_read_tokens: 0, total_tokens: 10482 },
    ...(structured === undefined ? {} : { structured_output: structured }),
    ...overrides,
  }
}

/** agy's `stream-json` output: the init event, one tool step per named tool, then the result event. */
export function agyStream(result: Record<string, unknown>, tools: readonly string[] = ['read_file', 'read_file']): string {
  const id = 'c0ffee00-0000-4000-8000-000000000000'
  const events: unknown[] = [
    { event: 'init', conversation_id: id, init: { model: 'gemini-3.8-flash-high', cwd: '/packet', tools: ['read_file'], permission_mode: 'request-review' } },
    { event: 'step_update', step_update: { conversation_id: id, step_index: 0, state: 'DONE', step_type: 'user_input' } },
    ...tools.flatMap((tool, i) => [
      { event: 'step_update', step_update: { conversation_id: id, step_index: i + 1, state: 'ACTIVE', step_type: 'tool', tool_name: tool, tool_info: { name: tool, parameters: {} } } },
      { event: 'step_update', step_update: { conversation_id: id, step_index: i + 1, state: 'DONE', step_type: 'tool', tool_name: tool, tool_info: { name: tool, parameters: {} } } },
    ]),
    { event: 'step_update', step_update: { conversation_id: id, step_index: tools.length + 1, state: 'DONE', step_type: 'agent_response', text_delta: 'Done.' } },
    { event: 'result', result },
  ]
  return `${events.map((event) => JSON.stringify(event)).join('\n')}\n`
}

/** An agy run that returned the given findings as structured output. */
export function agyFindings(value: unknown, tools?: readonly string[]): AgyScript {
  return { stdout: agyStream(agyResult(value), tools) }
}

export function vibeStdout(finalText: string): string {
  return JSON.stringify([
    { type: 'message', role: 'user', content: [{ type: 'text', text: 'prompt' }] },
    { type: 'effect', detail: { toolName: 'read_file' }, state: { status: 'completed' } },
    { type: 'message', role: 'assistant', content: [{ type: 'text', text: 'Reading.' }] },
    {
      type: 'message',
      role: 'assistant',
      generationStatus: 'completed',
      content: [{ type: 'text', text: finalText }],
      outcome: { type: 'committed' },
    },
  ])
}

export function isolatedExport(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    outcome: 'finished',
    stop_reason: null,
    error: null,
    exit_code: 0,
    cost_usd: 0.0123,
    steps: 3,
    config: {
      active_model: 'mistral-large-4-0',
      enable_mcp: false,
      enabled_tools: ['read_file'],
      include_project_context: false,
      outside_workdir: 'deny',
      ...overrides,
    },
  }
}

export function fenced(value: unknown): string {
  return `I read both files.\n\n\`\`\`json\n${JSON.stringify(value, null, 2)}\n\`\`\`\n`
}

export function finding(overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    id: 'F1',
    severity: 'major',
    category: 'contradiction',
    check: 'contradiction',
    location: 'User story 3',
    claim: 'The spec contradicts the recorded decision about when widgets are frobnicated.',
    evidence: [
      { kind: 'quote', file: 'artifact/spec.md', value: SPEC_CANARY },
      { kind: 'quote', file: 'references/CONTEXT.md', value: DECISION_CANARY },
    ],
    suggestion: 'Say that only the nightly job frobnicates widgets.',
    ...overrides,
  }
}

/** Two Vibe critics of different families, both in the spec stage's base roster. */
export function twoCriticRouting(): unknown {
  return {
    clients: {
      'vibe-glm': { agent: 'critic-glm', binary: 'vibe', family: 'zhipu', kind: 'vibe', model: 'glm-5' },
      'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
    },
    stages: {
      spec: {
        base: [
          { client: 'vibe-mistral-large', lenses: ['fidelity'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
          { client: 'vibe-glm', lenses: ['testability'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 },
        ],
        distinct_families: true,
      },
    },
    version: 1,
  }
}

/** One Vibe critic on each code stage, so prepare can build a code packet and fan-out can run it. */
export function codeStageRouting(): unknown {
  const slot = { client: 'vibe-mistral-large', lenses: ['failure-modes'], max_price_usd: 0.5, role: 'spec-redteam', time_limit_s: 60 }
  return {
    clients: {
      'vibe-mistral-large': { agent: 'critic-mistral-large', binary: 'vibe', family: 'mistral', kind: 'vibe', model: 'mistral-large-4-0' },
    },
    stages: {
      'feature-review': { base: [slot], distinct_families: true },
      'ticket-review': { base: [slot], distinct_families: true },
    },
    version: 1,
  }
}

export interface Harness {
  readonly home: string
  readonly repo: string
  readonly assetsRoot: string
  readonly git: FakeGit
  readonly gh: FakeGh
  readonly process: FakeVibeProcess
  readonly agy: FakeAgyProcess
  readonly launches: LaunchLog
  readonly clock: FixedClock
  readonly commands: CritiqueCommands
  readonly cleanup: () => Promise<void>
  readonly readText: (path: string) => Promise<string>
  readonly writeText: (path: string, content: string) => Promise<void>
  readonly removeFile: (path: string) => Promise<void>
}

/** The `critic-<model>` agents setup installs (registry `vibe_critic_agents:`). */
export const VIBE_CRITIC_AGENTS = ['critic-mistral-large', 'critic-mistral-small', 'critic-glm', 'critic-deepseek-flash'] as const

export const AGENT_KIT_REMOTE: GitRemote = {
  name: 'origin',
  url: 'git@github.com:JorisJonkers-dev/agent-kit.git',
}

export async function harness(
  options: {
    readonly remotes?: readonly GitRemote[]
    readonly scripts?: VibeScript[]
    readonly agyScripts?: AgyScript[]
    /** Replaces the shipped routing.json in a private copy of the assets. */
    readonly routing?: unknown
    /** Rewrites shipped asset files, by path under the critique skill, in a private copy of the assets. */
    readonly assets?: Readonly<Record<string, (shipped: string) => string>>
    /**
     * This machine's routing override in `~/.config/agent-kit/critique-routing.json`:
     * a value is written as JSON, a string as it is.
     */
    readonly routingOverride?: unknown
    /** The platform the runner reports; Antigravity is set up on macOS only. */
    readonly platform?: NodeJS.Platform
    /** Whether `agy` is on PATH. */
    readonly agyOnPath?: boolean
    /** What `agy models` answers; signed in unless named. */
    readonly agyModels?: AgyModelsReply
    /** Whether `vibe` is on PATH. */
    readonly vibeOnPath?: boolean
    /** How the personal Vibe home is signed in. */
    readonly vibeSignIn?: VibeSignIn
    /** What `vibe --version` prints when vibe is on PATH. */
    readonly vibeVersion?: string
    /** What `agy --version` prints when agy is on PATH. */
    readonly agyVersion?: string
    /** The critic agents setup installed into `~/.vibe/agents/`; every one setup declares unless named. */
    readonly vibeAgents?: readonly string[]
    /**
     * What each installed agent file holds: the one agent-kit's generator
     * renders for it unless named, or this text for every agent (an older
     * agent file, say).
     */
    readonly vibeAgentFile?: string
    /**
     * The issue-tracker doc Pocock's setup wrote, or `none` for a repository
     * without one. Local markdown unless named, so the `.scratch/` spec is the
     * tracker's.
     */
    readonly tracker?: keyof typeof TRACKER_DOCS | 'none'
    /** The issues gh serves, by `<repo>#<number>`. */
    readonly issues?: Readonly<Record<string, GhIssue | string>>
    /** How those issues link: sub-issues and native blocked-by links. */
    readonly links?: FakeGhLinks
    /** Further environment variables the runner sees, such as `CRITIQUE_OFF`. */
    readonly env?: Readonly<Record<string, string>>
    /** Gives Vibe critics the runner's search server, served in process by the runner's own code. */
    readonly search?: boolean
    /** The personal Vibe home's `config.toml`. */
    readonly vibeConfig?: string
  } = {},
): Promise<Harness> {
  const base = await mkdtemp(join(tmpdir(), 'critique-'))
  let assetsRoot = ASSETS_ROOT
  if (options.routing !== undefined || options.assets !== undefined) {
    assetsRoot = join(base, 'assets')
    await cp(ASSETS_ROOT, assetsRoot, { recursive: true })
    if (options.routing !== undefined) await writeFile(join(assetsRoot, 'routing.json'), JSON.stringify(options.routing))
    for (const [path, rewrite] of Object.entries(options.assets ?? {})) {
      await writeFile(join(assetsRoot, path), rewrite(await readFile(join(assetsRoot, path), 'utf8')))
    }
  }
  const home = join(base, 'home')
  if (options.routingOverride !== undefined) {
    await mkdir(join(home, '.config', 'agent-kit'), { recursive: true })
    const override = options.routingOverride
    await writeFile(
      join(home, '.config', 'agent-kit', 'critique-routing.json'),
      typeof override === 'string' ? override : JSON.stringify(override),
    )
  }
  const repo = join(base, 'repo')
  await mkdir(join(repo, '.git'), { recursive: true })
  await mkdir(join(repo, '.scratch', 'widgets'), { recursive: true })
  await writeFile(
    join(repo, '.scratch', 'widgets', 'spec.md'),
    `# Widgets\n\n## User Stories\n\n1. As the owner, I want widgets.\n\n3. ${SPEC_CANARY}\n`,
  )
  await writeFile(join(repo, 'CONTEXT.md'), `# Glossary\n\n${DECISION_CANARY}\n`)
  const bin = join(base, 'bin')
  await mkdir(bin, { recursive: true })
  if (options.agyOnPath ?? true) await writeFile(join(bin, 'agy'), '#!/bin/sh\n')
  if (options.vibeOnPath ?? true) await writeFile(join(bin, 'vibe'), '#!/bin/sh\n')
  const vibeHome = join(home, '.vibe')
  await mkdir(join(vibeHome, 'agents'), { recursive: true })
  for (const agent of options.vibeAgents ?? VIBE_CRITIC_AGENTS) {
    const rendered = await readFile(join(GENERATED_AGENTS, `${agent}.toml`), 'utf8').catch(() => '')
    await writeFile(join(vibeHome, 'agents', `${agent}.toml`), options.vibeAgentFile ?? rendered)
  }
  if (options.vibeConfig !== undefined) await writeFile(join(vibeHome, 'config.toml'), options.vibeConfig)
  if (options.vibeSignIn === 'env-file' || options.vibeSignIn === 'env-file-no-keychain') await writeFile(join(vibeHome, '.env'), "MISTRAL_API_KEY='sk-personal'\n")
  const tracker = options.tracker ?? 'local-markdown'
  if (tracker !== 'none') {
    await mkdir(join(repo, 'docs', 'agents'), { recursive: true })
    await writeFile(join(repo, 'docs', 'agents', 'issue-tracker.md'), TRACKER_DOCS[tracker])
  }
  const git = new FakeGit(repo, options.remotes ?? [AGENT_KIT_REMOTE])
  await git.commit()
  const gh = new FakeGh(options.issues ?? {}, options.links)
  const launches = new LaunchLog()
  const vibeVersion = (options.vibeOnPath ?? true) ? (options.vibeVersion ?? 'vibe 3.0.4') : null
  const process = new FakeVibeProcess(repo, options.scripts ?? [], launches, options.vibeSignIn ?? 'keychain', vibeVersion)
  const agyVersion = (options.agyOnPath ?? true) ? (options.agyVersion ?? '1.4.12') : null
  const agy = new FakeAgyProcess(repo, options.agyScripts ?? [], options.agyModels ?? 'signed-in', launches, agyVersion)
  const files = new NodeCritiqueFiles()
  const env = new FakeEnv({ HOME: home, PATH: [join(base, 'nothing-here'), bin].join(':'), ...options.env })
  const clock = new FixedClock('2026-10-30T12:00:00.000Z')
  const commands = createCritiqueCommands({
    assetsRoot,
    clients: {
      agy: new AgyCriticClient(agy, files, env, options.platform ?? 'darwin'),
      vibe: new VibeCriticClient(process, files, env),
    },
    clock,
    env,
    files,
    gh,
    git,
    ...(options.search === true ? { search: new ProcessSearchClient(process, SEARCH_COMMAND) } : {}),
  })
  return {
    agy,
    assetsRoot,
    cleanup: () => rm(base, { force: true, recursive: true }),
    clock,
    commands,
    gh,
    git,
    home,
    launches,
    process,
    readText: (path) => readFile(path, 'utf8'),
    removeFile: (path) => rm(path),
    repo,
    writeText: async (path, content) => {
      await mkdir(dirname(path), { recursive: true })
      await writeFile(path, content)
    },
  }
}

/** The model and family of each shipped critic client, as record writes them to the ledger. */
const LEDGER_CLIENTS: Readonly<Record<string, { readonly model: string; readonly family: string }>> = {
  'agy-gemini-flash-high': { family: 'google', model: 'gemini-3.8-flash-high' },
  'vibe-glm': { family: 'zhipu', model: 'zai-glm-5-3' },
  'vibe-mistral-large': { family: 'mistral', model: 'mistral-large-4-0' },
}

/** One critic of a synthetic run. `role_hash` and `lens_hash` stand in for the hashes of its role and lens files. */
export interface SyntheticCritic {
  readonly client: string
  readonly role_hash?: string
  readonly lens_hash?: string
  readonly cost_usd?: number | null
  readonly status?: 'ok' | 'failed' | 'violated-readonly'
  readonly failure?: string
}

/** One finding of a synthetic run, raised by the critics of the named clients. */
export interface SyntheticFinding {
  readonly raised_by: readonly string[]
  readonly verdict: 'accepted' | 'rejected' | 'needs-human' | 'hint'
  readonly pre_verdict?: string
}

export interface SyntheticRun {
  readonly critics: readonly SyntheticCritic[]
  readonly findings?: readonly SyntheticFinding[]
  readonly stage?: string
  /** The spec reference; null leaves it out, as on lines recorded before the ledger named the spec. */
  readonly spec?: string | null
  readonly repository?: string
  /** When record wrote the run; one minute after the previous run by default. */
  readonly recorded_at?: string
}

/** One critic launch as fan-out appends it to the ledger. */
export interface SyntheticInvocation {
  readonly kind: 'vibe' | 'agy'
  readonly flow: string
  /** What the launch was charged; 0 unless named, as for every agy launch. */
  readonly charged_usd?: number
  readonly recorded_at?: string
  readonly repository?: string
  readonly stage?: string
}

/**
 * Builds a user ledger the way record writes one: per run, one `run` line
 * per critic, then one `finding` line per finding, metadata only.
 */
export class SyntheticLedger {
  private readonly lines: Record<string, unknown>[] = []
  private count = 0

  /** Adds one run. */
  run(run: SyntheticRun): this {
    this.count += 1
    const recordedAt = run.recorded_at ?? new Date(Date.UTC(2026, 9, 1) + this.count * 60_000).toISOString()
    const common = {
      depth: 'base',
      recorded_at: recordedAt,
      repository: run.repository ?? 'github.com/jorisjonkers-dev/agent-kit',
      run_id: `run-${String(this.count)}`,
      stage: run.stage ?? 'spec',
      ...(run.spec === null ? {} : { spec: run.spec ?? 'github.com/jorisjonkers-dev/agent-kit#167' }),
    }
    const ids = run.critics.map((critic, index) => `c${String(index + 1)}-${critic.client}`)
    const identity = (index: number) => {
      const critic = run.critics[index]
      if (critic === undefined) throw new Error(`no critic ${String(index)}`)
      const client = LEDGER_CLIENTS[critic.client] ?? { family: 'mistral', model: critic.client }
      return {
        client: critic.client,
        critic_id: ids[index],
        family: client.family,
        lens_hashes: { fidelity: critic.lens_hash ?? 'lens-1' },
        lenses: ['fidelity'],
        model: client.model,
        prompt_hashes: { contract: 'contract-1', preamble: 'preamble-1', role: critic.role_hash ?? 'role-1', schema: 'schema-1' },
        role: 'spec-redteam',
      }
    }
    run.critics.forEach((critic, index) => {
      const status = critic.status ?? 'ok'
      this.lines.push({
        type: 'run',
        ...common,
        ...identity(index),
        cost_usd: critic.cost_usd === undefined ? 0.01 : critic.cost_usd,
        duration_ms: 1000,
        exit_code: status === 'ok' ? 0 : 1,
        failure: critic.failure ?? null,
        finding_count: 0,
        status,
      })
    })
    ;(run.findings ?? []).forEach((finding, index) => {
      const raisers = finding.raised_by.map((client) => run.critics.findIndex((critic) => critic.client === client))
      this.lines.push({
        type: 'finding',
        ...common,
        ...identity(Math.max(raisers[0] ?? 0, 0)),
        category: 'contradiction',
        evidence_kinds: ['quote'],
        finding_id: `F${String(index + 1)}`,
        pre_verdict: finding.pre_verdict ?? null,
        raised_by: raisers.map((raiser) => ids[raiser] ?? `c0-${String(raiser)}`),
        raised_by_families: [],
        reason_code: finding.pre_verdict ?? null,
        severity: 'major',
        verdict: finding.verdict,
      })
    })
    return this
  }

  /**
   * Adds one critic launch the way fan-out logs it. Recorded an hour before
   * the harness clock unless named, so it counts as today's.
   */
  invocation(launch: SyntheticInvocation): this {
    this.count += 1
    const client = launch.kind === 'vibe' ? 'vibe-mistral-large' : 'agy-gemini-flash-high'
    this.lines.push({
      charged_usd: launch.charged_usd ?? 0,
      client,
      critic_id: `c1-${client}`,
      family: LEDGER_CLIENTS[client]?.family,
      flow: launch.flow,
      kind: launch.kind,
      model: LEDGER_CLIENTS[client]?.model,
      recorded_at: launch.recorded_at ?? '2026-10-30T11:00:00.000Z',
      repository: launch.repository ?? 'github.com/jorisjonkers-dev/agent-kit',
      round: 1,
      run_id: `run-${String(this.count)}`,
      stage: launch.stage ?? 'spec',
      status: 'ok',
      type: 'invocation',
    })
    return this
  }

  /** Adds a line as it is, for a ledger a later version or a broken write left. */
  raw(line: unknown): this {
    this.lines.push(line as Record<string, unknown>)
    return this
  }

  text(): string {
    return this.lines.map((line) => `${JSON.stringify(line)}\n`).join('')
  }

  /** Writes the ledger where the runner reads it in the harness's HOME, and returns its path. */
  async writeTo(h: Pick<Harness, 'home' | 'writeText'>): Promise<string> {
    const path = join(h.home, '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl')
    await h.writeText(path, this.text())
    return path
  }
}
