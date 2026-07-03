import { optional } from './types.js';
export function joinPrompt(...parts) {
    const joined = parts
        .filter((part) => part !== undefined && part.length > 0)
        .join('\n\n');
    return joined.length === 0 ? undefined : joined;
}
export function normalizeWatchdogConfig(config = {}) {
    return {
        ...optional('diskCapBytes', config.diskCapBytes),
        enableTierEscalation: config.enableTierEscalation ?? true,
        loop: {
            ...optional('maxCycleGram', config.maxCycleGram),
            repeatLimit: config.repeatLimit ?? 3,
            windowSize: config.windowSize ?? 20,
        },
        maxRestarts: config.maxRestarts ?? 2,
        stallAfterS: config.stallAfterS ?? 300,
    };
}
export function spawnInput(preamble, modelTier) {
    return {
        preamble,
        ...optional('modelTier', modelTier),
    };
}
export function isPromiseLike(value) {
    return (typeof value === 'object' &&
        value !== null &&
        'then' in value &&
        typeof value.then === 'function');
}
export function thenMaybe(value, next) {
    return isPromiseLike(value) ? value.then(next) : next();
}
