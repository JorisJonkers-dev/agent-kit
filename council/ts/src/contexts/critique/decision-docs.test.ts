import { mkdir, readFile } from 'node:fs/promises'
import { join } from 'node:path'

import { afterEach, describe, expect, it } from 'vitest'

import { DECISION_CANARY, harness, type Harness } from '../../../test/critique/harness.js'
import type { PrepareResult } from './index.js'

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
