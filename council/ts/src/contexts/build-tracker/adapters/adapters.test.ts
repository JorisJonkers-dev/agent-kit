import { execFile } from 'node:child_process'
import { mkdir, mkdtemp, readdir, realpath, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join } from 'node:path'
import { promisify } from 'node:util'

import { afterEach, describe, expect, it } from 'vitest'

import { NodeBuildTrackerFiles } from './fs/index.js'
import { GhCliBuildTrackerIssues } from './gh/index.js'
import { GitBuildTrackerRepositories, parseRemote } from './git/index.js'
import { NodeBuildTrackerProcess, type BuildTrackerProcess, type ProcessRequest, type ProcessResult } from './process/index.js'

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

async function tempDir(): Promise<string> {
  const dir = await realpath(await mkdtemp(join(tmpdir(), 'build-tracker-adapter-')))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  return dir
}

/** A process that answers every call with the next canned result and records what it was asked. */
class ScriptedProcess implements BuildTrackerProcess {
  readonly requests: ProcessRequest[] = []

  constructor(private readonly results: ProcessResult[]) {}

  exec(request: ProcessRequest): Promise<ProcessResult> {
    this.requests.push(request)
    return Promise.resolve(this.results.shift() ?? { exitCode: 0, stderr: '', stdout: '' })
  }
}

const ok = (value: unknown): ProcessResult => ({ exitCode: 0, stderr: '', stdout: JSON.stringify(value) })
const WIDGETS = { host: 'github.com', owner: 'acme', repo: 'widgets' }

describe('GhCliBuildTrackerIssues', () => {
  it('reads one issue through gh api on the repository host', async () => {
    const process = new ScriptedProcess([
      ok({ html_url: 'https://github.com/acme/widgets/issues/10', number: 10, state: 'open', title: 'Spec: widgets' }),
    ])

    const issue = await new GhCliBuildTrackerIssues(process).issue(WIDGETS, 10)

    expect(issue).toEqual({ number: 10, repository: WIDGETS, state: 'open', title: 'Spec: widgets' })
    expect(process.requests).toEqual([{ args: ['api', '--hostname', 'github.com', 'repos/acme/widgets/issues/10'], command: 'gh' }])
  })

  it('reads every page of sub-issues, each in the repository its URL names', async () => {
    const process = new ScriptedProcess([
      ok([
        [{ html_url: 'https://github.com/acme/widgets/issues/1', number: 1, state: 'closed', title: 'one' }],
        [{ html_url: 'https://GitHub.com/acme/gadgets/issues/2', number: 2, state: 'open', title: 'two' }, { number: 3 }],
      ]),
    ])

    const issues = await new GhCliBuildTrackerIssues(process).subIssues(WIDGETS, 10)

    expect(issues).toEqual([
      { number: 1, repository: WIDGETS, state: 'closed', title: 'one' },
      { number: 2, repository: { host: 'github.com', owner: 'acme', repo: 'gadgets' }, state: 'open', title: 'two' },
      { number: 3, repository: WIDGETS, state: 'open', title: '' },
    ])
    expect(process.requests[0]?.args).toEqual([
      'api',
      '--hostname',
      'github.com',
      '--paginate',
      '--slurp',
      'repos/acme/widgets/issues/10/sub_issues?per_page=100',
    ])
  })

  it('reads an issue blockers from its native blocked-by links', async () => {
    const process = new ScriptedProcess([ok([[{ html_url: 'https://github.com/acme/widgets/pull/4', number: 4, state: 'closed' }]])])

    const blockers = await new GhCliBuildTrackerIssues(process).blockedBy({ ...WIDGETS, host: 'ghe.example.com' }, 7)

    expect(blockers).toEqual([{ number: 4, repository: WIDGETS, state: 'closed', title: '' }])
    expect(process.requests[0]?.args).toEqual([
      'api',
      '--hostname',
      'ghe.example.com',
      '--paginate',
      '--slurp',
      'repos/acme/widgets/issues/7/dependencies/blocked_by?per_page=100',
    ])
  })

  it.each([
    ['its stderr', { exitCode: 1, stderr: '\n  HTTP 404: Not Found\nmore', stdout: '' }, 'HTTP 404: Not Found'],
    ['its stdout when stderr is empty', { exitCode: 4, stderr: '', stdout: '{"message":"Bad credentials"}' }, '{"message":"Bad credentials"}'],
    ['its exit code when it printed nothing', { exitCode: 2, stderr: ' ', stdout: '' }, 'exit code 2'],
  ])('rejects a gh failure with %s', async (_name, result, detail) => {
    const gh = new GhCliBuildTrackerIssues(new ScriptedProcess([result]))

    await expect(gh.issue(WIDGETS, 10)).rejects.toThrow(
      `gh api repos/acme/widgets/issues/10 exited ${String(result.exitCode)}: ${detail}`,
    )
  })

  it.each([
    ['text that is not JSON', 'not json'],
    ['an object instead of pages', '{"number": 1}'],
    ['pages that are not lists', '[{"number": 1}]'],
    ['an entry without a number', '[[{"title": "x"}]]'],
  ])('rejects a list that is %s', async (_name, stdout) => {
    const gh = new GhCliBuildTrackerIssues(new ScriptedProcess([{ exitCode: 0, stderr: '', stdout }]))

    await expect(gh.subIssues(WIDGETS, 10)).rejects.toThrow('gh api printed something that is not a list of issues')
  })

  it.each([
    ['text that is not JSON', 'nope'],
    ['a list', '[]'],
    ['null', 'null'],
  ])('rejects an issue that is %s', async (_name, stdout) => {
    const gh = new GhCliBuildTrackerIssues(new ScriptedProcess([{ exitCode: 0, stderr: '', stdout }]))

    await expect(gh.issue(WIDGETS, 10)).rejects.toThrow('gh api printed something that is not an issue')
  })
})

const run = promisify(execFile)

async function git(dir: string, ...args: string[]): Promise<void> {
  await run('git', ['-C', dir, ...args])
}

describe('GitBuildTrackerRepositories', () => {
  const repositories = new GitBuildTrackerRepositories(new NodeBuildTrackerProcess())

  it("finds a directory's root, its place under it and origin's location", async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')
    await git(root, 'remote', 'add', 'upstream', 'https://example.com/other/thing.git')
    await git(root, 'remote', 'add', 'origin', 'git@github.com:Acme/Widgets.git')
    await mkdir(join(root, 'docs', 'specs'), { recursive: true })

    expect(await repositories.locate(join(root, 'docs', 'specs'))).toEqual({
      mainRoot: root,
      prefix: 'docs/specs/',
      remote: { host: 'github.com', owner: 'Acme', repo: 'Widgets' },
      root,
    })
    expect((await repositories.locate(root))?.prefix).toBe('')
  })

  it('takes the first remote when there is no origin, and none when there is no remote', async () => {
    const root = await tempDir()
    await git(root, 'init', '-q')

    expect((await repositories.locate(root))?.remote).toBeUndefined()

    await git(root, 'remote', 'add', 'fork', 'ssh://git@gitlab.example.com:2222/team/sub/tool.git')
    expect((await repositories.locate(root))?.remote).toEqual({ host: 'gitlab.example.com', owner: 'team/sub', repo: 'tool' })
  })

  it('says a directory outside any repository is in none', async () => {
    const dir = await tempDir()

    expect(await repositories.locate(dir)).toBeUndefined()
  })

  it("names a linked worktree's main working tree, and a bare common directory as itself", async () => {
    const process = new ScriptedProcess([
      { exitCode: 0, stderr: '', stdout: '/work/linked\n/work/main/.git\n\n' },
      { exitCode: 1, stderr: '', stdout: '' },
      { exitCode: 0, stderr: '', stdout: '/work/linked\n/srv/repo.git\nsub/\n' },
      { exitCode: 0, stderr: '', stdout: 'remote.origin.url not-a-url\n' },
    ])
    const scripted = new GitBuildTrackerRepositories(process)

    expect(await scripted.locate('/work/linked')).toEqual({ mainRoot: '/work/main', prefix: '', remote: undefined, root: '/work/linked' })
    expect(await scripted.locate('/work/linked/sub')).toEqual({ mainRoot: '/srv/repo.git', prefix: 'sub/', remote: undefined, root: '/work/linked' })
    expect(process.requests[0]).toEqual({
      args: ['-C', '/work/linked', 'rev-parse', '--path-format=absolute', '--show-toplevel', '--git-common-dir', '--show-prefix'],
      command: 'git',
    })
  })

  it.each([
    ['git@github.com:acme/widgets.git', { host: 'github.com', owner: 'acme', repo: 'widgets' }],
    ['https://GitHub.com/acme/widgets/', { host: 'github.com', owner: 'acme', repo: 'widgets' }],
    ['https://user@github.com/acme/widgets', { host: 'github.com', owner: 'acme', repo: 'widgets' }],
    ['https://github.com/widgets', undefined],
    ['/local/path/widgets', undefined],
  ])('reads the remote %s', (url, expected) => {
    expect(parseRemote(url)).toEqual(expected)
  })
})

describe('NodeBuildTrackerProcess', () => {
  const process = new NodeBuildTrackerProcess()

  it('reports the output of a program that succeeds and of one that fails', async () => {
    const script = "process.stdout.write('out'); process.stderr.write('err'); process.exit(Number(process.argv[1]))"

    expect(await process.exec({ args: ['-e', script, '0'], command: globalThis.process.execPath })).toEqual({
      exitCode: 0,
      stderr: 'err',
      stdout: 'out',
    })
    expect(await process.exec({ args: ['-e', script, '3'], command: globalThis.process.execPath })).toEqual({
      exitCode: 3,
      stderr: 'err',
      stdout: 'out',
    })
  })

  it('rejects a program it cannot start', async () => {
    await expect(process.exec({ args: [], command: 'build-tracker-no-such-program' })).rejects.toThrow(
      /^cannot run build-tracker-no-such-program: /,
    )
  })
})

describe('NodeBuildTrackerFiles', () => {
  it('lists only the files in a directory', async () => {
    const dir = await tempDir()
    await writeFile(join(dir, 'a.md'), 'a')
    await mkdir(join(dir, 'sub'))

    expect(await new NodeBuildTrackerFiles().listFiles(dir)).toEqual(['a.md'])
    expect((await readdir(dir)).sort()).toEqual(['a.md', 'sub'])
  })
})
