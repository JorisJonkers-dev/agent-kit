import { mkdir, readdir, readFile } from 'node:fs/promises'
import { join, resolve } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import {
  ASSETS_ROOT,
  DECISION_CANARY,
  fenced,
  finding,
  harness,
  isolatedExport,
  SPEC_CANARY,
  vibeStdout,
  type Harness,
} from '../../../test/critique/harness.js'
import type { FanOutResult, PrepareResult } from './index.js'

const open: Harness[] = []

afterEach(async () => {
  await Promise.all(open.splice(0).map((h) => h.cleanup()))
})

/** The domain doc Pocock's setup writes, cut to the part that names paths. */
const POCOCK_DOMAIN_DOC = `# Domain Docs

## Before exploring, read these

- **\`CONTEXT.md\`** at the repo root, or
- **\`CONTEXT-MAP.md\`** at the repo root if it exists: it points at one \`CONTEXT.md\` per context.
- **\`docs/adr/\`**: read ADRs that touch the area. In multi-context repos, also check \`src/<context>/docs/adr/\`.

The \`/domain-modeling\` skill creates them lazily.

\`\`\`
/
├── CONTEXT.md
├── docs/adr/
│   └── \`0002-postgres-for-write-model.md\`
\`\`\`
`

async function optedIn(): Promise<Harness> {
  const h = await harness()
  open.push(h)
  await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
  return h
}

async function packetFiles(h: Harness): Promise<readonly string[]> {
  const result = (await h.commands.prepare.run({
    orchestrator_family: 'anthropic',
    repo: h.repo,
    spec: '.scratch/widgets/spec.md',
    stage: 'spec',
  })) as PrepareResult
  return result.packet_files
}

async function write(h: Harness, path: string, content = `# ${path}\n`): Promise<void> {
  await h.writeText(join(h.repo, path), content)
}

describe('prepare without a domain doc', () => {
  it('freezes the glossary along with every ADR', async () => {
    const h = await optedIn()
    await write(h, 'docs/adr/0002-postgres.md')
    await write(h, 'docs/adr/0001-event-sourcing.md')
    await write(h, 'docs/adr/diagram.png')

    expect(await packetFiles(h)).toEqual([
      'artifact/spec.md',
      'references/CONTEXT.md',
      'references/docs/adr/0001-event-sourcing.md',
      'references/docs/adr/0002-postgres.md',
    ])
  })
})

describe('prepare with a domain doc', () => {
  it('freezes the files and directories the domain doc names outside its code blocks', async () => {
    const h = await optedIn()
    await write(h, 'docs/agents/domain.md', POCOCK_DOMAIN_DOC)
    await write(h, 'docs/adr/0001-event-sourcing.md')
    await write(h, '0002-postgres-for-write-model.md')

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md', 'references/docs/adr/0001-event-sourcing.md'])
  })

  it('freezes the decision doc itself, as the critics read it', async () => {
    const h = await optedIn()
    await write(h, 'docs/agents/domain.md', POCOCK_DOMAIN_DOC)

    const result = (await h.commands.prepare.run({
      orchestrator_family: 'anthropic',
      repo: h.repo,
      spec: '.scratch/widgets/spec.md',
      stage: 'spec',
    })) as PrepareResult

    expect(await readFile(join(result.packet_dir, 'references', 'CONTEXT.md'), 'utf8')).toContain(DECISION_CANARY)
  })

  it('uses the domain doc instead of the glossary fallback', async () => {
    const h = await optedIn()
    await write(h, 'GLOSSARY.md')
    await write(h, 'docs/agents/domain.md', 'Read `CONTEXT.md` before exploring.\n')

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md'])
  })

  it('follows a context map to each context and its ADRs', async () => {
    const h = await optedIn()
    await h.removeFile(join(h.repo, 'CONTEXT.md'))
    await write(h, 'docs/agents/domain.md', POCOCK_DOMAIN_DOC)
    await write(h, 'CONTEXT-MAP.md', '# Contexts\n\n- [Ordering](./src/ordering/CONTEXT.md)\n- `src/billing/CONTEXT.md`\n- `src/billing/README.md`\n')
    await write(h, 'src/ordering/CONTEXT.md')
    await write(h, 'src/ordering/docs/adr/0001-orders.md')
    await write(h, 'src/billing/CONTEXT.md')
    await write(h, 'docs/adr/0001-system.md')

    expect(await packetFiles(h)).toEqual([
      'artifact/spec.md',
      'references/CONTEXT-MAP.md',
      'references/src/ordering/CONTEXT.md',
      'references/src/ordering/docs/adr/0001-orders.md',
      'references/src/billing/CONTEXT.md',
      'references/docs/adr/0001-system.md',
    ])
  })

  it('skips a named directory that holds no docs', async () => {
    const h = await optedIn()
    await write(h, 'docs/agents/domain.md', POCOCK_DOMAIN_DOC)
    await mkdir(join(h.repo, 'docs', 'adr'), { recursive: true })

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md'])
  })

  it('never names a path outside the repository', async () => {
    const h = await optedIn()
    await h.writeText(join(h.repo, '..', 'secrets.md'), '# Not this repository\n')
    await write(h, 'docs/agents/domain.md', 'Read `../secrets.md`, `/etc/hosts` and `CONTEXT.md`.\n')

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md'])
  })
})

describe('prepare reads each ADR status', () => {
  it('leaves out ADRs explicitly proposed, rejected, superseded or deprecated, and keeps accepted and status-less ones', async () => {
    const h = await optedIn()
    await write(h, 'docs/adr/0001-no-status.md', '# Widgets run nightly\n\nWe frobnicate widgets in the nightly job.\n')
    await write(h, 'docs/adr/0002-accepted.md', '---\nstatus: accepted\n---\n\n# Accepted\n')
    await write(h, 'docs/adr/0003-proposed.md', '---\nstatus: proposed\n---\n\n# Proposed\n')
    await write(h, 'docs/adr/0004-rejected.md', '# Rejected\n\nStatus: Rejected\n')
    await write(h, 'docs/adr/0005-superseded.md', '---\ntitle: Old\nstatus: superseded by ADR-0007\n---\n\n# Superseded\n')
    await write(h, 'docs/adr/0006-deprecated.md', '# Deprecated\n\n**Status:** deprecated\n')
    await write(h, 'docs/adr/0007-heading.md', '# Heading\n\n## Status\n\nSuperseded by [0008](0008-accepted-heading.md)\n')
    await write(h, 'docs/adr/0008-accepted-heading.md', '# Accepted heading\n\n## Status\n\nAccepted\n')
    await write(h, 'docs/adr/0009-frontmatter-no-status.md', '---\ntitle: No status\n---\n\n# No status\n')
    await write(h, 'docs/adr/0010-fenced.md', '# Fenced\n\nA record template:\n\n```\nStatus: rejected\n```\n')

    expect(await packetFiles(h)).toEqual([
      'artifact/spec.md',
      'references/CONTEXT.md',
      'references/docs/adr/0001-no-status.md',
      'references/docs/adr/0002-accepted.md',
      'references/docs/adr/0008-accepted-heading.md',
      'references/docs/adr/0009-frontmatter-no-status.md',
      'references/docs/adr/0010-fenced.md',
    ])
  })

  it('keeps a status-less ADR that only mentions a status word in its prose', async () => {
    const h = await optedIn()
    await write(h, 'docs/adr/0001-prose.md', '# Prose\n\nThe old approach was rejected, and the proposed one superseded it.\n')

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md', 'references/docs/adr/0001-prose.md'])
  })

  it('applies to the ADRs a domain doc names', async () => {
    const h = await optedIn()
    await write(h, 'docs/agents/domain.md', POCOCK_DOMAIN_DOC)
    await write(h, 'docs/adr/0001-kept.md')
    await write(h, 'docs/adr/0002-proposed.md', 'Status: proposed\n\n# Proposed\n')

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md', 'references/docs/adr/0001-kept.md'])
  })

  it('never reads a status into a glossary that defines the word', async () => {
    const h = await optedIn()
    await write(h, 'CONTEXT.md', `# Glossary\n\n**Status:** deprecated terms are listed last.\n\n${DECISION_CANARY}\n`)

    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md'])
  })

  it("still loads agent-kit's own ADRs, which carry no status line", async () => {
    const h = await optedIn()
    const houseAdrs = resolve(ASSETS_ROOT, '../../../docs/adr')
    const names = (await readdir(houseAdrs)).filter((name) => name.endsWith('.md')).sort()
    for (const name of names) await write(h, `docs/adr/${name}`, await readFile(join(houseAdrs, name), 'utf8'))

    expect(names.length).toBeGreaterThan(0)
    expect(await packetFiles(h)).toEqual(['artifact/spec.md', 'references/CONTEXT.md', ...names.map((name) => `references/docs/adr/${name}`)])
  })
})

describe('a quote from a record not in force', () => {
  const RETIRED = 'Decided: widgets are frobnicated on every write.'

  async function quoted(status: string): Promise<FanOutResult['findings'][number] | undefined> {
    const omission = finding({
      category: 'omission',
      evidence: [
        { file: 'artifact/spec.md', kind: 'quote', value: SPEC_CANARY },
        { file: 'references/docs/adr/0002-every-write.md', kind: 'quote', value: RETIRED },
      ],
    })
    const h = await harness({ scripts: [{ exitCode: 0, exportJson: isolatedExport(), stdout: vibeStdout(fenced({ findings: [omission] })) }] })
    open.push(h)
    await h.commands.allow.run({ repo: h.repo, families: ['mistral'] })
    await write(h, 'docs/adr/0002-every-write.md', `---\nstatus: ${status}\n---\n\n# Every write\n\n${RETIRED}\n`)
    const run = await h.commands.prepare.run({ orchestrator_family: 'anthropic', repo: h.repo, spec: '.scratch/widgets/spec.md', stage: 'spec' })
    if (run.status !== 'ready') throw new Error('not ready')
    return (await h.commands.fanOut.run({ run_dir: run.run_dir })).findings[0]
  }

  it('counts as a decision quote while the record is accepted', async () => {
    expect((await quoted('accepted'))?.pre_verdict).toBeUndefined()
  })

  it('cannot set the decision flag once the record is superseded, because the record is not in the packet', async () => {
    expect((await quoted('superseded by ADR-0003'))?.pre_verdict).toMatchObject({ reason_code: 'fabricated-evidence', verdict: 'rejected' })
  })
})
