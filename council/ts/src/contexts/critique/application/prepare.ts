import { basename, isAbsolute, join, relative, resolve } from 'node:path'

import { criticIdentity, type CriticIdentity } from '../domain/critic.js'
import { ORCHESTRATOR_FAMILIES, type OrchestratorFamily } from '../domain/family.js'
import { compactTimestamp, sha256, type PacketFile, type RunManifest } from '../domain/manifest.js'
import { CritiqueError, refusal, type Refusal } from '../domain/outcomes.js'
import { parseRouting, resolveRoster, type DroppedCritic } from '../domain/routing.js'
import { CRITIQUE_STAGES, type CritiqueStage, type Depth } from '../domain/stage.js'
import { readInput, readJsonFile } from './decode.js'
import { manifestPath } from './locations.js'
import type { CritiqueDeps } from './ports.js'
import { readAllowlist, resolveRepository, type ResolvedRepository } from './repository.js'

/** Decision docs tried, in order, when the caller names none. */
const DEFAULT_REFERENCES = ['GLOSSARY.md', 'CONTEXT.md']

interface PrepareInput {
  readonly stage: CritiqueStage
  readonly orchestratorFamily: OrchestratorFamily
  readonly repo: string
  readonly spec: string
  /** The reference paths the caller names; undefined means the default decision doc. */
  readonly references: readonly string[] | undefined
}

export interface PrepareResult {
  readonly status: 'ready'
  readonly run_id: string
  readonly run_dir: string
  readonly manifest_path: string
  readonly packet_dir: string
  readonly packet_files: readonly string[]
  readonly depth: Depth
  readonly critics: readonly CriticIdentity[]
  readonly dropped: readonly DroppedCritic[]
}

/**
 * Resolves the repository and its family allowlist, then the roster, and only
 * then freezes the artifact and its references into a packet. A refused
 * repository returns before anything is written. A roster that drops every
 * critic still prepares a run: fan-out and record then report it degraded,
 * so the flow goes on.
 */
export async function prepare(deps: CritiqueDeps, raw: unknown): Promise<PrepareResult | Refusal> {
  const input = readInput(raw, 'prepare', (fields): PrepareInput => ({
    orchestratorFamily: fields.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
    references: fields.has('references') ? fields.strings('references') : undefined,
    repo: fields.string('repo'),
    spec: fields.string('spec'),
    stage: fields.oneOf('stage', CRITIQUE_STAGES),
  }))
  const { orchestratorFamily, stage } = input
  const repository = await resolveRepository(deps, input.repo)
  if (repository.status === 'refused') return repository
  const entry = (await readAllowlist(deps)).repositories[repository.key]
  if (entry === undefined) {
    return refusal('not-opted-in', `${repository.key} has no family allowlist entry on this machine; record one with allow`)
  }
  const routing = await readJsonFile(deps.files, join(deps.assetsRoot, 'routing.json'), 'invalid-routing', parseRouting)
  const roster = resolveRoster(routing, {
    allowedFamilies: entry.families,
    orchestratorFamily,
    stage,
  })
  const runId = await freshRunId(deps, repository, stage)
  const runDir = join(repository.commonDir, 'critique', runId)
  const packetDir = join(runDir, 'packet')
  const packetFiles = await freezePacket(deps, repository, packetDir, input)
  const manifest: RunManifest = {
    allowed_families: entry.families,
    created_at: deps.clock.now().toISOString(),
    critics: roster.critics,
    depth: roster.depth,
    dropped: roster.dropped,
    orchestrator_family: orchestratorFamily,
    packet_dir: packetDir,
    packet_files: packetFiles,
    repo_root: repository.root,
    repository: repository.key,
    run_dir: runDir,
    run_id: runId,
    stage,
    version: 1,
  }
  await deps.files.writeText(manifestPath(runDir), `${JSON.stringify(manifest, null, 2)}\n`)
  return {
    critics: roster.critics.map(criticIdentity),
    depth: roster.depth,
    dropped: roster.dropped,
    manifest_path: manifestPath(runDir),
    packet_dir: packetDir,
    packet_files: packetFiles.map((file) => file.path),
    run_dir: runDir,
    run_id: runId,
    status: 'ready',
  }
}

async function freshRunId(deps: CritiqueDeps, repository: ResolvedRepository, stage: CritiqueStage): Promise<string> {
  const base = `${compactTimestamp(deps.clock.now())}-${stage}`
  let candidate = base
  for (let n = 2; await deps.files.exists(manifestPath(join(repository.commonDir, 'critique', candidate))); n += 1) {
    candidate = `${base}-${String(n)}`
  }
  return candidate
}

/**
 * Copies the artifact to `artifact/` and each reference to `references/`
 * under its repository path. Nothing else is ever put at the packet root, so
 * a `.vibe/`, `.agents/` or `AGENTS.md` there can never become critic config.
 */
async function freezePacket(
  deps: CritiqueDeps,
  repository: ResolvedRepository,
  packetDir: string,
  input: PrepareInput,
): Promise<readonly PacketFile[]> {
  const spec = insideRepository(repository.root, input.spec)
  const references =
    input.references === undefined
      ? await defaultReferences(deps, repository.root)
      : input.references.map((path) => insideRepository(repository.root, path))
  const sources = [
    { kind: 'artifact' as const, path: join('artifact', basename(spec)), source: spec },
    ...references.map((source) => ({ kind: 'reference' as const, path: join('references', source), source })),
  ]
  const frozen: PacketFile[] = []
  for (const file of sources) {
    const text = await deps.files.readText(join(repository.root, file.source))
    await deps.files.writeText(join(packetDir, file.path), text)
    frozen.push({ ...file, sha256: sha256(text) })
  }
  return frozen
}

async function defaultReferences(deps: CritiqueDeps, root: string): Promise<readonly string[]> {
  for (const candidate of DEFAULT_REFERENCES) {
    if (await deps.files.exists(join(root, candidate))) return [candidate]
  }
  return []
}

function insideRepository(root: string, path: string): string {
  const rel = relative(root, isAbsolute(path) ? path : resolve(root, path))
  if (rel.startsWith('..') || isAbsolute(rel)) throw new CritiqueError('outside-repository', `${path} is outside the repository`)
  return rel
}
