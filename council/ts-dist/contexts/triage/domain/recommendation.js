import catalogJson from './lens-catalog.json' with { type: 'json' };
const lensKinds = [
    'api',
    'architecture',
    'data',
    'feature',
    'infra',
    'library',
    'migration',
    'performance',
    'program',
    'refactor',
    'security',
    'test',
    'ui',
];
const lensRisks = ['low', 'medium', 'high', 'critical'];
const lensSizes = ['small', 'medium', 'large', 'program'];
const lensTiers = ['codex-medium', 'codex-high', 'codex-xhigh'];
const typicalRounds = [1, 2, 3];
const kindAliases = {
    bugfix: ['feature', 'test', 'infra'],
    'cross-cutting': ['infra', 'refactor', 'architecture'],
    'design-system': ['ui', 'library'],
    hotfix: ['infra', 'feature', 'test'],
    maintenance: ['refactor', 'infra'],
    prototype: ['feature', 'ui', 'api'],
    'ui-tweak': ['ui', 'feature'],
};
const riskOrdinal = {
    low: 0,
    medium: 1,
    high: 2,
    critical: 3,
};
const stopWords = new Set([
    'and',
    'are',
    'but',
    'for',
    'from',
    'has',
    'have',
    'into',
    'need',
    'needs',
    'not',
    'our',
    'that',
    'the',
    'their',
    'this',
    'through',
    'with',
    'without',
]);
export function loadLensCatalog(value = catalogJson) {
    return parseLensCatalog(normalizeCatalogInput(value));
}
export function parseLensCatalog(value) {
    if (!isRecord(value) || value.schemaVersion !== 1)
        throw new TypeError('Lens catalog must declare schema version 1');
    if (!Array.isArray(value.lenses))
        throw new TypeError('Lens catalog must include a lenses array');
    if (typeof value.count !== 'number' || value.count !== value.lenses.length) {
        throw new TypeError('Lens catalog count must equal lenses length');
    }
    const lenses = value.lenses.map((lens, index) => parseLensDefinition(lens, index));
    const ids = new Set();
    for (const lens of lenses) {
        if (ids.has(lens.id))
            throw new TypeError(`Lens catalog contains duplicate id: ${lens.id}`);
        ids.add(lens.id);
    }
    for (const lens of lenses)
        assertLensReferencesExist(lens, ids);
    return { count: value.count, lenses, schemaVersion: 1 };
}
export const lensCatalog = loadLensCatalog();
export function recommendLenses(problemProfile = {}, catalog = lensCatalog) {
    const profile = prepareProfile(problemProfile);
    if (!profile.hasDimensions) {
        return {
            lenses: [],
            negotiation: {
                considered: [],
                dropped: [],
                tensionsBalanced: [],
            },
            rationale: ['No profile dimensions were provided; no lenses were recommended.'],
            rounds: 1,
            workerCount: 1,
        };
    }
    const considered = catalog.lenses.map((lens) => scoreLens(lens, profile)).filter((lens) => lens.score > 0);
    const sorted = [...considered].sort(compareScoredLenses);
    const negotiated = negotiateRoster(sorted, profile);
    const lenses = negotiated.selected.map((lens) => lens.ref);
    return {
        lenses,
        negotiation: {
            considered: sorted.map((lens) => lens.ref),
            dropped: negotiated.dropped,
            tensionsBalanced: negotiated.tensionsBalanced,
        },
        rationale: buildRationale(problemProfile, catalog.count, lenses.length, negotiated.cap),
        rounds: deriveRounds(negotiated.selected, profile),
        workerCount: lenses.length === 0 ? 1 : lenses.length,
    };
}
function parseLensDefinition(value, index) {
    if (!isRecord(value))
        throw new TypeError(`Lens at index ${String(index)} must be an object`);
    const whenBeneficial = value.whenBeneficial;
    if (!isRecord(whenBeneficial))
        throw new TypeError(`Lens ${stringField(value, 'id')} must include whenBeneficial`);
    return {
        category: stringField(value, 'category'),
        concerns: stringArrayField(value, 'concerns'),
        conflictsWith: stringArrayField(value, 'conflictsWith'),
        focus: stringField(value, 'focus'),
        id: stringField(value, 'id'),
        name: stringField(value, 'name'),
        pairsWith: stringArrayField(value, 'pairsWith'),
        suggestedTier: tierField(value),
        tension: stringField(value, 'tension'),
        typicalRounds: roundsField(value),
        whenBeneficial: {
            kinds: kindArrayField(whenBeneficial),
            risk: riskArrayField(whenBeneficial),
            signals: stringArrayField(whenBeneficial, 'signals'),
            sizes: sizeArrayField(whenBeneficial),
        },
    };
}
function assertLensReferencesExist(lens, ids) {
    for (const field of ['pairsWith', 'conflictsWith']) {
        for (const reference of lens[field]) {
            if (!ids.has(reference))
                throw new TypeError(`Lens ${lens.id} references unknown lens ${reference}`);
        }
    }
}
function stringField(value, field) {
    const fieldValue = value[field];
    if (typeof fieldValue !== 'string' || fieldValue.length === 0)
        throw new TypeError(`Lens ${field} must be a string`);
    return fieldValue;
}
function stringArrayField(value, field) {
    const fieldValue = value[field];
    if (!Array.isArray(fieldValue))
        throw new TypeError(`Lens ${field} must be an array`);
    return fieldValue.map((item) => {
        if (typeof item !== 'string')
            throw new TypeError(`Lens ${field} must contain only strings`);
        return item;
    });
}
function kindArrayField(value) {
    return stringArrayField(value, 'kinds').map((kind) => {
        if (!isLensKind(kind))
            throw new TypeError(`Lens kind is invalid: ${kind}`);
        return kind;
    });
}
function riskArrayField(value) {
    return stringArrayField(value, 'risk').map((risk) => {
        if (!isLensRisk(risk))
            throw new TypeError(`Lens risk is invalid: ${risk}`);
        return risk;
    });
}
function sizeArrayField(value) {
    return stringArrayField(value, 'sizes').map((size) => {
        if (!isLensSize(size))
            throw new TypeError(`Lens size is invalid: ${size}`);
        return size;
    });
}
function tierField(value) {
    const tier = stringField(value, 'suggestedTier');
    if (!isLensTier(tier))
        throw new TypeError(`Lens tier is invalid: ${tier}`);
    return tier;
}
function roundsField(value) {
    const rounds = value.typicalRounds;
    if (!isTypicalRounds(rounds))
        throw new TypeError('Lens typicalRounds must be 1, 2, or 3');
    return rounds;
}
function normalizeCatalogInput(value) {
    if (!isRecord(value) || !Array.isArray(value.lenses))
        return value;
    const rawLenses = value.lenses;
    const ids = new Set(rawLenses.flatMap((lens) => (isRecord(lens) && typeof lens.id === 'string' ? [lens.id] : [])));
    return {
        ...value,
        lenses: rawLenses.map((lens) => {
            if (!isRecord(lens))
                return lens;
            const whenBeneficial = isRecord(lens.whenBeneficial) ? lens.whenBeneficial : {};
            return {
                ...lens,
                conflictsWith: normalizedReferences(lens.conflictsWith, ids),
                pairsWith: normalizedReferences(lens.pairsWith, ids),
                tension: typeof lens.tension === 'string' && lens.tension.length > 0 ? lens.tension : 'No catalog tension provided.',
                typicalRounds: isTypicalRounds(lens.typicalRounds) ? lens.typicalRounds : 1,
                whenBeneficial: {
                    kinds: normalizedKnownArray(whenBeneficial.kinds, isLensKind),
                    risk: normalizedKnownArray(whenBeneficial.risk, isLensRisk),
                    signals: normalizedStringArray(whenBeneficial.signals),
                    sizes: normalizedKnownArray(whenBeneficial.sizes, isLensSize),
                },
            };
        }),
    };
}
function normalizedReferences(value, ids) {
    return normalizedStringArray(value).filter((reference) => ids.has(reference));
}
function normalizedKnownArray(value, isKnown) {
    return normalizedStringArray(value).filter(isKnown);
}
function normalizedStringArray(value) {
    return Array.isArray(value) ? value.filter((item) => typeof item === 'string') : [];
}
function prepareProfile(problemProfile) {
    const size = problemProfile.size === 'trivial' ? 'small' : problemProfile.size;
    const directKind = isLensKind(problemProfile.kind) ? problemProfile.kind : undefined;
    const aliasKinds = directKind === undefined && problemProfile.kind !== undefined ? kindAliases[problemProfile.kind] ?? [] : [];
    const signals = problemProfile.signals?.filter((signal) => signal.trim().length > 0) ?? [];
    const hasDimensions = size !== undefined ||
        directKind !== undefined ||
        aliasKinds.length > 0 ||
        problemProfile.risk !== undefined ||
        problemProfile.landscape !== undefined ||
        problemProfile.parallelism !== undefined ||
        signals.length > 0;
    return {
        aliasKinds,
        hasDimensions,
        signals,
        ...(directKind === undefined ? {} : { directKind }),
        ...(problemProfile.landscape === undefined ? {} : { landscape: problemProfile.landscape }),
        ...(problemProfile.parallelism === undefined ? {} : { parallelism: problemProfile.parallelism }),
        ...(problemProfile.risk === undefined ? {} : { risk: problemProfile.risk }),
        ...(size === undefined ? {} : { size }),
    };
}
function scoreLens(definition, profile) {
    let score = 0;
    let dimensionMatchCount = 0;
    if (profile.size !== undefined && definition.whenBeneficial.sizes.includes(profile.size)) {
        score += 30;
        dimensionMatchCount += 1;
    }
    if (profile.directKind !== undefined && definition.whenBeneficial.kinds.includes(profile.directKind)) {
        score += 24;
        dimensionMatchCount += 1;
    }
    else if (profile.aliasKinds.some((kind) => definition.whenBeneficial.kinds.includes(kind))) {
        score += 16;
        dimensionMatchCount += 1;
    }
    if (profile.risk !== undefined) {
        const riskScore = riskMatchScore(profile.risk, definition.whenBeneficial.risk);
        if (riskScore > 0)
            dimensionMatchCount += 1;
        score += riskScore;
    }
    const signalMatch = signalMatchScore(profile.signals, definition.whenBeneficial.signals);
    if (signalMatch.score > 0)
        dimensionMatchCount += 1;
    score += signalMatch.score;
    const landscapeScore = landscapeMatchScore(profile, definition);
    if (landscapeScore > 0)
        dimensionMatchCount += 1;
    score += landscapeScore;
    if (profile.parallelism === 'high' && hasAnyKind(definition, ['program', 'infra'])) {
        score += 4;
        dimensionMatchCount += 1;
    }
    return {
        definition,
        dimensionMatchCount,
        hasSignalMatch: signalMatch.matchCount > 0,
        ref: toLensRef(definition, score),
        score,
        signalMatchCount: signalMatch.matchCount,
    };
}
function riskMatchScore(risk, lensRisksForScore) {
    if (lensRisksForScore.includes(risk))
        return 20;
    return lensRisksForScore.some((lensRisk) => Math.abs(riskOrdinal[lensRisk] - riskOrdinal[risk]) === 1) ? 8 : 0;
}
function signalMatchScore(profileSignals, lensSignals) {
    const lensPhrases = new Set(lensSignals.map(normalizePhrase));
    const lensTokens = new Set(lensSignals.flatMap(tokenizeSignal));
    let exactMatches = 0;
    let tokenMatches = 0;
    for (const signal of profileSignals) {
        const phrase = normalizePhrase(signal);
        if (lensPhrases.has(phrase)) {
            exactMatches += 1;
        }
        else if (tokenizeSignal(signal).some((token) => lensTokens.has(token))) {
            tokenMatches += 1;
        }
    }
    const cappedExactMatches = Math.min(exactMatches, 2);
    const cappedTokenMatches = Math.min(tokenMatches, 2);
    return {
        matchCount: cappedExactMatches + cappedTokenMatches,
        score: cappedExactMatches * 14 + cappedTokenMatches * 8,
    };
}
function landscapeMatchScore(profile, lens) {
    if (profile.landscape === 'brownfield' && hasAnyKind(lens, ['migration', 'refactor']))
        return 4;
    if (profile.landscape === 'greenfield' && hasAnyKind(lens, ['architecture', 'feature', 'api']))
        return 4;
    return 0;
}
function negotiateRoster(sortedCandidates, profile) {
    const cap = rosterCap(profile);
    const categoryLimit = cap <= 3 ? 1 : 2;
    const droppedById = new Map();
    const tensionsBalanced = [];
    const selected = [];
    let remaining = [...sortedCandidates];
    while (selected.length < cap && remaining.length > 0) {
        const negotiated = remaining.map((candidate) => negotiateCandidate(candidate, selected)).sort(compareNegotiatedLenses);
        const next = firstSelectable(negotiated, selected, categoryLimit, profile, droppedById, tensionsBalanced);
        if (next === undefined)
            break;
        selected.push(next.scored);
        tensionsBalanced.push(...selectedTensions(next));
        remaining = remaining.filter((candidate) => candidate.definition.id !== next.scored.definition.id);
    }
    return {
        cap,
        dropped: [...droppedById.values()],
        selected,
        tensionsBalanced,
    };
}
function firstSelectable(negotiated, selected, categoryLimit, profile, droppedById, tensionsBalanced) {
    for (const candidate of negotiated) {
        const rejection = rejectionFor(candidate.scored, selected, categoryLimit, profile, tensionsBalanced);
        if (rejection === undefined)
            return candidate;
        droppedById.set(candidate.scored.definition.id, rejection);
    }
    return undefined;
}
function rejectionFor(candidate, selected, categoryLimit, profile, tensionsBalanced) {
    const selectedInCategory = selected.filter((lens) => lens.definition.category === candidate.definition.category);
    if (selectedInCategory.length >= categoryLimit) {
        return {
            id: candidate.definition.id,
            name: candidate.definition.name,
            reason: `Dropped because category ${candidate.definition.category} reached its limit of ${String(categoryLimit)}.`,
            score: candidate.score,
        };
    }
    const conflicts = selected.filter((lens) => lensesConflict(candidate.definition, lens.definition));
    const blockingConflict = conflicts.find((lens) => !conflictOverrideApplies(candidate, lens, profile));
    if (blockingConflict !== undefined) {
        tensionsBalanced.push({
            lensIds: [blockingConflict.definition.id, candidate.definition.id],
            reason: `Dropped ${candidate.definition.id} because it conflicts with ${blockingConflict.definition.id}.`,
        });
        return {
            blockedBy: blockingConflict.definition.id,
            id: candidate.definition.id,
            name: candidate.definition.name,
            reason: `Conflicts with selected lens ${blockingConflict.definition.id}.`,
            score: candidate.score,
        };
    }
    for (const conflict of conflicts) {
        tensionsBalanced.push({
            lensIds: [conflict.definition.id, candidate.definition.id],
            reason: `Critical-risk override kept ${candidate.definition.id} despite conflict with ${conflict.definition.id}.`,
        });
    }
    return undefined;
}
function conflictOverrideApplies(candidate, selected, profile) {
    return profile.risk === 'critical' && candidate.hasSignalMatch && candidate.score >= selected.score + 15;
}
function negotiateCandidate(candidate, selected) {
    const pairBoosts = selected
        .filter((lens) => lensesPair(candidate.definition, lens.definition))
        .map((lens) => lens.definition.id);
    const selectedInSameCategory = selected.filter((lens) => lens.definition.category === candidate.definition.category);
    const categoryBalanced = selectedInSameCategory.length === 0;
    return {
        categoryBalanced,
        negotiatedScore: candidate.score + pairBoosts.length * 8 + (categoryBalanced ? 4 : 0) - selectedInSameCategory.length * 6,
        pairBoosts,
        scored: candidate,
    };
}
function selectedTensions(selectedLens) {
    const balances = [];
    for (const pairId of selectedLens.pairBoosts) {
        balances.push({
            lensIds: [pairId, selectedLens.scored.definition.id],
            reason: `Pair boost balanced complementary lenses ${pairId} and ${selectedLens.scored.definition.id}.`,
        });
    }
    if (selectedLens.categoryBalanced) {
        balances.push({
            lensIds: [selectedLens.scored.definition.id],
            reason: `Category spread added ${selectedLens.scored.definition.category}.`,
        });
    }
    return balances;
}
function rosterCap(profile) {
    const baseCaps = {
        large: 7,
        medium: 5,
        program: 9,
        small: 3,
    };
    const size = profile.size ?? 'medium';
    const riskBoost = profile.risk === 'critical' ? 1 : 0;
    const parallelismBoost = profile.parallelism === 'high' ? 1 : 0;
    return Math.min(baseCaps[size] + riskBoost + parallelismBoost, 10);
}
function deriveRounds(selected, profile) {
    if (selected.length === 0)
        return 1;
    const maxLensRounds = Math.max(...selected.map((lens) => lens.definition.typicalRounds));
    const needsTwoRounds = profile.risk === 'critical' || profile.size === 'program' || profile.parallelism === 'high' || selected.length >= 6;
    const needsThreeRounds = profile.size === 'program' && profile.risk === 'critical' && selected.length >= 8;
    return Math.min(Math.max(maxLensRounds, needsThreeRounds ? 3 : needsTwoRounds ? 2 : 1), 3);
}
function buildRationale(profile, catalogCount, selectedCount, cap) {
    return [
        `Scored ${String(catalogCount)} lenses against ${profileSummary(profile)}.`,
        `Selected ${String(selectedCount)} lenses after negotiation with roster cap ${String(cap)}.`,
        'Worker count follows the selected lens count; rounds derive from lens depth, risk, scale, and parallelism.',
    ];
}
function profileSummary(profile) {
    const parts = [
        profile.size === undefined ? undefined : `size=${profile.size}`,
        profile.kind === undefined ? undefined : `kind=${profile.kind}`,
        profile.risk === undefined ? undefined : `risk=${profile.risk}`,
        profile.landscape === undefined ? undefined : `landscape=${profile.landscape}`,
        profile.parallelism === undefined ? undefined : `parallelism=${profile.parallelism}`,
        profile.signals === undefined ? undefined : `signals=${String(profile.signals.length)}`,
    ].filter((part) => part !== undefined);
    return parts.join(', ');
}
function compareNegotiatedLenses(left, right) {
    return (right.negotiatedScore - left.negotiatedScore ||
        compareScoredLenses(left.scored, right.scored));
}
function compareScoredLenses(left, right) {
    return (right.score - left.score ||
        right.signalMatchCount - left.signalMatchCount ||
        right.dimensionMatchCount - left.dimensionMatchCount ||
        left.definition.category.localeCompare(right.definition.category) ||
        left.definition.id.localeCompare(right.definition.id));
}
function toLensRef(definition, score) {
    return {
        category: definition.category,
        focus: definition.focus,
        id: definition.id,
        name: definition.name,
        score,
        suggestedTier: definition.suggestedTier,
    };
}
function lensesPair(left, right) {
    return left.pairsWith.includes(right.id) || right.pairsWith.includes(left.id);
}
function lensesConflict(left, right) {
    return left.conflictsWith.includes(right.id) || right.conflictsWith.includes(left.id);
}
function hasAnyKind(lens, kinds) {
    return kinds.some((kind) => lens.whenBeneficial.kinds.includes(kind));
}
function normalizePhrase(value) {
    return value.trim().toLowerCase().replace(/\s+/gu, ' ');
}
function tokenizeSignal(value) {
    return normalizePhrase(value)
        .split(/[^a-z0-9]+/u)
        .filter((token) => token.length >= 3 && !stopWords.has(token));
}
function isLensKind(value) {
    return typeof value === 'string' && lensKinds.includes(value);
}
function isLensRisk(value) {
    return typeof value === 'string' && lensRisks.includes(value);
}
function isLensSize(value) {
    return typeof value === 'string' && lensSizes.includes(value);
}
function isLensTier(value) {
    return typeof value === 'string' && lensTiers.includes(value);
}
function isTypicalRounds(value) {
    return typeof value === 'number' && typicalRounds.includes(value);
}
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
