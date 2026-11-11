import { relative } from 'node:path'

import type { ArtifactSource } from '../domain/manifest.js'
import { refusal, type Refusal } from '../domain/outcomes.js'
import { PLAN_CHECK_PATH } from '../domain/refactoring-plan.js'
import type { PacketLimits } from '../domain/repository-packet.js'
import { sampleTree, type PreparedRepository } from './adr-packet.js'
import { CheckPlanCommand } from './check-plan.js'
import type { UnfrozenFile } from './code-packet.js'
import { PLAN_FILE } from './plan-file.js'
import type { CritiqueDeps } from './ports.js'
import { decisionReferences } from './recovery-packet.js'

export interface PlanPacketRequest {
  /** The plan file, absolute. */
  readonly path: string
  /** The commit, or branch, the plan targets: its tree is sampled. */
  readonly head: string
  /** Routing's packet limits. */
  readonly limits: PacketLimits
}

export interface PlanPacket {
  readonly status: 'read'
  /** What depth is chosen from: the plan, as one text. */
  readonly artifact: ArtifactSource & { readonly text: string }
  readonly files: readonly UnfrozenFile[]
  readonly repository: PreparedRepository
  /** How many steps the plan has, and how many of its migrate and contract steps touch stored data. */
  readonly steps: number
  readonly stored_data_steps: number
}

/**
 * Reads what the plan stage freezes: the plan under `artifact/`, the
 * checker's report on it under `check/plan.json`, the tree sampled at the
 * head the plan targets (the files its anchors and touches name first),
 * `tree/MANIFEST.tsv` and `facts/`, and under `references/` the accepted
 * ADRs and the glossary. The plan is never committed, so its path may lie in
 * the git common directory; no tracker is read.
 */
export class PlanPacketReader {
  constructor(private readonly deps: CritiqueDeps) {}

  async read(root: string, request: PlanPacketRequest): Promise<PlanPacket | Refusal> {
    const { files } = this.deps
    if (!(await files.exists(request.path))) {
      return refusal('no-plan', `found no refactoring plan at ${request.path}; draft ${PLAN_FILE} there first, or name the plan file with plan`)
    }
    const text = await files.readText(request.path)
    const check = await new CheckPlanCommand(this.deps).checkFile(request.path, root)
    const relativePath = relative(root, request.path)
    const reference = relativePath.startsWith('..') ? request.path : relativePath
    const { documents } = await decisionReferences(this.deps, root)
    const packet: readonly UnfrozenFile[] = [
      { kind: 'artifact', path: `artifact/${PLAN_FILE}`, source: reference, text },
      { kind: 'check', path: PLAN_CHECK_PATH, source: `check-plan ${reference}`, text: `${JSON.stringify(check, null, 2)}\n` },
      ...documents,
    ]
    const sampled = await sampleTree(this.deps, root, { artifactText: text, files: packet }, request)
    if (sampled.status === 'refused') return sampled
    return {
      artifact: { labels: [], reference, text, tracker: 'none' },
      files: sampled.files,
      repository: sampled.repository,
      status: 'read',
      steps: check.steps.length,
      stored_data_steps: check.stored_data_steps.length,
    }
  }
}
