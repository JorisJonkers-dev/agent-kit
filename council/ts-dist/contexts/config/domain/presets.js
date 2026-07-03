export const DEFAULT_INTENSITY = 'standard';
export const ROLE_KEYS = ['planner_a', 'planner_b', 'consolidator', 'worker', 'verifier'];
export const INT_KEYS = ['rounds', 'max_workers'];
export const CODEX_EFFORTS = ['low', 'medium', 'high', 'xhigh'];
export const CONFIG_KEYS = [
    'intensity',
    ...ROLE_KEYS,
    'codex_effort',
    ...INT_KEYS,
];
export const BASE_ROLES = {
    planner_a: 'claude:opus',
    planner_b: 'codex:gpt-5.5',
    consolidator: 'claude:opus',
    verifier: 'claude:sonnet',
};
export const PRESETS = {
    quick: { rounds: 1, codex_effort: 'low', worker: 'claude:haiku', max_workers: 4 },
    standard: { rounds: 2, codex_effort: 'high', worker: 'claude:haiku', max_workers: 6 },
    thorough: { rounds: 3, codex_effort: 'high', worker: 'claude:sonnet', max_workers: 6 },
    max: { rounds: 3, codex_effort: 'xhigh', worker: 'claude:sonnet', max_workers: 8 },
};
export function requireIntensity(value) {
    const parsed = optionalIntensity(value);
    if (parsed === undefined) {
        throw new Error(`unknown intensity ${String(value)}; choose from ${Object.keys(PRESETS).join(', ')}`);
    }
    return parsed;
}
export function optionalIntensity(value) {
    return typeof value === 'string' && value in PRESETS ? value : undefined;
}
export function requireCodexEffort(value) {
    const parsed = optionalCodexEffort(value);
    if (parsed === undefined) {
        throw new Error(`codex_effort must be one of ${CODEX_EFFORTS.join(', ')}`);
    }
    return parsed;
}
export function optionalCodexEffort(value) {
    return typeof value === 'string' && CODEX_EFFORTS.includes(value)
        ? value
        : undefined;
}
