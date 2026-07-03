export function buildAdversarialCritiqueRounds(participants, options, roundCount) {
    assertPositiveInteger(roundCount, 'roundCount');
    if (participants.length < 2) {
        throw new Error('at least two participants are required');
    }
    return Array.from({ length: roundCount }, (_, index) => {
        const round = index + 1;
        return {
            round,
            assignments: options.flatMap((option) => assignmentsForOption(participants, option, round)),
        };
    });
}
export function resolveDesignVote(decisionId, options, votes) {
    const decisionOptions = options.filter((option) => option.decision_id === decisionId);
    if (decisionOptions.length === 0) {
        throw new Error(`decision has no options: ${decisionId}`);
    }
    const optionIds = new Set(decisionOptions.map((option) => option.id));
    const relevantVotes = votes.filter((vote) => vote.decision_id === decisionId);
    const validVotes = relevantVotes.filter((vote) => optionIds.has(vote.option_id));
    const ignoredVotes = relevantVotes.filter((vote) => !optionIds.has(vote.option_id));
    const counts = countVotes(decisionOptions, validVotes);
    const winner = counts.reduce((best, count) => (best.votes >= count.votes ? best : count));
    const supportCount = winner.votes;
    const totalVotes = relevantVotes.length;
    const reasons = escalationReasons(decisionOptions, validVotes, ignoredVotes, counts, supportCount, validVotes.length, totalVotes);
    const escalates = reasons.length > 0;
    const resolvedOption = decisionOptions.reduce((selected, option) => option.id === winner.option_id ? option : selected);
    const resolution = formatResolution(supportCount, totalVotes);
    const base = {
        decision_id: decisionId,
        resolution,
        support_count: supportCount,
        total_votes: totalVotes,
        counts,
        escalation_reasons: reasons,
        escalate_to_consolidator: escalates,
        ignored_votes: ignoredVotes,
        winning_option_id: winner.option_id,
    };
    if (escalates) {
        return {
            ...base,
            status: 'contested',
        };
    }
    return {
        ...base,
        status: 'settled',
        resolved_option: resolvedOption,
    };
}
export function resolveDesignVotes(options, votes) {
    return [...new Set(options.map((option) => option.decision_id))].map((decisionId) => resolveDesignVote(decisionId, options, votes));
}
export function splitDesignLedger(ledger) {
    const entries = ledger.entries ?? [];
    return {
        settled: { entries: entries.filter((entry) => entry.status === 'settled') },
        contested: { entries: entries.filter((entry) => entry.status !== 'settled') },
    };
}
export function ledgerEntriesFromResolutions(resolutions, options) {
    return resolutions.flatMap((resolution) => entriesForResolution(resolution, options));
}
export function buildLockSpecMergeInputs(split) {
    const settledEntries = split.settled.entries ?? [];
    const contestedEntries = split.contested.entries ?? [];
    return {
        settled_entries: settledEntries,
        contested_entries: contestedEntries,
        consolidator_entry_ids: contestedEntries.map((entry) => entry.id),
        can_lock_without_consolidator: contestedEntries.length === 0,
    };
}
function assignmentsForOption(participants, option, round) {
    return participants.flatMap((reviewer, reviewerIndex) => {
        const subjectId = option.proposed_by ?? rotatingSubjectId(participants, reviewerIndex, round);
        if (subjectId === reviewer.id) {
            return [];
        }
        return [
            {
                round,
                decision_id: option.decision_id,
                option_id: option.id,
                reviewer_id: reviewer.id,
                subject_id: subjectId,
            },
        ];
    });
}
function rotatingSubjectId(participants, reviewerIndex, round) {
    const subjectIndex = (reviewerIndex + round) % participants.length;
    const subject = participants.reduce((selected, participant, index) => index === subjectIndex ? participant : selected);
    return subject.id;
}
function countVotes(options, votes) {
    const initialCounts = new Map(options.map((option) => [option.id, 0]));
    for (const vote of votes) {
        initialCounts.set(vote.option_id, (initialCounts.get(vote.option_id) ?? 0) + 1);
    }
    return options
        .map((option) => ({ option_id: option.id, votes: initialCounts.get(option.id) ?? 0 }))
        .sort((left, right) => right.votes - left.votes);
}
function escalationReasons(options, votes, ignoredVotes, counts, supportCount, validVoteCount, totalVotes) {
    const reasons = [];
    if (options.some((option) => option.flagged === true) || votes.some((vote) => vote.flagged === true)) {
        reasons.push('flagged');
    }
    if (ignoredVotes.length > 0) {
        reasons.push('missing-option');
    }
    if (validVoteCount === 0) {
        reasons.push('no-votes');
    }
    else if (hasTie(counts, supportCount)) {
        reasons.push('tie');
    }
    else if (supportCount < Math.floor(totalVotes / 2) + 1) {
        reasons.push('no-majority');
    }
    return reasons;
}
function hasTie(counts, supportCount) {
    const next = counts[1];
    return next?.votes === supportCount;
}
function formatResolution(supportCount, totalVotes) {
    return `${String(supportCount)}/${String(totalVotes)}`;
}
function entriesForResolution(resolution, options) {
    if (resolution.status === 'settled') {
        const option = resolution.resolved_option;
        if (option === undefined) {
            throw new Error(`settled decision has no resolved option: ${resolution.decision_id}`);
        }
        return [entryFromOption(option, 'settled', resolution.resolution)];
    }
    return options
        .filter((option) => option.decision_id === resolution.decision_id)
        .map((option) => entryFromOption(option, 'contested', resolution.resolution));
}
function entryFromOption(option, status, resolution) {
    return {
        id: option.id,
        decision: option.decision,
        rationale: option.rationale ?? `vote resolved ${resolution}`,
        status,
        ...(option.task_refs === undefined ? {} : { task_refs: option.task_refs }),
        ...(option.context_refs === undefined ? {} : { context_refs: option.context_refs }),
        ...(option.supersedes === undefined ? {} : { supersedes: option.supersedes }),
        ...(option.content_hash === undefined ? {} : { content_hash: option.content_hash }),
    };
}
function assertPositiveInteger(value, name) {
    if (!Number.isInteger(value) || value < 1) {
        throw new Error(`${name} must be a positive integer`);
    }
}
