import { join } from 'node:path'

import {
  buildTriageGatePayload,
  classifyTriage,
  recommendLenses,
  triageLensProfile,
  type TriageGatePayload,
  type TriageInput,
} from '../contexts/triage/index.js'

export interface TriageWorkflowInput {
  readonly triage: TriageInput
  readonly runDir?: string
  readonly signals?: readonly string[]
}

export interface TriageWorkflowWrite {
  readonly path: string
  readonly text: string
}

export interface TriageWorkflowDeps {
  readonly writeText?: (path: string, text: string) => Promise<void>
}

export async function triageWorkflow(
  input: TriageWorkflowInput,
  deps: TriageWorkflowDeps = {},
): Promise<TriageGatePayload> {
  const recommendation = recommendLenses(triageLensProfile(input.triage, input.signals))
  const payload = buildTriageGatePayload({
    input: input.triage,
    recommendation,
    verdict: classifyTriage(input.triage),
  })
  if (input.runDir !== undefined && deps.writeText !== undefined) {
    await deps.writeText(join(input.runDir, 'triage.json'), `${JSON.stringify(payload, null, 2)}\n`)
  }
  return payload
}
