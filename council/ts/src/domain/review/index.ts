import type { Story, Task, TaskDifficulty } from '../contracts/index.js'

export type ReviewFindingSeverity = 'blocking' | 'advisory'

export type ReviewDecision = 'approve' | 'changes-requested'

export interface ReviewerFinding {
  readonly severity: ReviewFindingSeverity
  readonly claim: string
  readonly evidence: string
  readonly ac_ref?: string
}

export interface ReviewerReport {
  readonly reviewer: string
  readonly findings: readonly ReviewerFinding[]
}

export interface ReviewFindingRefutation {
  readonly claim: string
  readonly reason: string
  readonly ac_ref?: string
}

export interface MergedReviewFinding {
  readonly severity: ReviewFindingSeverity
  readonly claim: string
  readonly evidence: readonly string[]
  readonly reviewers: readonly string[]
  readonly ac_ref?: string
}

export interface ReviewFix {
  readonly claim: string
  readonly evidence: readonly string[]
  readonly reviewers: readonly string[]
  readonly ac_ref?: string
}

export interface ReviewTriageInput {
  readonly reports: readonly ReviewerReport[]
  readonly refutations?: readonly ReviewFindingRefutation[]
  readonly maxFixes?: number
}

export interface ReviewTriageVerdict {
  readonly decision: ReviewDecision
  readonly findings: readonly MergedReviewFinding[]
  readonly refuted_findings: readonly ReviewerFinding[]
  readonly fix_list: readonly ReviewFix[]
}

export interface LockedSpecSection {
  readonly section: string
  readonly requirement: string
  readonly title?: string
}

export type ReviewChecklistLens = 'spec-adherence' | 'duplicated-code'

export interface ReviewChecklistItem {
  readonly id: string
  readonly lens: ReviewChecklistLens
  readonly prompt: string
  readonly ref?: string
}

export interface SpecAdherenceLensInput {
  readonly storyAcceptanceCriteria: readonly string[]
  readonly taskAcceptanceCriteria?: readonly string[] | undefined
  readonly lockedSpecSections?: readonly LockedSpecSection[] | undefined
}

export interface ReviewStrategyInput {
  readonly review?: {
    readonly council?: boolean
  }
  readonly task: Pick<Task, 'acceptance_criteria' | 'difficulty' | 'paths' | 'verify'>
  readonly story: Pick<Story, 'acceptance_criteria'>
  readonly lockedSpecSections?: readonly LockedSpecSection[]
  readonly reviewerPool?: readonly string[]
  readonly legacyVerifier?: string
}

export interface LegacySingleVerifierStrategy {
  readonly mode: 'legacy-single-verifier'
  readonly reviewer: string
  readonly verify: string
}

export interface ReviewCouncilStrategy {
  readonly mode: 'review-council'
  readonly reviewers: readonly string[]
  readonly checklist: readonly ReviewChecklistItem[]
}

export type ReviewStrategy = LegacySingleVerifierStrategy | ReviewCouncilStrategy

export const DEFAULT_REVIEWERS = [
  'spec-adherence-reviewer',
  'implementation-reviewer',
  'test-reviewer',
] as const

const DEFAULT_LEGACY_VERIFIER = 'legacy-single-verifier'

const REVIEWER_COUNTS: Readonly<Record<TaskDifficulty, 1 | 2 | 3>> = {
  trivial: 1,
  moderate: 2,
  hard: 3,
}

export function blockingFinding(
  claim: string,
  evidence: string,
  acRef?: string,
): ReviewerFinding {
  return finding('blocking', claim, evidence, acRef)
}

export function advisoryFinding(
  claim: string,
  evidence: string,
  acRef?: string,
): ReviewerFinding {
  return finding('advisory', claim, evidence, acRef)
}

export function reviewerCountForDifficulty(difficulty: TaskDifficulty): 1 | 2 | 3 {
  return REVIEWER_COUNTS[difficulty]
}

export function selectReviewers(
  difficulty: TaskDifficulty,
  reviewerPool: readonly string[] = DEFAULT_REVIEWERS,
): readonly string[] {
  const required = reviewerCountForDifficulty(difficulty)

  if (reviewerPool.length < required) {
    throw new RangeError(`reviewer pool must contain at least ${String(required)} reviewer(s)`)
  }

  return reviewerPool.slice(0, required)
}

export function planReviewStrategy(input: ReviewStrategyInput): ReviewStrategy {
  if (input.review?.council !== true) {
    return {
      mode: 'legacy-single-verifier',
      reviewer: input.legacyVerifier ?? DEFAULT_LEGACY_VERIFIER,
      verify: input.task.verify,
    }
  }

  return {
    mode: 'review-council',
    reviewers: selectReviewers(input.task.difficulty, input.reviewerPool),
    checklist: [
      ...buildSpecAdherenceLens({
        storyAcceptanceCriteria: input.story.acceptance_criteria,
        taskAcceptanceCriteria: input.task.acceptance_criteria,
        lockedSpecSections: input.lockedSpecSections,
      }),
      ...buildDuplicatedCodeChecklist(input.task.paths),
    ],
  }
}

export function buildSpecAdherenceLens(
  input: SpecAdherenceLensInput,
): readonly ReviewChecklistItem[] {
  return [
    ...input.storyAcceptanceCriteria.map((criterion, index) =>
      checklistItem(
        `story-ac-${String(index + 1)}`,
        'spec-adherence',
        `Compare the diff to story AC ${String(index + 1)}: ${criterion}`,
        `story.ac[${String(index + 1)}]`,
      ),
    ),
    ...(input.taskAcceptanceCriteria ?? []).map((criterion, index) =>
      checklistItem(
        `task-ac-${String(index + 1)}`,
        'spec-adherence',
        `Compare the diff to task AC ${String(index + 1)}: ${criterion}`,
        `task.ac[${String(index + 1)}]`,
      ),
    ),
    ...(input.lockedSpecSections ?? []).map((section) =>
      checklistItem(
        `locked-spec-${section.section}`,
        'spec-adherence',
        `Compare the diff to locked-spec §${section.section}: ${section.requirement}`,
        `locked-spec §${section.section}`,
      ),
    ),
  ]
}

export function buildDuplicatedCodeChecklist(
  paths: readonly string[] = [],
): readonly ReviewChecklistItem[] {
  const scope = paths.length === 0 ? 'the diff' : paths.join(', ')

  return [
    checklistItem(
      'duplicated-code-search',
      'duplicated-code',
      `Check ${scope} for copied logic, repeated conditionals, or duplicate data tables.`,
    ),
    checklistItem(
      'duplicated-code-resolution',
      'duplicated-code',
      'Require a blocking finding when duplicated production code should be shared instead.',
    ),
  ]
}

export function triageReviewFindings(input: ReviewTriageInput): ReviewTriageVerdict {
  const allFindings = input.reports.flatMap((report) => report.findings)
  const refutedFindings = allFindings.filter((finding) =>
    isRefuted(finding, input.refutations ?? []),
  )
  const activeFindings = allFindings.filter(
    (finding) => !isRefuted(finding, input.refutations ?? []),
  )
  const findings = mergeFindings(input.reports, activeFindings)
  const blockingFindings = findings.filter((finding) => finding.severity === 'blocking')
  const boundedFixes = blockingFindings.slice(0, boundedFixLimit(input.maxFixes)).map(toFix)

  return {
    decision: blockingFindings.length === 0 ? 'approve' : 'changes-requested',
    findings,
    refuted_findings: refutedFindings,
    fix_list: boundedFixes,
  }
}

function finding(
  severity: ReviewFindingSeverity,
  claim: string,
  evidence: string,
  acRef?: string,
): ReviewerFinding {
  if (acRef === undefined) {
    return { severity, claim, evidence }
  }

  return { severity, claim, evidence, ac_ref: acRef }
}

function checklistItem(
  id: string,
  lens: ReviewChecklistLens,
  prompt: string,
  ref?: string,
): ReviewChecklistItem {
  if (ref === undefined) {
    return { id, lens, prompt }
  }

  return { id, lens, prompt, ref }
}

function mergeFindings(
  reports: readonly ReviewerReport[],
  activeFindings: readonly ReviewerFinding[],
): readonly MergedReviewFinding[] {
  const reviewersByFinding = new Map<ReviewerFinding, string>()

  for (const report of reports) {
    for (const finding of report.findings) {
      reviewersByFinding.set(finding, report.reviewer)
    }
  }

  const merged = new Map<string, MutableMergedReviewFinding>()

  for (const activeFinding of activeFindings) {
    const key = findingKey(activeFinding)
    const existing = merged.get(key)
    const reviewer = reviewersByFinding.get(activeFinding)

    if (existing === undefined) {
      merged.set(key, {
        severity: activeFinding.severity,
        claim: activeFinding.claim,
        evidence: [activeFinding.evidence],
        reviewers: reviewer === undefined ? [] : [reviewer],
        ac_ref: activeFinding.ac_ref,
      })
      continue
    }

    existing.severity = mergeSeverity(existing.severity, activeFinding.severity)
    pushUnique(existing.evidence, activeFinding.evidence)

    if (reviewer !== undefined) {
      pushUnique(existing.reviewers, reviewer)
    }
  }

  return [...merged.values()].map(freezeMergedFinding)
}

interface MutableMergedReviewFinding {
  severity: ReviewFindingSeverity
  claim: string
  evidence: string[]
  reviewers: string[]
  ac_ref: string | undefined
}

function freezeMergedFinding(finding: MutableMergedReviewFinding): MergedReviewFinding {
  if (finding.ac_ref === undefined) {
    return {
      severity: finding.severity,
      claim: finding.claim,
      evidence: finding.evidence,
      reviewers: finding.reviewers,
    }
  }

  return {
    severity: finding.severity,
    claim: finding.claim,
    evidence: finding.evidence,
    reviewers: finding.reviewers,
    ac_ref: finding.ac_ref,
  }
}

function mergeSeverity(
  current: ReviewFindingSeverity,
  next: ReviewFindingSeverity,
): ReviewFindingSeverity {
  return current === 'blocking' || next === 'blocking' ? 'blocking' : 'advisory'
}

function isRefuted(
  finding: ReviewerFinding,
  refutations: readonly ReviewFindingRefutation[],
): boolean {
  return refutations.some(
    (refutation) =>
      normalize(refutation.claim) === normalize(finding.claim) &&
      (refutation.ac_ref === undefined || refutation.ac_ref === finding.ac_ref),
  )
}

function toFix(finding: MergedReviewFinding): ReviewFix {
  if (finding.ac_ref === undefined) {
    return {
      claim: finding.claim,
      evidence: finding.evidence,
      reviewers: finding.reviewers,
    }
  }

  return {
    claim: finding.claim,
    evidence: finding.evidence,
    reviewers: finding.reviewers,
    ac_ref: finding.ac_ref,
  }
}

function boundedFixLimit(maxFixes: number | undefined): number {
  return Math.max(0, Math.floor(maxFixes ?? 5))
}

function findingKey(finding: ReviewerFinding): string {
  return `${normalize(finding.claim)}\u0000${finding.ac_ref ?? ''}`
}

function normalize(value: string): string {
  return value.trim().replaceAll(/\s+/g, ' ').toLocaleLowerCase()
}

function pushUnique(values: string[], value: string): void {
  if (!values.includes(value)) {
    values.push(value)
  }
}
