export function requireString(value, key) {
    const parsed = optionalString(value);
    if (parsed === undefined) {
        throw new Error(`${key} must be a string`);
    }
    return parsed;
}
export function requireNumber(value, key) {
    const parsed = optionalNumber(value);
    if (parsed === undefined) {
        throw new Error(`${key} must be a number`);
    }
    return parsed;
}
export function optionalString(value) {
    return typeof value === 'string' ? value : undefined;
}
export function optionalNumber(value) {
    return typeof value === 'number' ? value : undefined;
}
export function optionalBoolean(value) {
    return typeof value === 'boolean' ? value : undefined;
}
export function optionalStringArray(value) {
    return Array.isArray(value) && value.every((item) => typeof item === 'string') ? value : undefined;
}
export function optionalObject(value, map) {
    const table = asTomlTable(value);
    if (!table) {
        return undefined;
    }
    const mapped = map(table);
    return Object.keys(mapped).length > 0 ? mapped : undefined;
}
export function optionalStringRecord(value) {
    const table = asTomlTable(value);
    if (!table) {
        return undefined;
    }
    const entries = Object.entries(table).filter((entry) => typeof entry[1] === 'string');
    return entries.length > 0 ? Object.fromEntries(entries) : undefined;
}
export function optionalStringRecordTable(value, map) {
    const table = asTomlTable(value);
    if (!table) {
        return undefined;
    }
    const entries = Object.entries(table)
        .map(([key, entry]) => [key, latestTable(entry)])
        .filter((entry) => entry[1] !== undefined)
        .map(([key, table]) => [key, map(table)])
        .filter((entry) => Object.keys(entry[1]).length > 0);
    return entries.length > 0 ? Object.fromEntries(entries) : undefined;
}
export function asTomlTable(value) {
    return isTomlTable(value) ? value : undefined;
}
export function isTomlTable(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
export function omitUndefined(value) {
    return Object.fromEntries(Object.entries(value).filter((entry) => entry[1] !== undefined));
}
function latestTable(value) {
    const table = asTomlTable(value);
    if (table) {
        return table;
    }
    if (Array.isArray(value)) {
        return value
            .map((item) => asTomlTable(item))
            .filter((item) => item !== undefined)
            .at(-1);
    }
    return undefined;
}
