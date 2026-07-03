import { CONFIG_KEYS } from './presets.js';
import { parseToml } from './toml-parse.js';
import { findCommentIndex } from './toml-syntax.js';
export function writeTomlValue(value) {
    if (typeof value === 'string') {
        return JSON.stringify(value);
    }
    if (typeof value === 'number') {
        if (!Number.isFinite(value)) {
            throw new Error(`TOML number must be finite, got ${String(value)}`);
        }
        return String(value);
    }
    if (typeof value === 'boolean') {
        return value ? 'true' : 'false';
    }
    if (isTomlArray(value)) {
        return `[${value.map((item) => writeTomlValue(item)).join(', ')}]`;
    }
    throw new Error('inline TOML tables are not supported by the council writer');
}
export function writeCouncilConfig(source, config) {
    return writeTomlUpdates(parseToml(source), flattenCouncilConfig(config));
}
export function writeTomlUpdates(document, updates) {
    const lines = [...document.lines];
    const written = new Set();
    const assignments = [...document.assignments].sort((a, b) => b.lineIndex - a.lineIndex);
    assignments.forEach((assignment) => {
        const updateKey = pathKey([...assignment.tablePath, ...assignment.keyPath]);
        const value = updates.get(updateKey);
        if (value === undefined || written.has(updateKey)) {
            return;
        }
        lines[assignment.lineIndex] = replaceAssignmentValue(lines[assignment.lineIndex] ?? '', assignment.sourceKey, value);
        written.add(updateKey);
    });
    const existingInserts = new Map();
    const newTableInserts = new Map();
    updates.forEach((value, updateKey) => {
        if (written.has(updateKey)) {
            return;
        }
        queueMissingAssignment(document, existingInserts, newTableInserts, updateKey.split('.'), value);
    });
    const orderedExistingInserts = [...existingInserts.entries()].sort(([left], [right]) => right - left);
    orderedExistingInserts.forEach(([index, insertLines]) => lines.splice(index, 0, ...insertLines));
    newTableInserts.forEach((insert) => {
        if (lines.length > 0 && lines.at(-1) !== '') {
            lines.push('');
        }
        lines.push(`[${insert.path.join('.')}]`, ...insert.lines);
    });
    return `${lines.join('\n')}\n`;
}
function isTomlArray(value) {
    return Array.isArray(value);
}
function replaceAssignmentValue(line, sourceKey, value) {
    const commentIndex = findCommentIndex(line);
    const suffix = commentIndex < 0 ? '' : line.slice(commentIndex);
    const prefixMatch = /^(\s*)/.exec(line);
    const prefix = prefixMatch?.[1] ?? '';
    const spacing = suffix === '' ? '' : ' ';
    return `${prefix}${sourceKey} = ${writeTomlValue(value)}${spacing}${suffix}`.trimEnd();
}
function queueMissingAssignment(document, existingInserts, newTableInserts, fullPath, value) {
    const key = fullPath.at(-1);
    if (!key) {
        throw new Error('cannot write empty TOML path');
    }
    const tablePath = fullPath.slice(0, -1);
    const insertLine = `${key} = ${writeTomlValue(value)}`;
    const table = findTable(document, tablePath);
    if (!table && tablePath.length > 0) {
        const tableKey = pathKey(tablePath);
        const pending = newTableInserts.get(tableKey);
        if (pending) {
            pending.lines.push(insertLine);
        }
        else {
            newTableInserts.set(tableKey, { path: tablePath, lines: [insertLine] });
        }
        return;
    }
    const index = table ? findTableInsertIndex(document, table) : findRootInsertIndex(document);
    const pending = existingInserts.get(index);
    if (pending) {
        pending.push(insertLine);
    }
    else {
        existingInserts.set(index, [insertLine]);
    }
}
function findTable(document, tablePath) {
    const tables = [...document.tables].reverse();
    return (tables.find((table) => !table.array && samePath(table.path, tablePath)) ??
        tables.find((table) => table.array && samePath(table.path, tablePath)));
}
function findTableInsertIndex(document, table) {
    const next = document.tables.find((candidate) => candidate.lineIndex > table.lineIndex);
    return next?.lineIndex ?? document.lines.length;
}
function findRootInsertIndex(document) {
    return document.tables[0]?.lineIndex ?? document.lines.length;
}
function samePath(left, right) {
    return left.length === right.length && left.every((part, index) => part === right[index]);
}
function pathKey(path) {
    return path.join('.');
}
function flattenCouncilConfig(config) {
    const updates = new Map();
    addScalars(updates, [], config, CONFIG_KEYS);
    addScalars(updates, ['watchdog'], config.watchdog, [
        'stall_after_s',
        'window',
        'repeat_limit',
        'max_restarts',
        'escalate_model',
        'disk_cap_gib',
    ]);
    addScalars(updates, ['design'], config.design, ['lenses', 'rounds']);
    addNestedScalars(updates, ['design', 'stages'], config.design?.stages, ['engine', 'effort']);
    addScalars(updates, ['review'], config.review, ['council', 'max_fix_rounds']);
    addRecord(updates, ['review', 'difficulty'], config.review?.difficulty);
    addScalars(updates, ['github'], config.github, ['enabled', 'assignee']);
    addNestedScalars(updates, ['engines'], config.engines, ['argv', 'stream_format', 'result_extraction']);
    addRecord(updates, ['triage', 'matrix_overrides'], config.triage?.matrix_overrides);
    addScalars(updates, ['context'], config.context, ['pack_stale_after_s']);
    addRecord(updates, ['model_matrix', 'roles'], config.model_matrix?.roles);
    addNestedScalars(updates, ['model_matrix', 'intensity'], config.model_matrix?.intensity, [
        'rounds',
        'codex_effort',
        'worker',
        'max_workers',
    ]);
    return updates;
}
function addScalars(updates, prefix, source, keys) {
    if (!source) {
        return;
    }
    keys.forEach((key) => {
        const value = source[key];
        if (value !== undefined) {
            updates.set(pathKey([...prefix, key]), value);
        }
    });
}
function addRecord(updates, prefix, source) {
    if (!source) {
        return;
    }
    Object.entries(source).forEach(([key, value]) => {
        if (value !== undefined) {
            updates.set(pathKey([...prefix, key]), value);
        }
    });
}
function addNestedScalars(updates, prefix, source, keys) {
    if (!source) {
        return;
    }
    Object.entries(source).forEach(([name, value]) => {
        addScalars(updates, [...prefix, name], value, keys);
    });
}
