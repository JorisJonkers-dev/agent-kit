import { join } from 'node:path'

import { criticIdentity, type CriticIdentity } from '../domain/critic.js'
import { chooseDepth, type DepthChoice } from '../domain/depth.js'
import { ORCHESTRATOR_FAMILIES, type OrchestratorFamily } from '../domain/family.js'
import { compactTimestamp, sha256, type ArtifactSource, type PacketFile, type RunManifest } from '../domain/manifest.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import {
  candidateClients,
  resolveRoster,
  stageRouting,
  type ClientReadiness,
  type DroppedCritic,
  type Routing,
  type RosterRequest,
} from '../domain/routing.js'
import { CRITIQUE_STAGES, DEPTHS, type CritiqueStage, type Depth } from '../domain/stage.js'
import { readInput } from './decode.js'
import { DecisionDocs } from './decision-docs.js'
import { manifestPath } from './locations.js'
import { insideRepository } from './paths.js'
import { clientReadiness, type CritiqueCommand, type CritiqueDeps } from './ports.js'
import { RepositoryGate, type ResolvedRepository } from './repository.js'
import { RoutingSource } from './routing.js'
import { TrackerResolver, type TrackedArtifact } from './tracker.js'

interface PrepareInput {
  readonly stage: CritiqueStage
  readonly orchestratorFamily: OrchestratorFamily
  readonly repo: string
  /** The artifact as its tracker names it: an issue reference for GitHub, a path for local markdown. */
  readonly spec: string
  /** Whether a GitHub issue's comments are frozen with its body. */
  readonly comments: boolean
  /** The reference paths the caller names; undefined means the repository's decision docs. */
  readonly references: readonly string[] | undefined
  /** The depth the caller names, which always wins over the signals; undefined lets the signals choose. */
  readonly depth: Depth | undefined
}

export interface PrepareResult {
  readonly status: 'ready'
  readonly run_id: string
  readonly run_dir: string
  readonly manifest_path: string
  readonly packet_dir: string
  readonly packet_files: readonly string[]
  readonly artifact: ArtifactSource
  readonly depth: Depth
  readonly depth_choice: DepthChoice
  readonly critics: readonly CriticIdentity[]
  readonly dropped: readonly DroppedCritic[]
}

/**
 * Resolves the repository and its family allowlist, reads the artifact from
 * the repository's tracker, chooses the depth from the artifact (or takes
 * the caller's), then resolves the roster, and only then freezes
 * the artifact and its decision docs into a packet. A refused repository, an
 * unsupported tracker and an issue gh cannot read all return before anything
 * is written. A roster that drops every
 * critic still prepares a run: fan-out and record then report it degraded,
 * so the flow goes on.
 */
export class PrepareCommand implements CritiqueCommand<PrepareResult | Refusal> {
  private readonly repositories: RepositoryGate
  private readonly trackers: TrackerResolver
  private readonly decisionDocs: DecisionDocs
  private readonly routing: RoutingSource

  constructor(private readonly deps: CritiqueDeps) {
    this.repositories = new RepositoryGate(deps)
    this.trackers = new TrackerResolver(deps)
    this.decisionDocs = new DecisionDocs(deps.files)
    this.routing = new RoutingSource(deps)
  }

  async run(raw: unknown): Promise<PrepareResult | Refusal> {
    const input = readInput(raw, 'prepare', (fields): PrepareInput => ({
      comments: fields.has('comments') ? fields.boolean('comments') : false,
      depth: fields.has('depth') ? fields.oneOf('depth', DEPTHS) : undefined,
      orchestratorFamily: fields.oneOf('orchestrator_family', ORCHESTRATOR_FAMILIES),
      references: fields.has('references') ? fields.strings('references') : undefined,
      repo: fields.string('repo'),
      spec: fields.string('spec'),
      stage: fields.oneOf('stage', CRITIQUE_STAGES),
    }))
    const { orchestratorFamily, stage } = input
    const repository = await this.repositories.resolve(input.repo)
    if (repository.status === 'refused') return repository
    const entry = (await this.repositories.allowlist()).repositories[repository.key]
    if (entry === undefined) {
      return refusal('not-opted-in', `${repository.key} has no family allowlist entry on this machine; record one with allow`)
    }
    const tracker = await this.trackers.resolve(repository)
    if (tracker.status === 'refused') return tracker
    const artifact = await tracker.artifact({ comments: input.comments, reference: input.spec })
    if (artifact.status === 'refused') return artifact
    const references =
      input.references === undefined
        ? await this.decisionDocs.resolve(repository.root)
        : input.references.map((path) => insideRepository(repository.root, path))
    const routing = await this.routing.load()
    const { choice, depth } = chooseDepth(
      stage,
      stageRouting(routing, { stage }).depth_signals,
      { labels: artifact.labels, text: artifact.text },
      input.depth,
    )
    const request = { allowedFamilies: entry.families, depth, orchestratorFamily, stage }
    const roster = resolveRoster(routing, request, await this.readiness(routing, request))
    const runId = await this.freshRunId(repository, stage)
    const runDir = join(repository.commonDir, 'critique', runId)
    const packetDir = join(runDir, 'packet')
    const packetFiles = await this.freezePacket(repository, packetDir, artifact, references)
    const source: ArtifactSource = { labels: artifact.labels, reference: artifact.reference, tracker: artifact.tracker }
    const manifest: RunManifest = {
      allowed_families: entry.families,
      artifact: source,
      created_at: this.deps.clock.now().toISOString(),
      critics: roster.critics,
      depth: roster.depth,
      depth_choice: choice,
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
    await this.deps.files.writeText(manifestPath(runDir), `${JSON.stringify(manifest, null, 2)}\n`)
    return {
      artifact: source,
      critics: roster.critics.map(criticIdentity),
      depth: roster.depth,
      depth_choice: choice,
      dropped: roster.dropped,
      manifest_path: manifestPath(runDir),
      packet_dir: packetDir,
      packet_files: packetFiles.map((file) => file.path),
      run_dir: runDir,
      run_id: runId,
      status: 'ready',
    }
  }

  /** Asks each client the stage could run, and that the family rules permit, whether it can run here. */
  private async readiness(routing: Routing, request: RosterRequest): Promise<ReadonlyMap<string, ClientReadiness>> {
    const checks = candidateClients(routing, request).map(
      async ([name, client]) => [name, await clientReadiness(this.deps.clients, client)] as const,
    )
    return new Map(await Promise.all(checks))
  }

  private async freshRunId(repository: ResolvedRepository, stage: CritiqueStage): Promise<string> {
    const base = `${compactTimestamp(this.deps.clock.now())}-${stage}`
    let candidate = base
    for (let n = 2; await this.deps.files.exists(manifestPath(join(repository.commonDir, 'critique', candidate))); n += 1) {
      candidate = `${base}-${String(n)}`
    }
    return candidate
  }

  /**
   * Writes the artifact to `artifact/` and copies each reference to
   * `references/` under its repository path. Nothing else is ever put at the
   * packet root, so a `.vibe/`, `.agents/` or `AGENTS.md` there can never
   * become critic config.
   */
  private async freezePacket(
    repository: ResolvedRepository,
    packetDir: string,
    artifact: TrackedArtifact,
    references: readonly string[],
  ): Promise<readonly PacketFile[]> {
    const files = [
      { kind: 'artifact' as const, path: join('artifact', artifact.name), source: artifact.reference, text: artifact.text },
      ...(await Promise.all(
        references.map(async (source) => ({
          kind: 'reference' as const,
          path: join('references', source),
          source,
          text: await this.deps.files.readText(join(repository.root, source)),
        })),
      )),
    ]
    const frozen: PacketFile[] = []
    for (const { text, ...file } of files) {
      await this.deps.files.writeText(join(packetDir, file.path), text)
      frozen.push({ ...file, sha256: sha256(text) })
    }
    return frozen
  }
}
