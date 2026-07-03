import { expandCommand } from './commands.js';
import { CHECKPOINT_RESUME_INJECTION, checkpointResumeInjection } from './injection.js';
import { parseJsonRecord } from './json.js';
import { deliverGenericPrompt } from './prompt-delivery.js';
import { extractGenericResult } from './result-extraction.js';
import { createSpawnedEngineStream, progressEvent, } from './spawned-engine-stream.js';
const STDOUT_PARSER_FACTORIES = Object.freeze({
    json: () => new JsonGenericStdoutParser(),
    text: () => new TextGenericStdoutParser(),
});
export class GenericCommandEngineDriver {
    run(request, ports) {
        const command = expandCommand(request.engine, request);
        const child = ports.process.spawn(command);
        const stdoutParser = STDOUT_PARSER_FACTORIES[request.engine.streamFormat]();
        return createSpawnedEngineStream({
            child,
            command,
            engine: request.engine.name,
            injection: CHECKPOINT_RESUME_INJECTION,
            initialInput: async () => {
                await deliverGenericPrompt(request, ports, child);
            },
            injectInput: checkpointResumeInjection,
            parseStdoutLine: (line) => stdoutParser.parse(line),
            readResult: async (stdout) => extractGenericResult(request.engine.resultExtraction, stdout, request.outputFile, stdoutParser.jsonRecords, ports),
        });
    }
}
export const GENERIC_COMMAND_ENGINE_DRIVER = new GenericCommandEngineDriver();
export function runGenericCommandEngine(request, ports) {
    return GENERIC_COMMAND_ENGINE_DRIVER.run(request, ports);
}
class TextGenericStdoutParser {
    jsonRecords = [];
    parse(line) {
        return { event: progressEvent('stdout', line) };
    }
}
class JsonGenericStdoutParser {
    records = [];
    get jsonRecords() {
        return this.records;
    }
    parse(line) {
        const raw = parseJsonRecord(line);
        if (raw !== null) {
            this.records.push(raw);
            return { event: progressEvent('stdout', line, raw) };
        }
        return { event: progressEvent('stdout', line) };
    }
}
