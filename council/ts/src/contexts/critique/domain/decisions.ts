import { CritiqueError } from './outcomes.js'

/**
 * The decisions stage (#183) reviews what grilling decided before a spec is
 * written from it. Its artifact is not a tracker item: it is the decision
 * docs `/grill-with-docs` wrote, or the summary the orchestrator writes after
 * `/grill-me`, which writes none. The orchestrator also states the feature
 * being grilled, so critics review the decisions that bear on it.
 */

/** The most lines the orchestrator's feature statement may have. */
export const FEATURE_MAX_LINES = 5

/** The most lines the orchestrator's decision summary may have. */
export const SUMMARY_MAX_LINES = 40

/** Where an artifact that is no tracker item came from. */
export const DECISION_ARTIFACTS = { docs: 'decision-docs', summary: 'decision-summary' } as const

/** The packet path of the feature statement. */
export const FEATURE_PATH = 'feature/feature.md'

/** The packet file name of the orchestrator's decision summary, under `artifact/`. */
export const SUMMARY_NAME = 'decision-summary.md'

/** The feature statement as the packet freezes it. */
export function featureStatement(feature: string): string {
  return `# Feature\n\n${limitedLines('feature', feature, FEATURE_MAX_LINES)}\n`
}

/** The decision summary as the packet freezes it. */
export function decisionSummary(summary: string): string {
  return `${limitedLines('decision_summary', summary, SUMMARY_MAX_LINES)}\n`
}

function limitedLines(field: string, text: string, max: number): string {
  const trimmed = text.trim()
  const lines = trimmed.split(/\r?\n/).length
  if (lines > max) throw new CritiqueError('invalid-input', `${field} must be at most ${String(max)} lines; it has ${String(lines)}`)
  return trimmed
}
