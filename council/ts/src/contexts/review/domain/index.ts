import type { Story, Task, TaskDifficulty } from '../../../shared-kernel/index.js'

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

interface ReviewerSelectionPolicy {
  readonly reviewerCount: 1 | 2 | 3

  selectFrom(reviewerPool: readonly string[]): readonly string[]
}

abstract class FixedReviewerSelectionPolicy implements ReviewerSelectionPolicy {
  protected constructor(readonly reviewerCount: 1 | 2 | 3) {}

  selectFrom(reviewerPool: readonly string[]): readonly string[] {
    if (reviewerPool.length < this.reviewerCount) {
      throw new RangeError(
        `reviewer pool must contain at least ${String(this.reviewerCount)} reviewer(s)`,
      )
    }

    return reviewerPool.slice(0, this.reviewerCount)
  }
}

class TrivialReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
  constructor() {
    super(1)
  }
}

class ModerateReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
  constructor() {
    super(2)
  }
}

class HardReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
  constructor() {
    super(3)
  }
}

const REVIEWER_SELECTION_POLICIES: Readonly<Record<TaskDifficulty, ReviewerSelectionPolicy>> = {
  trivial: new TrivialReviewerSelectionPolicy(),
  moderate: new ModerateReviewerSelectionPolicy(),
  hard: new HardReviewerSelectionPolicy(),
}

interface ReviewPlanningPolicy {
  plan(input: ReviewStrategyInput): ReviewStrategy
}

class LegacyReviewPlanningPolicy implements ReviewPlanningPolicy {
  plan(input: ReviewStrategyInput): LegacySingleVerifierStrategy {
    return {
      mode: 'legacy-single-verifier',
      reviewer: input.legacyVerifier ?? DEFAULT_LEGACY_VERIFIER,
      verify: input.task.verify,
    }
  }
}

class CouncilReviewPlanningPolicy implements ReviewPlanningPolicy {
  plan(input: ReviewStrategyInput): ReviewCouncilStrategy {
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
}

type ReviewPlanningPolicyKey = 'council' | 'legacy'

const REVIEW_PLANNING_POLICIES: Readonly<Record<ReviewPlanningPolicyKey, ReviewPlanningPolicy>> =
  {
    council: new CouncilReviewPlanningPolicy(),
    legacy: new LegacyReviewPlanningPolicy(),
  }

interface ChecklistLensStrategy {
  readonly lens: ReviewChecklistLens

  items(): readonly ReviewChecklistItem[]
}

class SpecAdherenceChecklistLens implements ChecklistLensStrategy {
  readonly lens = 'spec-adherence'

  constructor(private readonly input: SpecAdherenceLensInput) {}

  items(): readonly ReviewChecklistItem[] {
    return [
      ...this.input.storyAcceptanceCriteria.map((criterion, index) =>
        checklistItem(
          `story-ac-${String(index + 1)}`,
          this.lens,
          `Compare the diff to story AC ${String(index + 1)}: ${criterion}`,
          `story.ac[${String(index + 1)}]`,
        ),
      ),
      ...(this.input.taskAcceptanceCriteria ?? []).map((criterion, index) =>
        checklistItem(
          `task-ac-${String(index + 1)}`,
          this.lens,
          `Compare the diff to task AC ${String(index + 1)}: ${criterion}`,
          `task.ac[${String(index + 1)}]`,
        ),
      ),
      ...(this.input.lockedSpecSections ?? []).map((section) =>
        checklistItem(
          `locked-spec-${section.section}`,
          this.lens,
          `Compare the diff to locked-spec §${section.section}: ${section.requirement}`,
          `locked-spec §${section.section}`,
        ),
      ),
    ]
  }
}

class DuplicatedCodeChecklistLens implements ChecklistLensStrategy {
  readonly lens = 'duplicated-code'

  constructor(private readonly paths: readonly string[]) {}

  items(): readonly ReviewChecklistItem[] {
    const scope = this.paths.length === 0 ? 'the diff' : this.paths.join(', ')

    return [
      checklistItem(
        'duplicated-code-search',
        this.lens,
        `Check ${scope} for copied logic, repeated conditionals, or duplicate data tables.`,
      ),
      checklistItem(
        'duplicated-code-resolution',
        this.lens,
        'Require a blocking finding when duplicated production code should be shared instead.',
      ),
    ]
  }
}

interface FindingSeverityPolicy {
  readonly severity: ReviewFindingSeverity

  finding(claim: string, evidence: string, acRef?: string): ReviewerFinding
  merge(next: FindingSeverityPolicy): FindingSeverityPolicy
  collectFix(state: ReviewVerdictState, finding: MergedReviewFinding): ReviewVerdictState
}

class BlockingFindingSeverityPolicy implements FindingSeverityPolicy {
  readonly severity = 'blocking'

  finding(claim: string, evidence: string, acRef?: string): ReviewerFinding {
    return reviewerFinding(this.severity, claim, evidence, acRef)
  }

  merge(): FindingSeverityPolicy {
    return this
  }

  collectFix(state: ReviewVerdictState, finding: MergedReviewFinding): ReviewVerdictState {
    return state.withBlockingFinding(finding)
  }
}

class AdvisoryFindingSeverityPolicy implements FindingSeverityPolicy {
  readonly severity = 'advisory'

  finding(claim: string, evidence: string, acRef?: string): ReviewerFinding {
    return reviewerFinding(this.severity, claim, evidence, acRef)
  }

  merge(next: FindingSeverityPolicy): FindingSeverityPolicy {
    return next
  }

  collectFix(state: ReviewVerdictState): ReviewVerdictState {
    return state
  }
}

const BLOCKING_FINDING_SEVERITY = new BlockingFindingSeverityPolicy()
const ADVISORY_FINDING_SEVERITY = new AdvisoryFindingSeverityPolicy()

const FINDING_SEVERITY_POLICIES: Readonly<
  Record<ReviewFindingSeverity, FindingSeverityPolicy>
> = {
  blocking: BLOCKING_FINDING_SEVERITY,
  advisory: ADVISORY_FINDING_SEVERITY,
}

interface ReviewVerdictState {
  readonly decision: ReviewDecision

  withBlockingFinding(finding: MergedReviewFinding): ReviewVerdictState
  fixes(maxFixes: number | undefined): readonly ReviewFix[]
}

class ApprovingReviewVerdictState implements ReviewVerdictState {
  readonly decision = 'approve'

  withBlockingFinding(finding: MergedReviewFinding): ReviewVerdictState {
    return new ChangesRequestedReviewVerdictState([finding])
  }

  fixes(): readonly ReviewFix[] {
    return []
  }
}

class ChangesRequestedReviewVerdictState implements ReviewVerdictState {
  readonly decision = 'changes-requested'

  constructor(private readonly blockingFindings: readonly MergedReviewFinding[]) {}

  withBlockingFinding(finding: MergedReviewFinding): ReviewVerdictState {
    return new ChangesRequestedReviewVerdictState([...this.blockingFindings, finding])
  }

  fixes(maxFixes: number | undefined): readonly ReviewFix[] {
    return this.blockingFindings.slice(0, boundedFixLimit(maxFixes)).map(toFix)
  }
}

export function blockingFinding(
  claim: string,
  evidence: string,
  acRef?: string,
): ReviewerFinding {
  return BLOCKING_FINDING_SEVERITY.finding(claim, evidence, acRef)
}

export function advisoryFinding(
  claim: string,
  evidence: string,
  acRef?: string,
): ReviewerFinding {
  return ADVISORY_FINDING_SEVERITY.finding(claim, evidence, acRef)
}

export function reviewerCountForDifficulty(difficulty: TaskDifficulty): 1 | 2 | 3 {
  return reviewerSelectionPolicyFor(difficulty).reviewerCount
}

export function selectReviewers(
  difficulty: TaskDifficulty,
  reviewerPool: readonly string[] = DEFAULT_REVIEWERS,
): readonly string[] {
  return reviewerSelectionPolicyFor(difficulty).selectFrom(reviewerPool)
}

export function planReviewStrategy(input: ReviewStrategyInput): ReviewStrategy {
  return reviewPlanningPolicyFor(input).plan(input)
}

export function buildSpecAdherenceLens(
  input: SpecAdherenceLensInput,
): readonly ReviewChecklistItem[] {
  return new SpecAdherenceChecklistLens(input).items()
}

export function buildDuplicatedCodeChecklist(
  paths: readonly string[] = [],
): readonly ReviewChecklistItem[] {
  return new DuplicatedCodeChecklistLens(paths).items()
}

export function triageReviewFindings(input: ReviewTriageInput): ReviewTriageVerdict {
  return new ReviewFindingTriage(input).verdict()
}

function reviewerSelectionPolicyFor(difficulty: TaskDifficulty): ReviewerSelectionPolicy {
  return REVIEWER_SELECTION_POLICIES[difficulty]
}

function reviewPlanningPolicyFor(input: ReviewStrategyInput): ReviewPlanningPolicy {
  return REVIEW_PLANNING_POLICIES[reviewPlanningPolicyKeyFor(input)]
}

function reviewPlanningPolicyKeyFor(input: ReviewStrategyInput): ReviewPlanningPolicyKey {
  return input.review?.council === true ? 'council' : 'legacy'
}

function reviewerFinding(
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
  return severityPolicyFor(current).merge(severityPolicyFor(next)).severity
}

function severityPolicyFor(severity: ReviewFindingSeverity): FindingSeverityPolicy {
  return FINDING_SEVERITY_POLICIES[severity]
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

class ReviewFindingTriage {
  constructor(private readonly input: ReviewTriageInput) {}

  verdict(): ReviewTriageVerdict {
    const allFindings = this.input.reports.flatMap((report) => report.findings)
    const refutationPolicy = new ReviewRefutationPolicySet(this.input.refutations ?? [])
    const refutedFindings = allFindings.filter((finding) => refutationPolicy.refutes(finding))
    const activeFindings = allFindings.filter((finding) => !refutationPolicy.refutes(finding))
    const findings = mergeFindings(this.input.reports, activeFindings)
    const verdictState = verdictStateFor(findings)

    return {
      decision: verdictState.decision,
      findings,
      refuted_findings: refutedFindings,
      fix_list: verdictState.fixes(this.input.maxFixes),
    }
  }
}

class ReviewRefutationPolicySet {
  private readonly policies: readonly ReviewRefutationPolicy[]

  constructor(refutations: readonly ReviewFindingRefutation[]) {
    this.policies = refutations.map((refutation) => new ReviewRefutationPolicy(refutation))
  }

  refutes(finding: ReviewerFinding): boolean {
    return this.policies.some((policy) => policy.refutes(finding))
  }
}

class ReviewRefutationPolicy {
  private readonly acRefPolicy: RefutationAcRefPolicy

  constructor(private readonly refutation: ReviewFindingRefutation) {
    this.acRefPolicy = acRefPolicyFor(refutation)
  }

  refutes(finding: ReviewerFinding): boolean {
    return (
      normalize(this.refutation.claim) === normalize(finding.claim) &&
      this.acRefPolicy.matches(finding)
    )
  }
}

interface RefutationAcRefPolicy {
  matches(finding: ReviewerFinding): boolean
}

class AnyAcRefPolicy implements RefutationAcRefPolicy {
  matches(): boolean {
    return true
  }
}

class ExactAcRefPolicy implements RefutationAcRefPolicy {
  constructor(private readonly acRef: string) {}

  matches(finding: ReviewerFinding): boolean {
    return finding.ac_ref === this.acRef
  }
}

const ANY_AC_REF_POLICY = new AnyAcRefPolicy()

function acRefPolicyFor(refutation: ReviewFindingRefutation): RefutationAcRefPolicy {
  if (refutation.ac_ref === undefined) {
    return ANY_AC_REF_POLICY
  }

  return new ExactAcRefPolicy(refutation.ac_ref)
}

function verdictStateFor(findings: readonly MergedReviewFinding[]): ReviewVerdictState {
  return findings.reduce<ReviewVerdictState>(
    (state, finding) => severityPolicyFor(finding.severity).collectFix(state, finding),
    new ApprovingReviewVerdictState(),
  )
}
