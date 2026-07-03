import matrixJson from './routing-matrix.json' with { type: 'json' };
export * from './recommendation.js';
const tierRank = {
    skip: 0,
    haiku: 1,
    sonnet: 2,
    opus: 3,
};
export function loadRoutingMatrix(value = matrixJson) {
    return parseRoutingMatrix(value);
}
export function parseRoutingMatrix(value) {
    if (!isRecord(value) || value.$schema !== './routing-matrix.schema.json' || value.schemaVersion !== 1) {
        throw new TypeError('Routing matrix must declare schema version 1');
    }
    if (!Array.isArray(value.routeProfiles) || !Array.isArray(value.routeRules) || !Array.isArray(value.stageAdjustments)) {
        throw new TypeError('Routing matrix must include profiles, rules, and stage adjustments');
    }
    return value;
}
export const routingMatrix = loadRoutingMatrix();
export function classifyTriage(input, matrix = routingMatrix) {
    const matches = matrix.routeRules.filter((rule) => matchesCondition(input, rule.when));
    const selectedRule = matches[0];
    if (selectedRule === undefined) {
        throw new Error('Routing matrix has no fallback rule');
    }
    const profile = matrix.routeProfiles.find((routeProfile) => routeProfile.route === selectedRule.route);
    if (profile === undefined) {
        throw new Error(`Routing matrix has no profile for ${selectedRule.route}`);
    }
    const candidateRules = matches.length > 1 ? matches.filter((rule) => hasSpecificCondition(rule.when)) : matches;
    const appliedAdjustments = matrix.stageAdjustments.filter((adjustment) => matchesCondition(input, adjustment.when));
    return {
        route: selectedRule.route,
        matchedRuleId: selectedRule.id,
        candidateRoutes: uniqueRoutes(candidateRules.map((rule) => rule.route)),
        reasons: [selectedRule.reason, ...appliedAdjustments.map((adjustment) => adjustment.reason)],
        useCaseRefs: selectedRule.useCaseRefs,
        stageTiers: applyStageAdjustments(profile.stageTiers, appliedAdjustments),
        plan: {
            dagShape: profile.dagShape,
            executesWorkers: profile.planExecutesWorkers,
            directWorkerPolicy: 'never-during-plan',
        },
    };
}
export function matchesCondition(input, condition) {
    return (includesOrAny(condition.size, input.size) &&
        includesOrAny(condition.landscape, input.landscape) &&
        includesOrAny(condition.kind, input.kind) &&
        includesOrAny(condition.risk, input.risk) &&
        includesOrAny(condition.clarity, input.clarity) &&
        includesOrAny(condition.parallelism, input.parallelism));
}
function applyStageAdjustments(base, adjustments) {
    return adjustments.reduce((stageTiers, adjustment) => mergeMinTiers(stageTiers, adjustment.minTiers), { ...base });
}
function mergeMinTiers(stageTiers, minTiers) {
    return {
        grill: maxTier(stageTiers.grill, minTiers.grill),
        survey: maxTier(stageTiers.survey, minTiers.survey),
        plan: maxTier(stageTiers.plan, minTiers.plan),
        critique: maxTier(stageTiers.critique, minTiers.critique),
        consolidate: maxTier(stageTiers.consolidate, minTiers.consolidate),
        tasking: maxTier(stageTiers.tasking, minTiers.tasking),
        verify: maxTier(stageTiers.verify, minTiers.verify),
    };
}
function maxTier(current, minimum) {
    return minimum === undefined || tierRank[current] >= tierRank[minimum] ? current : minimum;
}
function includesOrAny(allowed, value) {
    return allowed === undefined || allowed.includes(value);
}
function hasSpecificCondition(condition) {
    return Object.keys(condition).length > 0;
}
function uniqueRoutes(routes) {
    return [...new Set(routes)];
}
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
