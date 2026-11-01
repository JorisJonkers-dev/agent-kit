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
