import { isTomlTable } from './config-value-objects.js';
import { findCommentIndex, findMatchingBracket, findTopLevelChar, splitTopLevel } from './toml-syntax.js';
import { normalizeCouncilConfig } from './toml-normalize.js';
export function parseToml(source) {
    const finalNewline = source.endsWith('\n');
    const lines = source.replace(/\r\n/g, '\n').split('\n');
    if (finalNewline) {
        lines.pop();
    }
    const data = {};
    const assignments = [];
    const tables = [];
    let currentPath = [];
    let currentTable = data;
    lines.forEach((line, lineIndex) => {
        const body = stripInlineComment(line).trim();
        if (body === '') {
            return;
        }
        const arrayHeader = /^\[\[(.+)\]\]$/.exec(body);
        const tableHeader = /^\[(.+)\]$/.exec(body);
        if (arrayHeader) {
            currentPath = parseKeyPath(arrayHeader[1] ?? '');
            currentTable = appendArrayTable(data, currentPath);
            tables.push({ lineIndex, path: currentPath, array: true });
            return;
        }
        if (tableHeader) {
            currentPath = parseKeyPath(tableHeader[1] ?? '');
            currentTable = ensureTable(data, currentPath);
            tables.push({ lineIndex, path: currentPath, array: false });
            return;
        }
        const equalIndex = findTopLevelChar(body, '=');
        if (equalIndex < 1) {
            throw new Error(`invalid TOML assignment on line ${String(lineIndex + 1)}`);
        }
        const rawKey = body.slice(0, equalIndex).trim();
        const keyPath = parseKeyPath(rawKey);
        const value = parseTomlValue(body.slice(equalIndex + 1).trim());
        setNestedValue(currentTable, keyPath, value);
        assignments.push({ lineIndex, tablePath: currentPath, keyPath, sourceKey: rawKey });
    });
    return { source, lines, finalNewline, data, assignments, tables };
}
export function parseCouncilConfig(source) {
    return normalizeCouncilConfig(parseToml(source).data);
}
function parseTomlValue(raw) {
    if (raw.startsWith('"')) {
        return parseString(raw);
    }
    if (raw.startsWith('[')) {
        return parseArray(raw);
    }
    if (raw === 'true') {
        return true;
    }
    if (raw === 'false') {
        return false;
    }
    if (/^[+-]?\d+$/.test(raw)) {
        return Number.parseInt(raw, 10);
    }
    throw new Error(`unsupported TOML value ${raw}`);
}
function parseString(raw) {
    const commentIndex = findValueEnd(raw);
    const candidate = raw.slice(0, commentIndex);
    try {
        return JSON.parse(candidate);
    }
    catch {
        throw new Error(`invalid TOML string ${raw}`);
    }
}
function parseArray(raw) {
    const end = findMatchingBracket(raw);
    if (end < 0) {
        throw new Error(`unterminated TOML array ${raw}`);
    }
    const inner = raw.slice(1, end).trim();
    if (inner === '') {
        return [];
    }
    return splitTopLevel(inner, ',').map((part) => parseTomlValue(part.trim()));
}
function parseKeyPath(raw) {
    const parts = splitTopLevel(raw.trim(), '.').map((part) => part.trim());
    if (parts.some((part) => !/^[A-Za-z0-9_-]+$/.test(part))) {
        throw new Error(`unsupported TOML key path ${raw}`);
    }
    return parts;
}
function stripInlineComment(line) {
    const commentIndex = findCommentIndex(line);
    return commentIndex < 0 ? line : line.slice(0, commentIndex);
}
function findValueEnd(raw) {
    const commentIndex = findCommentIndex(raw);
    return commentIndex < 0 ? raw.length : commentIndex;
}
function setNestedValue(table, keyPath, value) {
    const head = keyPath[0] ?? '';
    const tail = keyPath.slice(1);
    if (tail.length === 0) {
        table[head] = value;
        return;
    }
    const child = table[head];
    if (!isTomlTable(child)) {
        table[head] = {};
    }
    setNestedValue(table[head], tail, value);
}
function ensureTable(root, path) {
    let table = root;
    path.forEach((part) => {
        const next = table[part];
        if (Array.isArray(next)) {
            throw new Error(`TOML table conflicts with existing array table ${path.join('.')}`);
        }
        if (!isTomlTable(next)) {
            table[part] = {};
        }
        table = table[part];
    });
    return table;
}
function appendArrayTable(root, path) {
    const parent = ensureTable(root, path.slice(0, -1));
    const name = path.at(-1) ?? '';
    const current = parent[name];
    const next = {};
    if (current === undefined) {
        parent[name] = [next];
        return next;
    }
    if (Array.isArray(current)) {
        const tables = current;
        tables.push(next);
        return next;
    }
    throw new Error(`TOML array table conflicts with existing table ${path.join('.')}`);
}
