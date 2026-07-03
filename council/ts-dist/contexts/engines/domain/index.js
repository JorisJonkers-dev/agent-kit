const PLACEHOLDER_PATTERN = /\{([a-z_]+)\}/g;
const BUILTIN_ENGINE_ENTRIES = {
    claude: {
        argv: [
            'sh',
            '-lc',
            'COUNCIL_EFFORT={effort} claude -p --model {model} --output-format json --permission-mode plan < {prompt_file} > {output}',
        ],
        stream_format: 'json',
        result_extraction: 'json.result',
    },
    codex: {
        argv: [
            'sh',
            '-lc',
            'codex exec -m {model} -c model_reasoning_effort={effort} --skip-git-repo-check -o {output} "$(cat {prompt_file})"',
        ],
        stream_format: 'text',
        result_extraction: 'output_file',
    },
};
const ENGINE_ENTRY_KEYS = new Set(['argv', 'label', 'prompt_delivery', 'result_extraction', 'stream_format']);
const PLACEHOLDERS = new Set(['effort', 'model', 'output', 'prompt_file']);
export function parseEngineRegistryConfig(config) {
    const configuredEntries = readConfiguredEntries(config);
    const engines = {};
    for (const [name, entry] of Object.entries(BUILTIN_ENGINE_ENTRIES)) {
        engines[name] = parseEngineEntry(name, entry, `engines.${name}`);
    }
    for (const [name, entry] of Object.entries(configuredEntries)) {
        engines[name] = parseEngineEntry(name, entry, `engines.${name}`);
    }
    return { engines };
}
export function getEngineDef(registry, name) {
    const engine = registry.engines[name];
    if (engine === undefined) {
        throw new Error(`Unknown engine: ${name}`);
    }
    return engine;
}
function readConfiguredEntries(config) {
    if (config === undefined || config === null) {
        return {};
    }
    const record = expectRecord(config, 'config');
    if ('engines' in record) {
        const entries = record.engines;
        return entries === undefined ? {} : expectRecord(entries, 'engines');
    }
    return record;
}
function parseEngineEntry(name, entry, path) {
    validateEngineName(name, path);
    const record = expectRecord(entry, path);
    validateKnownKeys(record, path);
    const argv = readArgv(record.argv, `${path}.argv`);
    const placeholders = collectPlaceholders(argv, `${path}.argv`);
    requirePlaceholder(placeholders, 'model', `${path}.argv`);
    requirePlaceholder(placeholders, 'effort', `${path}.argv`);
    requirePlaceholder(placeholders, 'output', `${path}.argv`);
    const promptDelivery = readPromptDelivery(record.prompt_delivery, placeholders, path);
    const streamFormat = readStreamFormat(record.stream_format, `${path}.stream_format`);
    const resultExtraction = readResultExtraction(record.result_extraction, `${path}.result_extraction`);
    const label = readOptionalString(record.label, `${path}.label`);
    return label === undefined
        ? { name, argv, promptDelivery, streamFormat, resultExtraction }
        : { name, argv, promptDelivery, streamFormat, resultExtraction, label };
}
function validateEngineName(name, path) {
    if (!/^[A-Za-z0-9][A-Za-z0-9_.-]*$/.test(name)) {
        throw new Error(`${path} has invalid engine name ${JSON.stringify(name)}`);
    }
}
function validateKnownKeys(record, path) {
    for (const key of Object.keys(record)) {
        if (!ENGINE_ENTRY_KEYS.has(key)) {
            throw new Error(`${path}.${key} is not a supported engine config key`);
        }
    }
}
function readArgv(value, path) {
    if (!Array.isArray(value) || value.length === 0) {
        throw new Error(`${path} must be a non-empty string array`);
    }
    return value.map((arg, index) => {
        if (typeof arg !== 'string' || arg.length === 0) {
            throw new Error(`${path}[${String(index)}] must be a non-empty string`);
        }
        return arg;
    });
}
function collectPlaceholders(argv, path) {
    const placeholders = new Set();
    for (const arg of argv) {
        for (const match of arg.matchAll(PLACEHOLDER_PATTERN)) {
            const [, placeholder = ''] = match;
            if (placeholder === 'prompt') {
                throw new Error(`${path} must not inline prompts with {prompt}; use {prompt_file} or stdin`);
            }
            if (!PLACEHOLDERS.has(placeholder)) {
                throw new Error(`${path} contains unsupported placeholder {${placeholder}}`);
            }
            placeholders.add(placeholder);
        }
    }
    return placeholders;
}
function requirePlaceholder(placeholders, placeholder, path) {
    if (!placeholders.has(placeholder)) {
        throw new Error(`${path} must include {${placeholder}}`);
    }
}
function readPromptDelivery(value, placeholders, path) {
    if (value === undefined) {
        return placeholders.has('prompt_file') ? 'prompt_file' : 'stdin';
    }
    if (value !== 'prompt_file' && value !== 'stdin') {
        throw new Error(`${path}.prompt_delivery must be "prompt_file" or "stdin"`);
    }
    if (value === 'prompt_file' && !placeholders.has('prompt_file')) {
        throw new Error(`${path}.argv must include {prompt_file} when prompt_delivery is "prompt_file"`);
    }
    if (value === 'stdin' && placeholders.has('prompt_file')) {
        throw new Error(`${path}.argv must not include {prompt_file} when prompt_delivery is "stdin"`);
    }
    return value;
}
function readStreamFormat(value, path) {
    if (value === undefined) {
        return 'text';
    }
    if (value !== 'json' && value !== 'text') {
        throw new Error(`${path} must be "json" or "text"`);
    }
    return value;
}
function readResultExtraction(value, path) {
    if (value === undefined || value === 'output_file') {
        return { mode: 'output_file' };
    }
    if (value === 'stdout') {
        return { mode: 'stdout' };
    }
    if (typeof value === 'string' && value.startsWith('json.')) {
        const jsonPath = value.slice('json.'.length).split('.');
        if (jsonPath.every((segment) => /^[A-Za-z0-9_-]+$/.test(segment))) {
            return { mode: 'json_path', path: jsonPath };
        }
    }
    throw new Error(`${path} must be "output_file", "stdout", or a json.<field> path`);
}
function readOptionalString(value, path) {
    if (value === undefined) {
        return undefined;
    }
    if (typeof value !== 'string' || value.length === 0) {
        throw new Error(`${path} must be a non-empty string`);
    }
    return value;
}
function expectRecord(value, path) {
    if (typeof value !== 'object' || value === null || Array.isArray(value)) {
        throw new Error(`${path} must be an object`);
    }
    return value;
}
