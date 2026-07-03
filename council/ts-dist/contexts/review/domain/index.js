export const DEFAULT_REVIEWERS = [
    'spec-adherence-reviewer',
    'implementation-reviewer',
    'test-reviewer',
];
const DEFAULT_LEGACY_VERIFIER = 'legacy-single-verifier';
class FixedReviewerSelectionPolicy {
    reviewerCount;
    constructor(reviewerCount) {
        this.reviewerCount = reviewerCount;
    }
    selectFrom(reviewerPool) {
        if (reviewerPool.length < this.reviewerCount) {
            throw new RangeError(`reviewer pool must contain at least ${String(this.reviewerCount)} reviewer(s)`);
        }
        return reviewerPool.slice(0, this.reviewerCount);
    }
}
class TrivialReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
    constructor() {
        super(1);
    }
}
class ModerateReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
    constructor() {
        super(2);
    }
}
class HardReviewerSelectionPolicy extends FixedReviewerSelectionPolicy {
    constructor() {
        super(3);
    }
}
const REVIEWER_SELECTION_POLICIES = {
    trivial: new TrivialReviewerSelectionPolicy(),
    moderate: new ModerateReviewerSelectionPolicy(),
    hard: new HardReviewerSelectionPolicy(),
};
class LegacyReviewPlanningPolicy {
    plan(input) {
        return {
            mode: 'legacy-single-verifier',
            reviewer: input.legacyVerifier ?? DEFAULT_LEGACY_VERIFIER,
            verify: input.task.verify,
        };
    }
}
class CouncilReviewPlanningPolicy {
    plan(input) {
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
        };
    }
}
const REVIEW_PLANNING_POLICIES = {
    council: new CouncilReviewPlanningPolicy(),
    legacy: new LegacyReviewPlanningPolicy(),
};
class SpecAdherenceChecklistLens {
    input;
    lens = 'spec-adherence';
    constructor(input) {
        this.input = input;
    }
    items() {
        return [
            ...this.input.storyAcceptanceCriteria.map((criterion, index) => checklistItem(`story-ac-${String(index + 1)}`, this.lens, `Compare the diff to story AC ${String(index + 1)}: ${criterion}`, `story.ac[${String(index + 1)}]`)),
            ...(this.input.taskAcceptanceCriteria ?? []).map((criterion, index) => checklistItem(`task-ac-${String(index + 1)}`, this.lens, `Compare the diff to task AC ${String(index + 1)}: ${criterion}`, `task.ac[${String(index + 1)}]`)),
            ...(this.input.lockedSpecSections ?? []).map((section) => checklistItem(`locked-spec-${section.section}`, this.lens, `Compare the diff to locked-spec §${section.section}: ${section.requirement}`, `locked-spec §${section.section}`)),
        ];
    }
}
class DuplicatedCodeChecklistLens {
    paths;
    lens = 'duplicated-code';
    constructor(paths) {
        this.paths = paths;
    }
    items() {
        const scope = this.paths.length === 0 ? 'the diff' : this.paths.join(', ');
        return [
            checklistItem('duplicated-code-search', this.lens, `Check ${scope} for copied logic, repeated conditionals, or duplicate data tables.`),
            checklistItem('duplicated-code-resolution', this.lens, 'Require a blocking finding when duplicated production code should be shared instead.'),
        ];
    }
}
class BlockingFindingSeverityPolicy {
    severity = 'blocking';
    finding(claim, evidence, acRef) {
        return reviewerFinding(this.severity, claim, evidence, acRef);
    }
    merge() {
        return this;
    }
    collectFix(state, finding) {
        return state.withBlockingFinding(finding);
    }
}
class AdvisoryFindingSeverityPolicy {
    severity = 'advisory';
    finding(claim, evidence, acRef) {
        return reviewerFinding(this.severity, claim, evidence, acRef);
    }
    merge(next) {
        return next;
    }
    collectFix(state) {
        return state;
    }
}
const BLOCKING_FINDING_SEVERITY = new BlockingFindingSeverityPolicy();
const ADVISORY_FINDING_SEVERITY = new AdvisoryFindingSeverityPolicy();
const FINDING_SEVERITY_POLICIES = {
    blocking: BLOCKING_FINDING_SEVERITY,
    advisory: ADVISORY_FINDING_SEVERITY,
};
class ApprovingReviewVerdictState {
    decision = 'approve';
    withBlockingFinding(finding) {
        return new ChangesRequestedReviewVerdictState([finding]);
    }
    fixes() {
        return [];
    }
}
class ChangesRequestedReviewVerdictState {
    blockingFindings;
    decision = 'changes-requested';
    constructor(blockingFindings) {
        this.blockingFindings = blockingFindings;
    }
    withBlockingFinding(finding) {
        return new ChangesRequestedReviewVerdictState([...this.blockingFindings, finding]);
    }
    fixes(maxFixes) {
        return this.blockingFindings.slice(0, boundedFixLimit(maxFixes)).map(toFix);
    }
}
export function blockingFinding(claim, evidence, acRef) {
    return BLOCKING_FINDING_SEVERITY.finding(claim, evidence, acRef);
}
export function advisoryFinding(claim, evidence, acRef) {
    return ADVISORY_FINDING_SEVERITY.finding(claim, evidence, acRef);
}
export function reviewerCountForDifficulty(difficulty) {
    return reviewerSelectionPolicyFor(difficulty).reviewerCount;
}
export function selectReviewers(difficulty, reviewerPool = DEFAULT_REVIEWERS) {
    return reviewerSelectionPolicyFor(difficulty).selectFrom(reviewerPool);
}
export function planReviewStrategy(input) {
    return reviewPlanningPolicyFor(input).plan(input);
}
export function buildSpecAdherenceLens(input) {
    return new SpecAdherenceChecklistLens(input).items();
}
export function buildDuplicatedCodeChecklist(paths = []) {
    return new DuplicatedCodeChecklistLens(paths).items();
}
export function triageReviewFindings(input) {
    return new ReviewFindingTriage(input).verdict();
}
function reviewerSelectionPolicyFor(difficulty) {
    return REVIEWER_SELECTION_POLICIES[difficulty];
}
function reviewPlanningPolicyFor(input) {
    return REVIEW_PLANNING_POLICIES[reviewPlanningPolicyKeyFor(input)];
}
function reviewPlanningPolicyKeyFor(input) {
    return input.review?.council === true ? 'council' : 'legacy';
}
function reviewerFinding(severity, claim, evidence, acRef) {
    if (acRef === undefined) {
        return { severity, claim, evidence };
    }
    return { severity, claim, evidence, ac_ref: acRef };
}
function checklistItem(id, lens, prompt, ref) {
    if (ref === undefined) {
        return { id, lens, prompt };
    }
    return { id, lens, prompt, ref };
}
function mergeFindings(reports, activeFindings) {
    const reviewersByFinding = new Map();
    for (const report of reports) {
        for (const finding of report.findings) {
            reviewersByFinding.set(finding, report.reviewer);
        }
    }
    const merged = new Map();
    for (const activeFinding of activeFindings) {
        const key = findingKey(activeFinding);
        const existing = merged.get(key);
        const reviewer = reviewersByFinding.get(activeFinding);
        if (existing === undefined) {
            merged.set(key, {
                severity: activeFinding.severity,
                claim: activeFinding.claim,
                evidence: [activeFinding.evidence],
                reviewers: reviewer === undefined ? [] : [reviewer],
                ac_ref: activeFinding.ac_ref,
            });
            continue;
        }
        existing.severity = mergeSeverity(existing.severity, activeFinding.severity);
        pushUnique(existing.evidence, activeFinding.evidence);
        if (reviewer !== undefined) {
            pushUnique(existing.reviewers, reviewer);
        }
    }
    return [...merged.values()].map(freezeMergedFinding);
}
function freezeMergedFinding(finding) {
    if (finding.ac_ref === undefined) {
        return {
            severity: finding.severity,
            claim: finding.claim,
            evidence: finding.evidence,
            reviewers: finding.reviewers,
        };
    }
    return {
        severity: finding.severity,
        claim: finding.claim,
        evidence: finding.evidence,
        reviewers: finding.reviewers,
        ac_ref: finding.ac_ref,
    };
}
function mergeSeverity(current, next) {
    return severityPolicyFor(current).merge(severityPolicyFor(next)).severity;
}
function severityPolicyFor(severity) {
    return FINDING_SEVERITY_POLICIES[severity];
}
function toFix(finding) {
    if (finding.ac_ref === undefined) {
        return {
            claim: finding.claim,
            evidence: finding.evidence,
            reviewers: finding.reviewers,
        };
    }
    return {
        claim: finding.claim,
        evidence: finding.evidence,
        reviewers: finding.reviewers,
        ac_ref: finding.ac_ref,
    };
}
function boundedFixLimit(maxFixes) {
    return Math.max(0, Math.floor(maxFixes ?? 5));
}
function findingKey(finding) {
    return `${normalize(finding.claim)}\u0000${finding.ac_ref ?? ''}`;
}
function normalize(value) {
    return value.trim().replaceAll(/\s+/g, ' ').toLocaleLowerCase();
}
function pushUnique(values, value) {
    if (!values.includes(value)) {
        values.push(value);
    }
}
class ReviewFindingTriage {
    input;
    constructor(input) {
        this.input = input;
    }
    verdict() {
        const allFindings = this.input.reports.flatMap((report) => report.findings);
        const refutationPolicy = new ReviewRefutationPolicySet(this.input.refutations ?? []);
        const refutedFindings = allFindings.filter((finding) => refutationPolicy.refutes(finding));
        const activeFindings = allFindings.filter((finding) => !refutationPolicy.refutes(finding));
        const findings = mergeFindings(this.input.reports, activeFindings);
        const verdictState = verdictStateFor(findings);
        return {
            decision: verdictState.decision,
            findings,
            refuted_findings: refutedFindings,
            fix_list: verdictState.fixes(this.input.maxFixes),
        };
    }
}
class ReviewRefutationPolicySet {
    policies;
    constructor(refutations) {
        this.policies = refutations.map((refutation) => new ReviewRefutationPolicy(refutation));
    }
    refutes(finding) {
        return this.policies.some((policy) => policy.refutes(finding));
    }
}
class ReviewRefutationPolicy {
    refutation;
    acRefPolicy;
    constructor(refutation) {
        this.refutation = refutation;
        this.acRefPolicy = acRefPolicyFor(refutation);
    }
    refutes(finding) {
        return (normalize(this.refutation.claim) === normalize(finding.claim) &&
            this.acRefPolicy.matches(finding));
    }
}
class AnyAcRefPolicy {
    matches() {
        return true;
    }
}
class ExactAcRefPolicy {
    acRef;
    constructor(acRef) {
        this.acRef = acRef;
    }
    matches(finding) {
        return finding.ac_ref === this.acRef;
    }
}
const ANY_AC_REF_POLICY = new AnyAcRefPolicy();
function acRefPolicyFor(refutation) {
    if (refutation.ac_ref === undefined) {
        return ANY_AC_REF_POLICY;
    }
    return new ExactAcRefPolicy(refutation.ac_ref);
}
function verdictStateFor(findings) {
    return findings.reduce((state, finding) => severityPolicyFor(finding.severity).collectFix(state, finding), new ApprovingReviewVerdictState());
}
