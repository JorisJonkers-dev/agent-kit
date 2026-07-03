import { requireNumber, requireString } from './config-value-objects.js';
import { BASE_ROLES, CONFIG_KEYS, DEFAULT_INTENSITY, PRESETS, requireCodexEffort, requireIntensity, } from './presets.js';
const ENV_DEFAULTS = {
    COUNCIL_CODEX_REASONING: 'high',
    COUNCIL_PLAN_TIMEOUT_S: 1200,
    COUNCIL_WORKER_TIMEOUT_S: 1800,
    COUNCIL_VERIFY_TIMEOUT_S: 600,
};
export function resolveCouncilConfig(input = {}) {
    const mergedBeforeFlags = mergeCouncilConfigs(input.user, input.project);
    const intensity = resolveIntensity(input.preset, mergedBeforeFlags, input.flags);
    const preset = PRESETS[intensity];
    const resolved = mergeCouncilConfigs({ ...BASE_ROLES, ...preset }, input.user, input.project, input.flags, { intensity });
    return {
        ...resolved,
        intensity,
        planner_a: requireString(resolved.planner_a, 'planner_a'),
        planner_b: requireString(resolved.planner_b, 'planner_b'),
        consolidator: requireString(resolved.consolidator, 'consolidator'),
        worker: requireString(resolved.worker, 'worker'),
        verifier: requireString(resolved.verifier, 'verifier'),
        codex_effort: requireCodexEffort(resolved.codex_effort),
        rounds: requireNumber(resolved.rounds, 'rounds'),
        max_workers: requireNumber(resolved.max_workers, 'max_workers'),
        runtime: resolveRuntime(input.env, resolved.codex_effort),
    };
}
export function coerceConfigValue(key, raw) {
    if (!isConfigKey(key)) {
        throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(', ')}`);
    }
    switch (key) {
        case 'intensity':
            return requireIntensity(raw);
        case 'rounds':
        case 'max_workers': {
            const parsed = Number.parseInt(raw, 10);
            if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
                throw new Error(`${key} must be an integer, got ${raw}`);
            }
            return parsed;
        }
        case 'codex_effort':
            return requireCodexEffort(raw);
        case 'planner_a':
        case 'planner_b':
        case 'consolidator':
        case 'worker':
        case 'verifier':
            if (!/^(claude|codex):.+/.test(raw)) {
                throw new Error(`${key} must be claude:<model> or codex:<model>, got ${raw}`);
            }
            return raw;
    }
}
function mergeCouncilConfigs(...configs) {
    return configs.reduce((merged, config) => deepMerge(merged, config), {});
}
function deepMerge(left, right) {
    if (!right) {
        return left;
    }
    const merged = { ...left };
    Object.entries(right).forEach(([key, value]) => {
        if (value === undefined) {
            return;
        }
        const existing = merged[key];
        merged[key] = isPlainObject(existing) && isPlainObject(value) ? deepMerge(existing, value) : value;
    });
    return merged;
}
function resolveIntensity(preset, config, flags) {
    return requireIntensity(flags?.intensity ?? config.intensity ?? preset ?? DEFAULT_INTENSITY);
}
function resolveRuntime(env, codexEffort) {
    const codexReasoning = env?.get('COUNCIL_CODEX_REASONING') ?? codexEffort ?? ENV_DEFAULTS.COUNCIL_CODEX_REASONING;
    return {
        codex_reasoning: codexReasoning,
        plan_timeout_s: envInt(env, 'COUNCIL_PLAN_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_PLAN_TIMEOUT_S),
        worker_timeout_s: envInt(env, 'COUNCIL_WORKER_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_WORKER_TIMEOUT_S),
        verify_timeout_s: envInt(env, 'COUNCIL_VERIFY_TIMEOUT_S', ENV_DEFAULTS.COUNCIL_VERIFY_TIMEOUT_S),
    };
}
function envInt(env, name, fallback) {
    const raw = env?.get(name);
    if (raw === undefined) {
        return fallback;
    }
    const parsed = Number.parseInt(raw, 10);
    if (!Number.isInteger(parsed) || String(parsed) !== raw.trim()) {
        throw new Error(`${name} must be an integer, got ${raw}`);
    }
    return parsed;
}
function isPlainObject(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
function isConfigKey(key) {
    return CONFIG_KEYS.includes(key);
}
