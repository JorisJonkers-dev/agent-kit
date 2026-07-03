export function expandCommand(engine, request) {
    const [command, ...args] = engine.argv.map((arg) => arg
        .replaceAll('{model}', request.model)
        .replaceAll('{effort}', request.effort)
        .replaceAll('{output}', request.outputFile)
        .replaceAll('{prompt_file}', request.promptFile));
    return {
        ...commandParts(command ?? engine.name, args),
        ...optionalCwd(request.cwd),
    };
}
export function commandParts(command, args) {
    return { command, args };
}
export function optionalCwd(cwd) {
    return cwd === undefined ? {} : { cwd };
}
