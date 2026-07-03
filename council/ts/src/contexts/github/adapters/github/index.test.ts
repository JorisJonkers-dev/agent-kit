import { describe, expect, it, vi } from 'vitest'

import type { Task } from '../../../../shared-kernel/index.js'
import type { ProcessCommand, ProcessPort, ProcessResult } from '../../../../ports/index.js'
import {
  GithubCliAdapter,
  type GithubLog,
  type GithubTaskIssueMirrorRequest,
} from './index.js'

type Handler = (command: ProcessCommand) => ProcessResult | Promise<ProcessResult>

class RecordingProcess implements ProcessPort {
  readonly commands: ProcessCommand[] = []
  private readonly handler: Handler

  constructor(handler: Handler) {
    this.handler = handler
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    this.commands.push(command)
    return this.handler(command)
  }
}

function success(stdout = ''): ProcessResult {
  return {
    exitCode: 0,
    stderr: '',
    stdout,
  }
}

function failure(exitCode = 1, stderr = 'gh failed', stdout = ''): ProcessResult {
  return {
    exitCode,
    stderr,
    stdout,
  }
}

function task(overrides: Partial<Task> = {}): Task {
  return {
    acceptance_criteria: ['mirrors to GitHub'],
    archetype: 'adapter',
    boundaries: 'GitHub only',
    context_profile: 'thin',
    depends_on: ['T1'],
    difficulty: 'moderate',
    id: 'T25',
    model: 'sonnet',
    objective: 'Mirror state',
    output_format: 'TypeScript',
    paths: ['council/ts/src/adapters/github/index.ts'],
    title: 'GitHub adapter',
    verify: 'npm test',
    ...overrides,
  }
}

function mirrorRequest(
  overrides: Partial<GithubTaskIssueMirrorRequest> = {},
): GithubTaskIssueMirrorRequest {
  return {
    runId: 'run-1',
    status: 'In Progress',
    task: task(),
    ...overrides,
  }
}

function logger(): { log: GithubLog; messages: string[]; errors: unknown[] } {
  const messages: string[] = []
  const errors: unknown[] = []
  return {
    errors,
    log(message, error) {
      messages.push(message)
      errors.push(error)
    },
    messages,
  }
}

describe('GithubCliAdapter', () => {
  it('detects the repository default branch', async () => {
    const process = new RecordingProcess(() => success('main\n'))
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.detectDefaultBranch('/repo')).resolves.toBe('main')

    expect(process.commands).toEqual([
      {
        args: [
          'repo',
          'view',
          '--json',
          'defaultBranchRef',
          '--jq',
          '.defaultBranchRef.name',
        ],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('returns undefined and logs when default branch detection fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.detectDefaultBranch('/repo')).resolves.toBeUndefined()

    expect(logs.messages).toEqual(['github detectDefaultBranch failed'])
  })

  it('lists only verified labels returned by gh label list', async () => {
    const process = new RecordingProcess(() =>
      success(JSON.stringify([{ name: 'manual' }, { bad: 'shape' }, { name: 'model:sonnet' }])),
    )
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.listVerifiedLabels('/repo')).resolves.toEqual([
      { name: 'manual' },
      { name: 'model:sonnet' },
    ])

    expect(process.commands).toEqual([
      {
        args: ['label', 'list', '--json', 'name', '--limit', '1000'],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('returns an empty label list and logs when gh label list fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.listVerifiedLabels('/repo')).resolves.toEqual([])

    expect(logs.messages).toEqual(['github listVerifiedLabels failed'])
  })

  it('creates labels only through the explicit bootstrap path', async () => {
    const process = new RecordingProcess(() => success())
    const adapter = new GithubCliAdapter(process, { githubBootstrap: true })

    await expect(
      adapter.bootstrapLabels('/repo', ['council/status:queued', 'manual']),
    ).resolves.toBeUndefined()

    expect(process.commands).toEqual([
      {
        args: ['label', 'create', 'council/status:queued', '--force'],
        command: 'gh',
        cwd: '/repo',
      },
      {
        args: ['label', 'create', 'manual', '--force'],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('skips bootstrap label creation unless bootstrap is enabled and not dry-run', async () => {
    const process = new RecordingProcess(() => success())
    const adapter = new GithubCliAdapter(process)
    const dryRunAdapter = new GithubCliAdapter(process, { dryRun: true, githubBootstrap: true })

    await expect(adapter.bootstrapLabels('/repo', ['manual'])).resolves.toBeUndefined()
    await expect(dryRunAdapter.bootstrapLabels('/repo', ['manual'])).resolves.toBeUndefined()

    expect(process.commands).toEqual([])
  })

  it('logs and continues when bootstrap label creation fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      githubBootstrap: true,
      log: logs.log,
    })

    await expect(adapter.bootstrapLabels('/repo', ['manual'])).resolves.toBeUndefined()

    expect(logs.messages).toEqual(['github bootstrapLabels failed'])
  })

  it('lists only milestone records with the expected shape', async () => {
    const process = new RecordingProcess(() =>
      success(JSON.stringify([{ number: 3, title: 'Run 1' }, { number: 'bad', title: 'bad' }])),
    )
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.listMilestones('/repo')).resolves.toEqual([
      { number: 3, title: 'Run 1' },
    ])

    expect(process.commands).toEqual([
      {
        args: ['api', 'repos/{owner}/{repo}/milestones', '--paginate', '-f', 'state=all'],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('returns an empty milestone list and logs when milestone listing fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.listMilestones('/repo')).resolves.toEqual([])

    expect(logs.messages).toEqual(['github listMilestones failed'])
  })

  it('reuses existing milestones and creates missing milestones unless dry-run is active', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args.includes('state=all')) {
        return success(JSON.stringify([{ number: 7, title: 'Run 1' }]))
      }
      return success(JSON.stringify({ number: 8, title: 'Run 2' }))
    })
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.ensureMilestone('/repo', 'Run 1')).resolves.toEqual({
      number: 7,
      title: 'Run 1',
    })
    await expect(adapter.ensureMilestone('/repo', 'Run 2')).resolves.toEqual({
      number: 8,
      title: 'Run 2',
    })
    await expect(
      adapter.ensureMilestone('/repo', 'Run 3', { dryRun: true }),
    ).resolves.toBeUndefined()

    expect(process.commands.map((command) => command.args)).toEqual([
      ['api', 'repos/{owner}/{repo}/milestones', '--paginate', '-f', 'state=all'],
      ['api', 'repos/{owner}/{repo}/milestones', '--paginate', '-f', 'state=all'],
      ['api', 'repos/{owner}/{repo}/milestones', '-f', 'title=Run 2'],
      ['api', 'repos/{owner}/{repo}/milestones', '--paginate', '-f', 'state=all'],
    ])
  })

  it('returns undefined and logs when milestone creation fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.ensureMilestone('/repo', 'Run 2')).resolves.toBeUndefined()

    expect(logs.messages).toEqual([
      'github listMilestones failed',
      'github ensureMilestone failed',
    ])
  })

  it('lists only issue records with the expected shape', async () => {
    const process = new RecordingProcess(() =>
      success(
        JSON.stringify([
          { body: 'body', labels: [{ name: 'manual' }], number: 12, state: 'OPEN', title: 'Task' },
          { number: 'bad', title: 'bad' },
        ]),
      ),
    )
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.listIssues('/repo')).resolves.toEqual([
      { body: 'body', labels: [{ name: 'manual' }], number: 12, state: 'OPEN', title: 'Task' },
    ])

    expect(process.commands).toEqual([
      {
        args: [
          'issue',
          'list',
          '--state',
          'all',
          '--json',
          'number,title,body,labels,state',
          '--limit',
          '1000',
        ],
        command: 'gh',
        cwd: '/repo',
      },
    ])
  })

  it('returns an empty issue list and logs when issue listing fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.listIssues('/repo')).resolves.toEqual([])

    expect(logs.messages).toEqual(['github listIssues failed'])
  })

  it('creates a task issue from verified labels without creating labels', async () => {
    const process = new RecordingProcess((command) => {
      const args = command.args.join(' ')
      if (args.startsWith('issue list')) {
        return success('[]')
      }
      if (args.startsWith('label list')) {
        return success(
          JSON.stringify([
            { name: 'manual' },
            { name: 'council/status:in-progress' },
            { name: 'difficulty:moderate' },
            { name: 'model:sonnet' },
            { name: 'council/archetype:adapter' },
            { name: 'area:council' },
          ]),
        )
      }
      if (args.includes('state=all')) {
        return success(JSON.stringify([{ number: 4, title: 'Run 1' }]))
      }
      if (args.startsWith('issue create')) {
        return success('https://github.com/owner/repo/issues/22\n')
      }
      return failure()
    })
    const adapter = new GithubCliAdapter(process)

    await expect(
      adapter.mirrorTaskIssue(
        '/repo',
        mirrorRequest({
          milestoneTitle: 'Run 1',
          preferredLabels: ['manual', 'missing'],
          specRef: '001-github-mirror',
        }),
      ),
    ).resolves.toEqual({
      duplicates: [],
      marker: '<!-- council-task-id: run-1/T25 -->',
      number: 22,
    })

    expect(process.commands.map((command) => command.args[0])).toEqual([
      'issue',
      'label',
      'api',
      'issue',
    ])
    expect(process.commands.at(-1)?.args).toEqual([
      'issue',
      'create',
      '--title',
      'GitHub adapter',
      '--body',
      expect.stringContaining('spec_ref: 001-github-mirror'),
      '--label',
      [
        'manual',
        'council/status:in-progress',
        'difficulty:moderate',
        'model:sonnet',
        'council/archetype:adapter',
        'area:council',
      ].join(','),
      '--milestone',
      'Run 1',
    ])
  })

  it('updates matched task issues, transitions status labels, and keeps duplicates visible', async () => {
    const marker = '<!-- council-task-id: run-1/T25 -->'
    const process = new RecordingProcess((command) => {
      const args = command.args.join(' ')
      if (args.startsWith('issue list')) {
        return success(
          JSON.stringify([
            {
              body: marker,
              labels: ['council/status:queued', { name: 'manual' }],
              number: 10,
              state: 'OPEN',
              title: 'Old',
            },
            { body: `duplicate ${marker}`, labels: [], number: 11, title: 'Duplicate' },
          ]),
        )
      }
      if (args.startsWith('label list')) {
        return success(
          JSON.stringify([
            { name: 'manual' },
            { name: 'council/status:landed' },
            { name: 'difficulty:moderate' },
            { name: 'model:sonnet' },
            { name: 'council/archetype:adapter' },
            { name: 'area:council' },
          ]),
        )
      }
      if (args.includes('state=all')) {
        return success(JSON.stringify([{ number: 4, title: 'Run 1' }]))
      }
      if (args.startsWith('issue edit')) {
        return success()
      }
      return failure()
    })
    const adapter = new GithubCliAdapter(process)

    await expect(
      adapter.mirrorTaskIssue(
        '/repo',
        mirrorRequest({
          landed: true,
          milestoneTitle: 'Run 1',
          preferredLabels: ['manual'],
          status: 'Landed',
        }),
      ),
    ).resolves.toEqual({
      duplicates: [{ body: `duplicate ${marker}`, labels: [], number: 11, title: 'Duplicate' }],
      marker,
      number: 10,
    })

    expect(process.commands.at(-1)?.args).toEqual([
      'issue',
      'edit',
      '10',
      '--title',
      'GitHub adapter',
      '--body',
      expect.stringContaining('# T25: GitHub adapter'),
      '--add-label',
      [
        'council/status:landed',
        'difficulty:moderate',
        'model:sonnet',
        'council/archetype:adapter',
        'area:council',
      ].join(','),
      '--remove-label',
      'council/status:queued',
      '--state',
      'closed',
      '--milestone',
      'Run 1',
    ])
  })

  it('does not mutate task issues during dry-run', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'issue') {
        return success('[]')
      }
      if (command.args[0] === 'label') {
        return success('[]')
      }
      return failure()
    })
    const adapter = new GithubCliAdapter(process, { dryRun: true })

    await expect(adapter.mirrorTaskIssue('/repo', mirrorRequest())).resolves.toEqual({
      duplicates: [],
      marker: '<!-- council-task-id: run-1/T25 -->',
    })

    expect(process.commands.map((command) => command.args[1])).toEqual(['list', 'list'])
  })

  it('returns an empty mirror result and logs when task issue mirroring fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.mirrorTaskIssue('/repo', mirrorRequest())).resolves.toEqual({
      duplicates: [],
      marker: '',
    })

    expect(logs.messages).toEqual([
      'github listIssues failed',
      'github listVerifiedLabels failed',
      'github mirrorTaskIssue failed',
    ])
  })

  it('creates pull requests with rendered bodies and detected default branches', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[0] === 'repo') {
        return success('main\n')
      }
      return success(JSON.stringify({ number: 5, url: 'https://github.com/owner/repo/pull/5' }))
    })
    const adapter = new GithubCliAdapter(process)

    await expect(
      adapter.createPullRequest({
        closingIssueNumbers: [22],
        cwd: '/repo',
        draft: true,
        extraSections: ['Validation: npm test'],
        head: 'feature',
        referenceIssueNumbers: [23],
        summary: 'Mirror task state.',
        title: 'Mirror tasks',
      }),
    ).resolves.toEqual({ number: 5, url: 'https://github.com/owner/repo/pull/5' })

    expect(process.commands.map((command) => command.args)).toEqual([
      ['repo', 'view', '--json', 'defaultBranchRef', '--jq', '.defaultBranchRef.name'],
      [
        'pr',
        'create',
        '--title',
        'Mirror tasks',
        '--body',
        'Mirror task state.\n\nCloses #22\nrefs #23\n\nValidation: npm test\n',
        '--json',
        'number,url',
        '--base',
        'main',
        '--head',
        'feature',
        '--draft',
      ],
    ])
  })

  it('does not create pull requests during dry-run', async () => {
    const process = new RecordingProcess(() => failure())
    const adapter = new GithubCliAdapter(process, { dryRun: true })

    await expect(
      adapter.createPullRequest({
        cwd: '/repo',
        summary: 'Mirror task state.',
        title: 'Mirror tasks',
      }),
    ).resolves.toBeUndefined()

    expect(process.commands).toEqual([])
  })

  it('returns undefined and logs when pull request creation fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(
      adapter.createPullRequest({
        base: 'main',
        cwd: '/repo',
        summary: 'Mirror task state.',
        title: 'Mirror tasks',
      }),
    ).resolves.toBeUndefined()

    expect(logs.messages).toEqual(['github createPullRequest failed'])
  })

  it('views pull requests and ignores unexpected gh JSON shapes', async () => {
    const process = new RecordingProcess((command) => {
      if (command.args[2] === '5') {
        return success(JSON.stringify({ number: 5, url: 'https://github.com/owner/repo/pull/5' }))
      }
      return success(JSON.stringify(null))
    })
    const adapter = new GithubCliAdapter(process)

    await expect(adapter.viewPullRequest('/repo', 5)).resolves.toEqual({
      number: 5,
      url: 'https://github.com/owner/repo/pull/5',
    })
    await expect(adapter.viewPullRequest('/repo', 6)).resolves.toBeUndefined()

    expect(process.commands.map((command) => command.args)).toEqual([
      ['pr', 'view', '5', '--json', 'number,url'],
      ['pr', 'view', '6', '--json', 'number,url'],
    ])
  })

  it('returns undefined and logs when pull request viewing fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(adapter.viewPullRequest('/repo', 5)).resolves.toBeUndefined()

    expect(logs.messages).toEqual(['github viewPullRequest failed'])
  })

  it('adds issue and pull request comments unless dry-run is active', async () => {
    const process = new RecordingProcess(() => success())
    const adapter = new GithubCliAdapter(process)

    await expect(
      adapter.addComment('/repo', { body: 'Issue note', kind: 'issue', number: 12 }),
    ).resolves.toBeUndefined()
    await expect(
      adapter.addComment('/repo', { body: 'PR note', kind: 'pr', number: 5 }),
    ).resolves.toBeUndefined()
    await expect(
      adapter.addComment('/repo', { body: 'Skipped', kind: 'issue', number: 13 }, { dryRun: true }),
    ).resolves.toBeUndefined()

    expect(process.commands.map((command) => command.args)).toEqual([
      ['issue', 'comment', '12', '--body', 'Issue note'],
      ['pr', 'comment', '5', '--body', 'PR note'],
    ])
  })

  it('logs and continues when adding a comment fails', async () => {
    const logs = logger()
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()), {
      log: logs.log,
    })

    await expect(
      adapter.addComment('/repo', { body: 'Issue note', kind: 'issue', number: 12 }),
    ).resolves.toBeUndefined()

    expect(logs.messages).toEqual(['github addComment failed'])
  })

  it('uses the default logger when none is provided', async () => {
    const warn = vi.spyOn(console, 'warn').mockImplementation(() => undefined)
    const adapter = new GithubCliAdapter(new RecordingProcess(() => failure()))

    await expect(adapter.detectDefaultBranch('/repo')).resolves.toBeUndefined()

    expect(warn).toHaveBeenCalledWith('github detectDefaultBranch failed')
    warn.mockRestore()
  })
})
