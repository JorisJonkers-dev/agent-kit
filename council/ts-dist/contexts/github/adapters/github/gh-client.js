export class GithubCommandError extends Error {
    args;
    cwd;
    result;
    constructor(args, cwd, result) {
        const detail = result.stderr.trim() || result.stdout.trim() || `exit code ${String(result.exitCode)}`;
        super(`gh ${args.join(' ')} failed in ${cwd}: ${detail}`);
        this.name = 'GithubCommandError';
        this.args = args;
        this.cwd = cwd;
        this.result = result;
    }
}
export class GhCommandClient {
    process;
    constructor(process) {
        this.process = process;
    }
    async ghJson(cwd, args) {
        const result = await this.gh(cwd, args);
        return result.stdout;
    }
    async gh(cwd, args) {
        const command = {
            args,
            command: 'gh',
            cwd,
        };
        const result = await this.process.exec(command);
        if (result.exitCode !== 0) {
            throw new GithubCommandError(args, cwd, result);
        }
        return result;
    }
}
