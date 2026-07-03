import { commandParts, optionalCwd } from './commands.js';
import { CHECKPOINT_RESUME_INJECTION, checkpointResumeInjection } from './injection.js';
import { writeStdinPromptAndClose } from './prompt-delivery.js';
import { createSpawnedEngineStream, progressEvent } from './spawned-engine-stream.js';
export class CodexEngineDriver {
    run(request, ports) {
        const command = {
            ...commandParts('codex', [
                'exec',
                '-m',
                request.model,
                '-c',
                `model_reasoning_effort=${request.effort}`,
                '--skip-git-repo-check',
                '-o',
                request.outputFile,
                '-',
            ]),
            ...optionalCwd(request.cwd),
        };
        const child = ports.process.spawn(command);
        return createSpawnedEngineStream({
            child,
            command,
            engine: request.engine.name,
            injection: CHECKPOINT_RESUME_INJECTION,
            initialInput: async () => {
                await writeStdinPromptAndClose(child, request.prompt);
            },
            injectInput: checkpointResumeInjection,
            parseStdoutLine: (line) => ({ event: progressEvent('stdout', line) }),
            readResult: async () => ({ text: await ports.files.readText(request.outputFile) }),
        });
    }
}
export const CODEX_ENGINE_DRIVER = new CodexEngineDriver();
export function runCodexEngine(request, ports) {
    return CODEX_ENGINE_DRIVER.run(request, ports);
}
