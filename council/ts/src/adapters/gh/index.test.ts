import { describe, expect, it } from 'vitest'

import type { ProcessCommand, ProcessPort, ProcessResult } from '../../ports/index.js'
import { GhCliAdapter } from './index.js'

class CannedProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []

  constructor(private readonly result: ProcessResult) {}

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    return Promise.resolve(this.result)
  }
}

const ISSUE = {
  body: 'The spec.',
  comments: [
    { author: { login: 'joris' }, body: 'Amended.', createdAt: '2026-10-30T09:00:00Z' },
    { author: null, body: 'From a deleted account.' },
    { body: 42 },
  ],
  labels: [{ name: 'type:feature' }, { color: 'fff' }],
  number: 167,
  title: 'Widgets',
  url: 'https://github.com/o/r/issues/167',
}

function adapter(result: Partial<ProcessResult>): { gh: GhCliAdapter; process: CannedProcess } {
  const process = new CannedProcess({ exitCode: 0, stderr: '', stdout: '', ...result })
  return { gh: new GhCliAdapter(process), process }
}

const REQUEST = { comments: true, cwd: '/repo', number: 167, repo: 'github.com/o/r' }

describe('the gh issue reader', () => {
  it('reads an issue with its labels and comments from gh issue view', async () => {
    const { gh, process } = adapter({ stdout: JSON.stringify(ISSUE) })

    expect(await gh.viewIssue(REQUEST)).toEqual({
      body: 'The spec.',
      comments: [
        { author: 'joris', body: 'Amended.', createdAt: '2026-10-30T09:00:00Z' },
        { author: 'unknown', body: 'From a deleted account.', createdAt: '' },
      ],
      labels: ['type:feature'],
      number: 167,
      title: 'Widgets',
      url: 'https://github.com/o/r/issues/167',
    })
    expect(process.commands).toEqual([
      {
        args: ['issue', 'view', '167', '--repo', 'github.com/o/r', '--json', 'number,title,body,url,labels,comments'],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('asks gh for no comments when the request wants none', async () => {
    const { body, labels, number, title, url } = ISSUE
    const { gh, process } = adapter({ stdout: JSON.stringify({ body, labels, number, title, url }) })

    expect((await gh.viewIssue({ ...REQUEST, comments: false })).comments).toEqual([])
    expect(process.commands[0]?.args.at(-1)).toBe('number,title,body,url,labels')
  })

  it('rejects with the first line gh printed when it fails', async () => {
    const { gh } = adapter({ exitCode: 1, stderr: '\nGraphQL: Could not resolve to an issue\nmore detail\n' })

    await expect(gh.viewIssue(REQUEST)).rejects.toThrow(/^gh issue view exited 1: GraphQL: Could not resolve to an issue$/)
  })

  it('rejects with the exit code when a failing gh prints nothing', async () => {
    const { gh } = adapter({ exitCode: 4 })

    await expect(gh.viewIssue(REQUEST)).rejects.toThrow('gh issue view exited 4: exit code 4')
  })

  it.each(['not json', '[]', JSON.stringify({ ...ISSUE, title: 7 })])('rejects output that is not an issue: %s', async (stdout) => {
    const { gh } = adapter({ stdout })

    await expect(gh.viewIssue(REQUEST)).rejects.toThrow('gh issue view printed something that is not an issue')
  })
})

const LOCATOR = { cwd: '/repo', number: 181, repo: 'github.com/o/r' }

function apiIssue(number: number, overrides: Record<string, unknown> = {}): Record<string, unknown> {
  return {
    body: `Ticket ${String(number)}.`,
    html_url: `https://github.com/o/r/issues/${String(number)}`,
    labels: [{ name: 'type:feature' }],
    number,
    title: `Ticket ${String(number)}`,
    url: `https://api.github.com/repos/o/r/issues/${String(number)}`,
    ...overrides,
  }
}

describe('the gh sub-issue reader', () => {
  it('reads every page of an issue’s sub-issues through the REST API on the request’s host', async () => {
    const { gh, process } = adapter({ stdout: JSON.stringify([[apiIssue(183)], [apiIssue(184, { body: null, labels: [] })]]) })

    expect(await gh.subIssues(LOCATOR)).toEqual([
      { body: 'Ticket 183.', comments: [], labels: ['type:feature'], number: 183, title: 'Ticket 183', url: 'https://github.com/o/r/issues/183' },
      { body: '', comments: [], labels: [], number: 184, title: 'Ticket 184', url: 'https://github.com/o/r/issues/184' },
    ])
    expect(process.commands).toEqual([
      {
        args: ['api', '--hostname', 'github.com', '--paginate', '--slurp', 'repos/o/r/issues/181/sub_issues?per_page=100'],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('reads whether each sub-issue is open or closed', async () => {
    const { gh } = adapter({ stdout: JSON.stringify([[apiIssue(183, { state: 'open' }), apiIssue(184, { state: 'closed' })]]) })

    expect((await gh.subIssues(LOCATOR)).map((issue) => issue.state)).toEqual(['open', 'closed'])
  })

  it('rejects with the first line gh printed when it fails', async () => {
    const { gh } = adapter({ exitCode: 1, stderr: 'gh: Not Found (HTTP 404)\n' })

    await expect(gh.subIssues(LOCATOR)).rejects.toThrow(/^gh api exited 1: gh: Not Found \(HTTP 404\)$/)
  })

  it.each(['not json', '{}', '[{}]', JSON.stringify([[{ number: 'x' }]])])('rejects output that is not a list of issues: %s', async (stdout) => {
    const { gh } = adapter({ stdout })

    await expect(gh.subIssues(LOCATOR)).rejects.toThrow('gh api printed something that is not a list of issues')
  })
})

describe('the gh blocked-by reader', () => {
  it('reads the issues that block an issue, with their web URLs', async () => {
    const { gh, process } = adapter({ stdout: JSON.stringify([[apiIssue(175)]]) })

    expect(await gh.blockedBy({ ...LOCATOR, number: 184, repo: 'ghe.example.com/o/r' })).toEqual([
      { number: 175, url: 'https://github.com/o/r/issues/175' },
    ])
    expect(process.commands[0]?.args).toEqual([
      'api',
      '--hostname',
      'ghe.example.com',
      '--paginate',
      '--slurp',
      'repos/o/r/issues/184/dependencies/blocked_by?per_page=100',
    ])
  })

  it('rejects a repository that is not host/owner/repo', async () => {
    const { gh } = adapter({ stdout: '[]' })

    await expect(gh.blockedBy({ ...LOCATOR, repo: 'o/r' })).rejects.toThrow('o/r is not a host/owner/repo repository')
  })
})
