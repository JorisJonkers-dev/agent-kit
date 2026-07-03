export function createStallDetectorState(nowMs, logBytes = 0) {
    return { logBytes, lastGrowthAtMs: nowMs };
}
export function evaluateStall(state, input) {
    if (input.logBytes > state.logBytes) {
        return {
            state: { logBytes: input.logBytes, lastGrowthAtMs: input.nowMs },
            detection: null,
        };
    }
    const idleMs = input.nowMs - state.lastGrowthAtMs;
    const stalled = idleMs >= input.stallAfterS * 1000;
    return {
        state,
        detection: stalled ? { kind: 'stall', idleMs, logBytes: input.logBytes } : null,
    };
}
export function createLoopDetectorState() {
    return { actions: [] };
}
export function appendLoopLine(state, line, config) {
    const actions = extractActionLines(line);
    if (actions.length === 0) {
        return { state, detection: null };
    }
    const nextActions = [...state.actions, ...actions].slice(-config.windowSize);
    const nextState = { actions: nextActions };
    return {
        state: nextState,
        detection: classifyLoopDetection(detectLoop(nextActions, config)).detection,
    };
}
export function extractActionLines(line) {
    const trimmed = line.trim();
    if (trimmed.length === 0) {
        return [];
    }
    const jsonActions = extractJsonActionLines(trimmed);
    if (jsonActions.length > 0) {
        return jsonActions;
    }
    const codexCommand = extractCodexCommand(trimmed);
    return codexCommand === null ? [] : [toActionLine(codexCommand)];
}
export function evaluateDiskUsageCap(input) {
    return input.duBytes > input.capBytes
        ? { kind: 'disk-cap', duBytes: input.duBytes, capBytes: input.capBytes }
        : null;
}
export function createEscalationState() {
    return runningWatchdogState.toEscalationState();
}
export function advanceEscalation(state, config) {
    const transition = watchdogStates[state.phase].advance(tierEscalationPolicy(config));
    return {
        state: transition.state.toEscalationState(),
        action: transition.action,
    };
}
class RunningWatchdogState {
    phase = 'ready';
    status = 'running';
    advance() {
        return transition(terminatedRestartingWatchdogState, 'terminate');
    }
    toEscalationState() {
        return { phase: this.phase };
    }
}
class TerminatedRestartingWatchdogState {
    phase = 'terminated';
    status = 'restarting';
    advance() {
        return transition(preambleRestartingWatchdogState, 'retry-with-preamble');
    }
    toEscalationState() {
        return { phase: this.phase };
    }
}
class PreambleRestartingWatchdogState {
    phase = 'retry-with-preamble';
    status = 'restarting';
    advance(policy) {
        return policy.afterRetryWithPreamble();
    }
    toEscalationState() {
        return { phase: this.phase };
    }
}
class EscalatedWatchdogState {
    phase = 'tier-escalated';
    status = 'escalated';
    advance() {
        return transition(stalledWatchdogState, 'stalled');
    }
    toEscalationState() {
        return { phase: this.phase };
    }
}
class StalledWatchdogState {
    phase = 'stalled';
    status = 'stalled';
    advance() {
        return transition(stalledWatchdogState, 'stalled');
    }
    toEscalationState() {
        return { phase: this.phase };
    }
}
class EnabledTierEscalationPolicy {
    afterRetryWithPreamble() {
        return transition(escalatedWatchdogState, 'escalate-tier');
    }
}
class DisabledTierEscalationPolicy {
    afterRetryWithPreamble() {
        return transition(stalledWatchdogState, 'stalled');
    }
}
class RunningLoopEvaluationState {
    status = 'running';
    detection = null;
}
class LoopingWatchdogState {
    detection;
    status = 'looping';
    constructor(detection) {
        this.detection = detection;
    }
}
class RepeatLoopDetectionPolicy {
    detect(actions, config) {
        const counts = new Map();
        for (const action of actions) {
            const count = (counts.get(action.normalized) ?? 0) + 1;
            if (count >= config.repeatLimit) {
                return { kind: 'loop-repeat', normalized: action.normalized, count };
            }
            counts.set(action.normalized, count);
        }
        return null;
    }
}
class CycleLoopDetectionPolicy {
    detect(actions, config) {
        const maxGram = Math.min(config.maxCycleGram ?? 5, 5, Math.floor(actions.length / 3));
        for (let gramSize = 1; gramSize <= maxGram; gramSize += 1) {
            const offset = actions.length - gramSize;
            const sequence = actions.slice(offset).map((action) => action.verbatim);
            if (matchesCycle(actions, sequence, gramSize)) {
                return { kind: 'loop-cycle', gramSize, sequence };
            }
        }
        return null;
    }
}
const runningWatchdogState = new RunningWatchdogState();
const terminatedRestartingWatchdogState = new TerminatedRestartingWatchdogState();
const preambleRestartingWatchdogState = new PreambleRestartingWatchdogState();
const escalatedWatchdogState = new EscalatedWatchdogState();
const stalledWatchdogState = new StalledWatchdogState();
const enabledTierEscalationPolicy = new EnabledTierEscalationPolicy();
const disabledTierEscalationPolicy = new DisabledTierEscalationPolicy();
const runningLoopEvaluationState = new RunningLoopEvaluationState();
const watchdogStates = {
    ready: runningWatchdogState,
    terminated: terminatedRestartingWatchdogState,
    'retry-with-preamble': preambleRestartingWatchdogState,
    'tier-escalated': escalatedWatchdogState,
    stalled: stalledWatchdogState,
};
const loopDetectionPolicies = [
    new RepeatLoopDetectionPolicy(),
    new CycleLoopDetectionPolicy(),
];
function transition(state, action) {
    return { state, action };
}
function tierEscalationPolicy(config) {
    return config.enableTierEscalation ? enabledTierEscalationPolicy : disabledTierEscalationPolicy;
}
function classifyLoopDetection(detection) {
    return detection === null ? runningLoopEvaluationState : new LoopingWatchdogState(detection);
}
function detectLoop(actions, config) {
    for (const policy of loopDetectionPolicies) {
        const detection = policy.detect(actions, config);
        if (detection !== null) {
            return detection;
        }
    }
    return null;
}
function matchesCycle(actions, sequence, gramSize) {
    for (let repeat = 2; repeat <= 3; repeat += 1) {
        const offset = actions.length - repeat * gramSize;
        for (let index = 0; index < gramSize; index += 1) {
            if (actions[offset + index]?.verbatim !== sequence[index]) {
                return false;
            }
        }
    }
    return true;
}
function extractJsonActionLines(line) {
    try {
        const parsed = JSON.parse(line);
        return findToolUses(parsed);
    }
    catch {
        return [];
    }
}
function findToolUses(value) {
    if (Array.isArray(value)) {
        return value.flatMap((item) => findToolUses(item));
    }
    if (!isRecord(value)) {
        return [];
    }
    const ownToolUse = value.type === 'tool_use' ? [toolUseToActionLine(value)] : [];
    const nestedToolUses = Object.values(value).flatMap((item) => findToolUses(item));
    return [...ownToolUse, ...nestedToolUses];
}
function toolUseToActionLine(toolUse) {
    const name = typeof toolUse.name === 'string' ? toolUse.name : 'unknown';
    const input = isRecord(toolUse.input) ? toolUse.input : {};
    return toActionLine(`tool:${name}:${stableStringify(input)}`);
}
function extractCodexCommand(line) {
    const match = /^(?:[$>]|exec(?:_command)?[: ]|command[: ])\s*(?<command>.+)$/iu.exec(line);
    return match?.groups?.command?.trim() ?? null;
}
function toActionLine(verbatim) {
    return {
        verbatim,
        normalized: normalizeActionLine(verbatim),
    };
}
function normalizeActionLine(line) {
    return line
        .trim()
        .toLowerCase()
        .replace(/[0-9a-f]{8,}/giu, '<hex>')
        .replace(/\d+/gu, '<n>')
        .replace(/\s+/gu, ' ');
}
function stableStringify(value) {
    if (Array.isArray(value)) {
        return `[${value.map((item) => stableStringify(item)).join(',')}]`;
    }
    if (isRecord(value)) {
        return `{${Object.keys(value)
            .sort()
            .map((key) => `${JSON.stringify(key)}:${stableStringify(value[key])}`)
            .join(',')}}`;
    }
    const serialized = JSON.stringify(value);
    return typeof serialized === 'string' ? serialized : 'undefined';
}
function isRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
