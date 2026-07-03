export function parseJsonRecord(line) {
    try {
        const parsed = JSON.parse(line);
        return isJsonRecord(parsed) ? parsed : null;
    }
    catch {
        return null;
    }
}
export function isJsonRecord(value) {
    return typeof value === 'object' && value !== null && !Array.isArray(value);
}
