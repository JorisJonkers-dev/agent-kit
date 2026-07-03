export function stableJsonStringify(value, level = 0) {
    if (value === null) {
        return 'null';
    }
    if (typeof value === 'string') {
        return quoteJsonString(value);
    }
    if (typeof value === 'number' || typeof value === 'boolean') {
        return primitiveJsonString(value);
    }
    if (Array.isArray(value)) {
        if (value.length === 0) {
            return '[]';
        }
        const currentIndent = ' '.repeat(level * 2);
        const nextIndent = ' '.repeat((level + 1) * 2);
        return `[\n${value.map((item) => `${nextIndent}${stableJsonStringify(item, level + 1)}`).join(',\n')}\n${currentIndent}]`;
    }
    if (isJsonRecord(value)) {
        const keys = Object.keys(value).sort();
        if (keys.length === 0) {
            return '{}';
        }
        const currentIndent = ' '.repeat(level * 2);
        const nextIndent = ' '.repeat((level + 1) * 2);
        return `{\n${keys
            .map((key) => `${nextIndent}${quoteJsonString(key)}: ${stableJsonStringify(value[key], level + 1)}`)
            .join(',\n')}\n${currentIndent}}`;
    }
    throw new Error(`cannot serialize non-JSON value: ${describeNonJsonValue(value)}`);
}
export function isJsonRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
export function isJsonArray(value) {
    return Array.isArray(value);
}
export function isStringArray(value) {
    return isJsonArray(value) && value.every((item) => typeof item === 'string');
}
export function comparableKey(value) {
    return stableJsonStringify(value);
}
export function stringifyForDisplay(value) {
    if (typeof value === 'string') {
        return value;
    }
    if (typeof value === 'number' || typeof value === 'boolean' || value === null) {
        return primitiveJsonString(value);
    }
    if (value === undefined) {
        return '';
    }
    return stableJsonStringify(value);
}
export function formatPythonList(values) {
    return `[${values.map((value) => pythonRepr(value)).join(', ')}]`;
}
export function pythonRepr(value) {
    return `'${value.replaceAll('\\', '\\\\').replaceAll("'", "\\'")}'`;
}
export function jsonErrorMessage(error) {
    return error instanceof Error ? error.message : String(error);
}
function quoteJsonString(value) {
    const quoted = JSON.stringify(value);
    let escaped = '';
    for (let index = 0; index < quoted.length; index += 1) {
        const code = quoted.charCodeAt(index);
        escaped += code > 127 ? `\\u${code.toString(16).padStart(4, '0')}` : quoted.charAt(index);
    }
    return escaped;
}
function primitiveJsonString(value) {
    if (typeof value === 'boolean') {
        return value ? 'true' : 'false';
    }
    if (value === null) {
        return 'null';
    }
    return Number.isFinite(value) ? String(Object.is(value, -0) ? 0 : value) : 'null';
}
function describeNonJsonValue(value) {
    return value === undefined ? 'undefined' : typeof value;
}
