export function createSpawnedEngineStream(config) {
    let inputStarted = false;
    const ensureInput = async () => {
        if (!inputStarted) {
            inputStarted = true;
            await config.initialInput();
        }
    };
    return {
        command: config.command,
        injection: config.injection,
        async inject(prompt) {
            await ensureInput();
            return config.injectInput(prompt);
        },
        async closeInput() {
            await config.child.closeStdin();
        },
        async *[Symbol.asyncIterator]() {
            await ensureInput();
            yield {
                type: 'started',
                engine: config.engine,
                command: config.command,
                injection: config.injection,
            };
            const stdoutLines = [];
            const stderrLines = [];
            for await (const line of readLines(config.child.stdout)) {
                stdoutLines.push(line);
                const parsed = config.parseStdoutLine(line);
                if (parsed.event !== undefined) {
                    yield parsed.event;
                }
            }
            for await (const line of readLines(config.child.stderr)) {
                stderrLines.push(line);
                yield progressEvent('stderr', line);
            }
            const exit = await config.child.exit;
            const stdout = stdoutLines.join('\n');
            const stderr = stderrLines.join('\n');
            if (exit.exitCode !== 0) {
                yield {
                    type: 'failed',
                    exitCode: exit.exitCode,
                    error: stderr.length === 0 ? `engine exited with code ${String(exit.exitCode)}` : stderr,
                    stdout,
                    stderr,
                };
                return;
            }
            const result = await config.readResult(stdout);
            if ('error' in result) {
                yield {
                    type: 'failed',
                    exitCode: exit.exitCode,
                    error: result.error,
                    stdout,
                    stderr,
                };
                return;
            }
            yield resultEvent(result, exit.exitCode);
        },
    };
}
export function progressEvent(stream, text, raw) {
    return raw === undefined
        ? { type: 'progress', stream, text }
        : { type: 'progress', stream, text, raw };
}
function resultEvent(extraction, exitCode) {
    const result = {
        type: 'result',
        text: extraction.text,
        exitCode,
    };
    if (extraction.costUsd !== undefined) {
        result.costUsd = extraction.costUsd;
    }
    if (extraction.metadata !== undefined) {
        result.metadata = extraction.metadata;
    }
    return result;
}
async function* readLines(chunks) {
    let pending = '';
    for await (const chunk of chunks) {
        pending += chunk;
        let newline = pending.indexOf('\n');
        while (newline >= 0) {
            yield stripCarriageReturn(pending.slice(0, newline));
            pending = pending.slice(newline + 1);
            newline = pending.indexOf('\n');
        }
    }
    if (pending.length > 0) {
        yield stripCarriageReturn(pending);
    }
}
function stripCarriageReturn(line) {
    return line.endsWith('\r') ? line.slice(0, -1) : line;
}
