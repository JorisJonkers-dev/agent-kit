import { isJsonRecord } from './json.js';
const RESULT_EXTRACTION_FACTORIES = Object.freeze({
    json_path: (extraction) => new JsonPathResultExtractionStrategy(extraction.path),
    output_file: () => OUTPUT_FILE_RESULT_EXTRACTION,
    stdout: () => STDOUT_RESULT_EXTRACTION,
});
export async function extractGenericResult(extraction, stdout, outputFile, jsonRecords, ports) {
    return RESULT_EXTRACTION_FACTORIES[extraction.mode](extraction).extract({
        stdout,
        outputFile,
        jsonRecords,
        ports,
    });
}
export function resultText(value) {
    if (typeof value === 'string') {
        return value;
    }
    return value === undefined ? '' : JSON.stringify(value);
}
export function resultExtraction(text, costUsd, metadata) {
    const result = { text };
    if (costUsd !== undefined) {
        result.costUsd = costUsd;
    }
    if (metadata !== undefined) {
        result.metadata = metadata;
    }
    return result;
}
export function readNumber(value) {
    return typeof value === 'number' ? value : undefined;
}
function extractJsonPathResult(jsonRecords, path) {
    const record = jsonRecords.at(-1);
    if (record === undefined) {
        return { error: 'json result line not found' };
    }
    const value = readJsonPath(record, path);
    if (value === undefined) {
        return { error: `json result path not found: ${path.join('.')}` };
    }
    return { text: resultText(value) };
}
class OutputFileResultExtractionStrategy {
    async extract(context) {
        return { text: await context.ports.files.readText(context.outputFile) };
    }
}
class StdoutResultExtractionStrategy {
    extract(context) {
        return { text: context.stdout };
    }
}
class JsonPathResultExtractionStrategy {
    path;
    constructor(path) {
        this.path = path;
    }
    extract(context) {
        return extractJsonPathResult(context.jsonRecords, this.path);
    }
}
const OUTPUT_FILE_RESULT_EXTRACTION = new OutputFileResultExtractionStrategy();
const STDOUT_RESULT_EXTRACTION = new StdoutResultExtractionStrategy();
function readJsonPath(record, path) {
    let current = record;
    for (const segment of path) {
        current = isJsonRecord(current) ? current[segment] : undefined;
    }
    return current;
}
