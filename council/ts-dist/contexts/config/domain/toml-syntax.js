export function findCommentIndex(line) {
    let inString = false;
    let escaped = false;
    let depth = 0;
    for (let index = 0; index < line.length; index += 1) {
        const char = line[index];
        if (escaped) {
            escaped = false;
            continue;
        }
        if (char === '\\' && inString) {
            escaped = true;
            continue;
        }
        if (char === '"') {
            inString = !inString;
            continue;
        }
        if (!inString && char === '[') {
            depth += 1;
            continue;
        }
        if (!inString && char === ']') {
            depth -= 1;
            continue;
        }
        if (!inString && depth === 0 && char === '#') {
            return index;
        }
    }
    return -1;
}
export function findTopLevelChar(line, target) {
    let inString = false;
    let escaped = false;
    let depth = 0;
    for (let index = 0; index < line.length; index += 1) {
        const char = line[index];
        if (escaped) {
            escaped = false;
            continue;
        }
        if (char === '\\' && inString) {
            escaped = true;
            continue;
        }
        if (char === '"') {
            inString = !inString;
            continue;
        }
        if (!inString && char === '[') {
            depth += 1;
            continue;
        }
        if (!inString && char === ']') {
            depth -= 1;
            continue;
        }
        if (!inString && depth === 0 && char === target) {
            return index;
        }
    }
    return -1;
}
export function splitTopLevel(raw, delimiter) {
    const parts = [];
    let start = 0;
    let inString = false;
    let escaped = false;
    let depth = 0;
    for (let index = 0; index < raw.length; index += 1) {
        const char = raw[index];
        if (escaped) {
            escaped = false;
            continue;
        }
        if (char === '\\' && inString) {
            escaped = true;
            continue;
        }
        if (char === '"') {
            inString = !inString;
            continue;
        }
        if (!inString && char === '[') {
            depth += 1;
            continue;
        }
        if (!inString && char === ']') {
            depth -= 1;
            continue;
        }
        if (!inString && depth === 0 && char === delimiter) {
            parts.push(raw.slice(start, index));
            start = index + 1;
        }
    }
    parts.push(raw.slice(start));
    return parts;
}
export function findMatchingBracket(raw) {
    let inString = false;
    let escaped = false;
    let depth = 0;
    for (let index = 0; index < raw.length; index += 1) {
        const char = raw[index];
        if (escaped) {
            escaped = false;
            continue;
        }
        if (char === '\\' && inString) {
            escaped = true;
            continue;
        }
        if (char === '"') {
            inString = !inString;
            continue;
        }
        if (!inString && char === '[') {
            depth += 1;
            continue;
        }
        if (!inString && char === ']') {
            depth -= 1;
            if (depth === 0) {
                return index;
            }
        }
    }
    return -1;
}
