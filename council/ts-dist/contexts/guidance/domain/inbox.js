export function emptyGuidanceState() {
    return {
        log: [],
        inbox: [],
        next_sequence: 1,
    };
}
export function appendGuidance(state, input) {
    if (state.log.some((entry) => entry.id === input.id)) {
        throw new Error(`guidance id already exists: ${input.id}`);
    }
    const entry = {
        ...input,
        sequence: state.next_sequence,
        task_sequence: nextTaskSequence(state.log, input.task_id),
    };
    const inboxItem = {
        guidance_id: entry.id,
        task_id: entry.task_id,
        sequence: entry.sequence,
        status: 'pending',
    };
    return {
        state: {
            log: [...state.log, entry],
            inbox: [...state.inbox, inboxItem],
            next_sequence: state.next_sequence + 1,
        },
        entry,
        inbox_item: inboxItem,
    };
}
export function guidanceForTask(state, taskId) {
    return state.log
        .filter((entry) => entry.task_id === taskId)
        .toSorted((left, right) => left.task_sequence - right.task_sequence);
}
export function pendingInbox(state) {
    return state.inbox
        .filter((item) => item.status === 'pending')
        .toSorted((left, right) => left.sequence - right.sequence);
}
export function updateInboxStatus(state, guidanceId, status) {
    if (!state.inbox.some((item) => item.guidance_id === guidanceId)) {
        throw new Error(`guidance inbox item does not exist: ${guidanceId}`);
    }
    return {
        ...state,
        inbox: state.inbox.map((item) => item.guidance_id === guidanceId
            ? {
                ...item,
                status,
            }
            : item),
    };
}
function nextTaskSequence(log, taskId) {
    const previousTaskEntries = log.filter((entry) => entry.task_id === taskId);
    if (previousTaskEntries.length === 0) {
        return 1;
    }
    return Math.max(...previousTaskEntries.map((entry) => entry.task_sequence)) + 1;
}
