import { isAbsolute, resolve } from 'node:path'

/**
 * The words of each simple command in a shell command line, as far as a
 * gate needs them: quotes are removed, command substitutions (`$(...)` and
 * backticks) and here-document bodies are skipped, redirections and their
 * targets are dropped, and `&&`, `||`, `;`, `|`, `&`, parentheses and new
 * lines end a command. It reads the text only; nothing is expanded or run.
 */
export function simpleCommands(line: string): readonly (readonly string[])[] {
  return new ShellScanner(line).commands()
}

class ShellScanner {
  private i = 0
  private readonly done: string[][] = []
  private words: string[] = []
  private word: string | undefined
  private skipNextWord = false
  /** Here-documents whose bodies start after the current line. */
  private heredocs: { readonly delimiter: string; readonly stripTabs: boolean }[] = []

  constructor(private readonly text: string) {}

  commands(): readonly (readonly string[])[] {
    while (this.i < this.text.length) this.step()
    this.endCommand()
    return this.done
  }

  private step(): void {
    const char = this.text.charAt(this.i)
    if (char === '\n') {
      this.endCommand()
      this.i += 1
      this.i = skipHeredocBodies(this.text, this.i, this.heredocs.splice(0))
    } else if (char === ' ' || char === '\t') {
      this.endWord()
      this.i += 1
    } else if (';&|()'.includes(char)) {
      this.endCommand()
      this.i += 1
    } else if (char === '<' || char === '>') {
      this.redirection()
    } else if (char === '#' && this.word === undefined) {
      while (this.i < this.text.length && this.text.charAt(this.i) !== '\n') this.i += 1
    } else if (char === "'") {
      const end = this.text.indexOf("'", this.i + 1)
      const stop = end === -1 ? this.text.length : end
      this.append(this.text.slice(this.i + 1, stop))
      this.i = stop + 1
    } else if (char === '"') {
      this.i = this.doubleQuoted(this.i + 1)
    } else if (char === '\\') {
      if (this.text.charAt(this.i + 1) !== '\n') this.append(this.text.charAt(this.i + 1))
      this.i += 2
    } else if (char === '`' || this.text.startsWith('$(', this.i)) {
      this.append('')
      this.i = skipSubstitution(this.text, this.i)
    } else {
      this.append(char)
      this.i += 1
    }
  }

  /** Reads a double-quoted string from just after its opening quote; returns the index after its closing one. */
  private doubleQuoted(start: number): number {
    let i = start
    this.append('')
    while (i < this.text.length) {
      const char = this.text.charAt(i)
      if (char === '"') return i + 1
      if (char === '\\' && '"\\$`\n'.includes(this.text.charAt(i + 1))) {
        if (this.text.charAt(i + 1) !== '\n') this.append(this.text.charAt(i + 1))
        i += 2
      } else if (char === '`' || this.text.startsWith('$(', i)) {
        i = skipSubstitution(this.text, i)
      } else {
        this.append(char)
        i += 1
      }
    }
    return i
  }

  /** A redirection: `<<` opens a here-document; any other drops its target word. A file descriptor before it is dropped too. */
  private redirection(): void {
    if (this.word !== undefined && /^\d+$/.test(this.word)) this.word = undefined
    this.endWord()
    if (this.text.startsWith('<<', this.i) && !this.text.startsWith('<<<', this.i)) {
      this.i += 2
      const stripTabs = this.text.charAt(this.i) === '-'
      if (stripTabs) this.i += 1
      const { delimiter, end } = readDelimiter(this.text, this.i)
      this.heredocs.push({ delimiter, stripTabs })
      this.i = end
      return
    }
    while ('<>&|'.includes(this.text.charAt(this.i)) && this.i < this.text.length) this.i += 1
    // `2>&1` names a descriptor, not a file; either way the word is not an argument.
    this.skipNextWord = true
  }

  private append(text: string): void {
    this.word = (this.word ?? '') + text
  }

  private endWord(): void {
    if (this.word === undefined) return
    if (this.skipNextWord) this.skipNextWord = false
    else this.words.push(this.word)
    this.word = undefined
  }

  private endCommand(): void {
    this.endWord()
    this.skipNextWord = false
    if (this.words.length > 0) this.done.push(this.words)
    this.words = []
  }
}

/** The delimiter word of a here-document, quotes removed, and where it ends. */
function readDelimiter(text: string, start: number): { readonly delimiter: string; readonly end: number } {
  let i = start
  while (text.charAt(i) === ' ' || text.charAt(i) === '\t') i += 1
  let delimiter = ''
  while (i < text.length && !' \t\n;&|()<>'.includes(text.charAt(i))) {
    const char = text.charAt(i)
    if (char !== "'" && char !== '"' && char !== '\\') delimiter += char
    i += 1
  }
  return { delimiter, end: i }
}

/** Skips the bodies of the here-documents opened on the line before `start`, each up to its delimiter line. */
function skipHeredocBodies(text: string, start: number, heredocs: readonly { readonly delimiter: string; readonly stripTabs: boolean }[]): number {
  let i = start
  for (const heredoc of heredocs) {
    while (i < text.length) {
      const end = text.indexOf('\n', i)
      const stop = end === -1 ? text.length : end
      const line = text.slice(i, stop)
      i = stop + 1
      if ((heredoc.stripTabs ? line.replace(/^\t+/, '') : line).trim() === heredoc.delimiter) break
    }
  }
  return Math.min(i, text.length)
}

/**
 * Skips a command substitution from its `$(` or backtick; returns the index
 * after its end. Quotes, nested substitutions and here-documents inside it
 * are skipped whole, so an apostrophe or a parenthesis in a commit message
 * written through `$(cat <<'EOF' ... EOF)` ends nothing.
 */
function skipSubstitution(text: string, start: number): number {
  if (text.charAt(start) === '`') {
    let i = start + 1
    while (i < text.length && text.charAt(i) !== '`') i += text.charAt(i) === '\\' ? 2 : 1
    return i + 1
  }
  let i = start + 2
  let depth = 1
  const heredocs: { readonly delimiter: string; readonly stripTabs: boolean }[] = []
  while (i < text.length && depth > 0) {
    const char = text.charAt(i)
    if (char === '\n') {
      i = skipHeredocBodies(text, i + 1, heredocs.splice(0))
    } else if (text.startsWith('<<', i) && !text.startsWith('<<<', i)) {
      const stripTabs = text.charAt(i + 2) === '-'
      const { delimiter, end } = readDelimiter(text, i + (stripTabs ? 3 : 2))
      heredocs.push({ delimiter, stripTabs })
      i = end
    } else if (char === "'") {
      const end = text.indexOf("'", i + 1)
      i = end === -1 ? text.length : end + 1
    } else if (char === '"') {
      i += 1
      while (i < text.length && text.charAt(i) !== '"') {
        if (text.charAt(i) === '\\') i += 2
        else if (text.startsWith('$(', i) || text.charAt(i) === '`') i = skipSubstitution(text, i)
        else i += 1
      }
      i += 1
    } else if (char === '\\') {
      i += 2
    } else if (char === '`' || text.startsWith('$(', i)) {
      i = skipSubstitution(text, i)
    } else {
      if (char === '(') depth += 1
      if (char === ')') depth -= 1
      i += 1
    }
  }
  return i
}

/** A pushed ref: where it comes from (null for the current branch's commit) and the branch it updates (null for the current branch). */
export interface PushTarget {
  readonly source: string | null
  readonly branch: string | null
}

/** One command a gate stands before, and the directory it runs in (null when the command line leaves that unclear). */
export type GatedInvocation =
  | {
      readonly kind: 'commit'
      readonly cwd: string | null
      /**
       * Whether the commit may record more than the index holds now: none,
       * tracked files beyond the index (`-a`, paths, `--include`, a `git add`
       * earlier in the line), or untracked files too (an earlier `git add`).
       */
      readonly widened: 'none' | 'tracked' | 'untracked'
    }
  | {
      readonly kind: 'push'
      readonly cwd: string | null
      /** Every branch the push updates; `all` for `--all`, `--branches` or `--mirror`. */
      readonly targets: readonly PushTarget[] | 'all'
      /** An earlier command of the line may move HEAD, so what is pushed is not known yet. */
      readonly head_moves: boolean
    }
  | {
      /** A `git merge` or `git pull` that may record a merge commit without any `git commit`. */
      readonly kind: 'merge'
      readonly cwd: string | null
      /**
       * What is merged, as revisions to resolve: the commits `git merge`
       * names, `@{upstream}` for none, and for a pull the remote-tracking
       * branch of each branch it fetches. The gate reads these as they are
       * now; whatever the pull itself, or a fetch earlier in the line,
       * brings in is not known yet.
       */
      readonly revisions: readonly string[]
      /** Whether git may fast-forward instead, recording no merge commit: one revision and no `--no-ff`. */
      readonly fast_forward: boolean
      /** An earlier command of the line may move HEAD, so what the merge brings in is not known yet. */
      readonly head_moves: boolean
    }
  | {
      readonly kind: 'pr-create'
      readonly cwd: string | null
      /** The branch `--head` names; null for the current branch. */
      readonly head: string | null
      readonly head_moves: boolean
    }

/** Git commands that may change the index, so a later commit in the same line records something else. */
const INDEX_CHANGING = new Set([
  'add',
  'am',
  'apply',
  'checkout',
  'cherry-pick',
  'commit',
  'merge',
  'mv',
  'pull',
  'read-tree',
  'rebase',
  'reset',
  'restore',
  'revert',
  'rm',
  'stage',
  'stash',
  'switch',
  'update-index',
])

/** Git commands that may add untracked files to the index. */
const ADDING = new Set(['add', 'stage'])

/** Git commands that may move HEAD, so a later push or PR sends a commit the gate cannot see yet. */
const HEAD_MOVING = new Set(['am', 'checkout', 'cherry-pick', 'commit', 'merge', 'pull', 'rebase', 'reset', 'revert', 'switch'])

/** Words that run the command after them. */
const WRAPPERS = new Set(['builtin', 'command', 'env', 'exec', 'nohup', 'time'])

/** Git's global options that take the next word as their value. */
const GIT_VALUED = new Set(['-C', '-c', '--git-dir', '--work-tree', '--namespace', '--super-prefix', '--config-env', '--exec-path'])

/**
 * The commits, pushes and PR creations a command line runs, in order. `cd`
 * and `git -C` move the directory a later command runs in; a `cd` to a
 * place the text alone does not name (`cd -`, a variable) leaves it unclear.
 */
export function gatedInvocations(line: string, cwd: string, home: string | undefined): readonly GatedInvocation[] {
  const found: GatedInvocation[] = []
  let dir: string | null = cwd
  let indexChanged: 'none' | 'tracked' | 'untracked' = 'none'
  let headMoved = false
  for (const words of simpleCommands(line)) {
    const command = unwrap(words)
    const [program, ...args] = command
    if (program === 'cd') {
      dir = changeDirectory(dir, args, home)
    } else if (program === 'git') {
      const git = gitSubcommand(args, dir, home)
      const merge = git.subcommand === 'merge' || git.subcommand === 'pull' ? mergeOf(git.subcommand, git.args) : 'none'
      if ((git.subcommand === 'commit' && !git.args.includes('--dry-run')) || merge === 'continue') {
        found.push({ cwd: git.cwd, kind: 'commit', widened: widest(indexChanged, merge === 'continue' ? 'none' : commitWidening(git.args)) })
      } else if (merge !== 'none') {
        found.push({ cwd: git.cwd, head_moves: headMoved, kind: 'merge', ...merge })
      } else if (git.subcommand === 'push' && !git.args.includes('--dry-run') && !git.args.includes('-n')) {
        const targets = pushTargets(git.args)
        if (targets !== 'none') found.push({ cwd: git.cwd, head_moves: headMoved, kind: 'push', targets })
      }
      if (git.subcommand !== undefined && INDEX_CHANGING.has(git.subcommand)) {
        indexChanged = widest(indexChanged, ADDING.has(git.subcommand) ? 'untracked' : 'tracked')
      }
      if (git.subcommand !== undefined && HEAD_MOVING.has(git.subcommand)) headMoved = true
    } else if (program === 'gh' && isPrCreate(args)) {
      found.push({ cwd: dir, head: optionValue(args, ['--head', '-H']), head_moves: headMoved, kind: 'pr-create' })
    }
  }
  return found
}

/** Drops leading variable assignments and wrappers such as `env` or `command`, with their options. */
function unwrap(words: readonly string[]): readonly string[] {
  let i = 0
  for (;;) {
    const word = words[i]
    if (word === undefined) return []
    if (/^[A-Za-z_][A-Za-z0-9_]*=/.test(word)) i += 1
    else if (WRAPPERS.has(word)) {
      i += 1
      while ((words[i] ?? '').startsWith('-')) i += 1
    } else return words.slice(i)
  }
}

function changeDirectory(dir: string | null, args: readonly string[], home: string | undefined): string | null {
  const [target] = args
  if (dir === null) return null
  if (target === undefined) return home ?? null
  return within(dir, target, home)
}

/** `path` from `dir`, `~` expanded; null when the text alone does not say where it is. */
function within(dir: string | null, path: string, home: string | undefined): string | null {
  if (dir === null || path === '-' || path.includes('$')) return null
  if (path === '~' || path.startsWith('~/')) return home === undefined ? null : resolve(home, path.slice(2))
  return isAbsolute(path) ? path : resolve(dir, path)
}

/** The git subcommand after git's global options, its arguments, and the directory `-C` leaves it in. */
function gitSubcommand(
  args: readonly string[],
  dir: string | null,
  home: string | undefined,
): { readonly subcommand: string | undefined; readonly args: readonly string[]; readonly cwd: string | null } {
  let cwd = dir
  let i = 0
  while (i < args.length && (args[i] ?? '').startsWith('-')) {
    const option = args[i] ?? ''
    if (option === '-C') cwd = within(cwd, args[i + 1] ?? '', home)
    i += GIT_VALUED.has(option) ? 2 : 1
  }
  return { args: args.slice(i + 1), cwd, subcommand: args[i] }
}

/** `git commit` options whose value is the next word, or the rest of a cluster such as `-mmessage`. */
const COMMIT_VALUED_SHORT = new Set(['m', 'F', 'C', 'c', 't', 'S'])
const COMMIT_VALUED_LONG = new Set([
  '--message',
  '--file',
  '--reuse-message',
  '--reedit-message',
  '--fixup',
  '--squash',
  '--author',
  '--date',
  '--template',
  '--cleanup',
  '--trailer',
  '--pathspec-from-file',
])
/** `git commit` options that commit files beyond what the index holds. */
const COMMIT_WIDENING_SHORT = new Set(['a', 'i', 'o', 'p'])
const COMMIT_WIDENING_LONG = new Set(['--all', '--include', '--only', '--patch', '--interactive', '--pathspec-from-file'])

/** Whether a commit's own options or paths make it record more than the index holds. */
function commitWidening(args: readonly string[]): 'none' | 'tracked' {
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i] ?? ''
    if (arg === '--') return i + 1 < args.length ? 'tracked' : 'none'
    if (arg.startsWith('--')) {
      const name = arg.split('=')[0] ?? arg
      if (COMMIT_WIDENING_LONG.has(name)) return 'tracked'
      if (COMMIT_VALUED_LONG.has(arg)) i += 1
    } else if (arg.startsWith('-') && arg.length > 1) {
      for (let j = 1; j < arg.length; j += 1) {
        const flag = arg.charAt(j)
        if (COMMIT_WIDENING_SHORT.has(flag)) return 'tracked'
        if (COMMIT_VALUED_SHORT.has(flag)) {
          // `-S` takes its key id attached or not at all.
          if (j === arg.length - 1 && flag !== 'S') i += 1
          break
        }
      }
    } else {
      // A path: the commit records that file as it is on disk.
      return 'tracked'
    }
  }
  return 'none'
}

function widest(a: 'none' | 'tracked' | 'untracked', b: 'none' | 'tracked' | 'untracked'): 'none' | 'tracked' | 'untracked' {
  const order = ['none', 'tracked', 'untracked'] as const
  return order[Math.max(order.indexOf(a), order.indexOf(b))] ?? 'untracked'
}

const PUSH_VALUED = new Set(['--repo', '-o', '--push-option', '--receive-pack', '--exec'])

/**
 * The branches a push updates: `all` for `--all`, `--branches` or
 * `--mirror`; `none` for a push of tags only or a deletion, which sends no
 * branch's commits.
 */
function pushTargets(args: readonly string[]): readonly PushTarget[] | 'all' | 'none' {
  const positional: string[] = []
  let tags = false
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i] ?? ''
    if (arg === '--') {
      positional.push(...args.slice(i + 1))
      break
    }
    if (arg === '--all' || arg === '--branches' || arg === '--mirror') return 'all'
    if (arg === '--delete' || arg === '-d') return 'none'
    if (arg === '--tags') tags = true
    if (PUSH_VALUED.has(arg)) i += 1
    else if (!arg.startsWith('-')) positional.push(arg)
  }
  const refspecs = positional.slice(1)
  if (refspecs.length === 0) return tags ? 'none' : [{ branch: null, source: null }]
  const targets = refspecs.flatMap((refspec): PushTarget[] => {
    const [rawSource = '', rawDestination] = refspec.replace(/^\+/, '').split(':')
    if (rawSource === '' || rawSource.startsWith('refs/tags/') || (rawDestination ?? '').startsWith('refs/tags/')) return []
    const source = rawSource === 'HEAD' || rawSource === '@' ? null : rawSource
    const destination = rawDestination ?? source
    return [{ branch: destination?.replace(/^refs\/heads\//, '') ?? null, source }]
  })
  return targets.length === 0 ? 'none' : targets
}

/** `git merge` options whose value is the next word, or the rest of a cluster such as `-mmessage`. */
const MERGE_VALUED_SHORT = new Set(['m', 'F', 's', 'X'])
const MERGE_VALUED_LONG = new Set(['--message', '--file', '--strategy', '--strategy-option', '--into-name', '--cleanup'])
/** `git pull` options whose value is the next word. */
const PULL_VALUED_SHORT = new Set(['s', 'X', 'j', 'o'])
const PULL_VALUED_LONG = new Set([
  '--strategy',
  '--strategy-option',
  '--depth',
  '--deepen',
  '--shallow-since',
  '--shallow-exclude',
  '--upload-pack',
  '--server-option',
  '--jobs',
  '--negotiation-tip',
  '--refmap',
])
/** Options after which the command records no merge commit: it fast-forwards only, squashes, stops before committing, or ends a merge without one. */
const NO_MERGE_COMMIT = new Set(['--ff-only', '--squash', '--no-commit', '--abort', '--quit'])

/**
 * What a `git merge` or `git pull` merges, when it may record a merge
 * commit; `continue` for `git merge --continue`, which commits the
 * merge staged in the index as `git commit` does; `none` when it records
 * no merge commit (it rebases, fast-forwards only, squashes or stops
 * before committing).
 */
function mergeOf(
  subcommand: 'merge' | 'pull',
  args: readonly string[],
): { readonly revisions: readonly string[]; readonly fast_forward: boolean } | 'continue' | 'none' {
  const [valuedShort, valuedLong] = subcommand === 'merge' ? [MERGE_VALUED_SHORT, MERGE_VALUED_LONG] : [PULL_VALUED_SHORT, PULL_VALUED_LONG]
  const positional: string[] = []
  let commits = true
  let fastForward = true
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i] ?? ''
    const name = arg.split('=')[0] ?? arg
    if (arg === '--') {
      positional.push(...args.slice(i + 1))
      break
    }
    if (subcommand === 'merge' && arg === '--continue') return 'continue'
    if (NO_MERGE_COMMIT.has(arg)) commits = false
    else if (arg === '--commit' || arg === '--no-squash') commits = true
    else if (arg === '--no-ff') fastForward = false
    else if (arg === '--ff') fastForward = true
    else if (subcommand === 'pull' && (arg === '-r' || name === '--rebase')) commits = arg === '--rebase=false'
    else if (subcommand === 'pull' && arg === '--no-rebase') commits = true
    else if (arg.startsWith('--')) {
      if (valuedLong.has(arg)) i += 1
    } else if (arg.startsWith('-') && arg.length > 1) {
      // A valued flag last in its cluster takes the next word; earlier, the rest of the cluster is its value.
      let j = 1
      while (j < arg.length && !valuedShort.has(arg.charAt(j))) j += 1
      if (j === arg.length - 1) i += 1
    } else positional.push(arg)
  }
  if (!commits) return 'none'
  const revisions = subcommand === 'merge' ? positional : pulledBranches(positional)
  const merged = revisions.length === 0 ? ['@{upstream}'] : revisions
  return { fast_forward: fastForward && merged.length === 1, revisions: merged }
}

/** The remote-tracking branch of each branch `git pull <remote> <refspec>...` fetches; none when it names no refspec. */
function pulledBranches(positional: readonly string[]): readonly string[] {
  const [remote, ...refspecs] = positional
  return refspecs.map((refspec) => `${remote ?? ''}/${(refspec.replace(/^\+/, '').split(':')[0] ?? '').replace(/^refs\/heads\//, '')}`)
}

/** Whether `gh` arguments are `pr create`, after any flag. */
function isPrCreate(args: readonly string[]): boolean {
  const words = args.filter((arg) => !arg.startsWith('-'))
  return words[0] === 'pr' && words[1] === 'create'
}

/** The value of the first of `names`, as `--name value` or `--name=value`. */
function optionValue(args: readonly string[], names: readonly string[]): string | null {
  for (let i = 0; i < args.length; i += 1) {
    const arg = args[i] ?? ''
    if (names.includes(arg)) return args[i + 1] ?? null
    const name = names.find((candidate) => arg.startsWith(`${candidate}=`))
    if (name !== undefined) return arg.slice(name.length + 1)
  }
  return null
}
