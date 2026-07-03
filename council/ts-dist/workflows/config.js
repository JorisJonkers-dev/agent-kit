import { coerceConfigValue, CONFIG_KEYS, parseCouncilConfig, resolveCouncilConfig, writeCouncilConfig, } from '../contexts/config/index.js';
export async function configWorkflow(input, deps) {
    if (input.action === 'path') {
        return { paths: input.paths };
    }
    const user = await readOptionalConfig(input.paths.user, deps.readText);
    const project = await readOptionalConfig(input.paths.project, deps.readText);
    const target = input.project ? input.paths.project : input.paths.user;
    const current = input.project ? project : user;
    if (input.action === 'show') {
        return {
            config: current,
            paths: input.paths,
            resolved: resolveCouncilConfig({ project, user }),
            target,
        };
    }
    if (input.action === 'get') {
        const key = requireConfigKey(input.key);
        const resolved = resolveCouncilConfig({ project, user });
        return { key, paths: input.paths, resolved, value: resolved[key], target };
    }
    if (input.action === 'set') {
        const key = requireConfigKey(input.key);
        if (input.value === undefined)
            throw new Error('config set requires <key> <value>');
        const next = { ...current, [key]: coerceConfigValue(key, input.value) };
        await writeConfig(target, next, deps, undefined);
        return { config: next, key, paths: input.paths, target, value: next[key] };
    }
    const key = requireConfigKey(input.key);
    const next = omitKey(current, key);
    await writeConfig(target, next, deps, key);
    return { config: next, key, paths: input.paths, target };
}
async function readOptionalConfig(path, readText) {
    try {
        return parseCouncilConfig(await readText(path));
    }
    catch (error) {
        if (isErrno(error, 'ENOENT'))
            return {};
        throw error;
    }
}
async function writeConfig(path, next, deps, unsetKey) {
    let source = '';
    try {
        source = await deps.readText(path);
    }
    catch (error) {
        if (!isErrno(error, 'ENOENT'))
            throw error;
    }
    const writableSource = unsetKey === undefined ? source : removeRootAssignment(source, unsetKey);
    await deps.writeText(path, writeCouncilConfig(writableSource, next));
}
function requireConfigKey(key) {
    if (key === undefined)
        throw new Error('config action requires a key');
    if (!CONFIG_KEYS.includes(key)) {
        throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(', ')}`);
    }
    return key;
}
function omitKey(object, key) {
    return Object.fromEntries(Object.entries(object).filter(([k]) => k !== key));
}
function removeRootAssignment(source, key) {
    const lines = source.replace(/\r\n/gu, '\n').split('\n');
    let inTable = false;
    const kept = lines.filter((line) => {
        if (/^\s*\[/.test(line))
            inTable = true;
        return inTable || !new RegExp(`^\\s*${escapeRegExp(key)}\\s*=`).test(line);
    });
    return kept.join('\n');
}
function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
}
function isErrno(error, code) {
    return error instanceof Error && 'code' in error && error.code === code;
}
