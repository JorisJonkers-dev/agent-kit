export class GitCommandError extends Error {
    args;
    cwd;
    result;
    constructor(args, cwd, result) {
        const detail = result.stderr.trim() || result.stdout.trim() || `exit code ${String(result.exitCode)}`;
        super(`git ${args.join(' ')} failed in ${cwd}: ${detail}`);
        this.name = 'GitCommandError';
        this.args = args;
        this.cwd = cwd;
        this.result = result;
    }
}
export class GitCliAdapter {
    integrationQueue = Promise.resolve();
    process;
    constructor(process) {
        this.process = process;
    }
    async root(cwd) {
        const result = await this.git(cwd, ['rev-parse', '--show-toplevel']);
        return result.stdout.trim();
    }
    async currentBranch(cwd) {
        const result = await this.git(cwd, ['branch', '--show-current']);
        return result.stdout.trim();
    }
    async createWorktree(cwd, branch, path) {
        await this.git(cwd, ['worktree', 'add', '-b', branch, path, 'HEAD']);
        return { branch, path };
    }
    async removeWorktree(cwd, path) {
        await this.git(cwd, ['worktree', 'remove', '--force', path]);
    }
    async changedFiles(cwd) {
        const entries = await this.status(cwd);
        return entries.map((entry) => entry.path);
    }
    async status(cwd) {
        const result = await this.git(cwd, ['status', '--porcelain=v1']);
        const output = result.stdout.trimEnd();
        if (output.length === 0) {
            return [];
        }
        return output.split('\n').map((line) => this.parseStatusLine(line));
    }
    async diff(cwd, options = {}) {
        const args = ['diff'];
        if (options.staged === true) {
            args.push('--staged');
        }
        const range = this.diffRange(options);
        if (range !== undefined) {
            args.push(range);
        }
        if (options.paths !== undefined && options.paths.length > 0) {
            args.push('--', ...options.paths);
        }
        const result = await this.git(cwd, args);
        return result.stdout;
    }
    async reconcileIntegrationBranch(cwd, request) {
        const job = this.integrationQueue.then(() => this.reconcileIntegrationBranchNow(cwd, request));
        this.integrationQueue = job.then(() => undefined, () => undefined);
        return job;
    }
    async pushBranch(cwd, request) {
        const remote = request.remote ?? 'origin';
        const setUpstream = request.setUpstream ?? true;
        const args = ['push'];
        if (setUpstream) {
            args.push('--set-upstream');
        }
        if (request.forceWithLease === true) {
            args.push('--force-with-lease');
        }
        args.push(remote, request.branch);
        await this.git(cwd, args);
    }
    async reconcileIntegrationBranchNow(cwd, request) {
        await this.checkoutIntegrationBranch(cwd, request);
        const merge = await this.git(cwd, ['merge', '--no-ff', '--no-edit', request.sourceBranch], {
            acceptedExitCodes: [0, 1],
        });
        if (merge.exitCode !== 0) {
            await this.git(cwd, ['merge', '--abort'], { acceptedExitCodes: [0, 128] });
            throw new GitCommandError(['merge', '--no-ff', '--no-edit', request.sourceBranch], cwd, merge);
        }
        const head = await this.git(cwd, ['rev-parse', 'HEAD']);
        return {
            head: head.stdout.trim(),
            integrationBranch: request.integrationBranch,
            sourceBranch: request.sourceBranch,
        };
    }
    async checkoutIntegrationBranch(cwd, request) {
        const branchRef = `refs/heads/${request.integrationBranch}`;
        const existing = await this.git(cwd, ['show-ref', '--verify', '--quiet', branchRef], {
            acceptedExitCodes: [0, 1],
        });
        if (existing.exitCode === 0) {
            await this.git(cwd, ['checkout', request.integrationBranch]);
            return;
        }
        const baseBranch = request.baseBranch ?? 'HEAD';
        await this.git(cwd, ['checkout', '-B', request.integrationBranch, baseBranch]);
    }
    async git(cwd, args, options = {}) {
        const command = {
            args,
            command: 'git',
            cwd,
        };
        const result = await this.process.exec(command);
        const acceptedExitCodes = options.acceptedExitCodes ?? [0];
        if (!acceptedExitCodes.includes(result.exitCode)) {
            throw new GitCommandError(args, cwd, result);
        }
        return result;
    }
    parseStatusLine(line) {
        const index = line.slice(0, 1);
        const worktree = line.slice(1, 2);
        const path = line.slice(3);
        const renameSeparator = ' -> ';
        const renameAt = path.indexOf(renameSeparator);
        if (renameAt === -1) {
            return { index, path, worktree };
        }
        return {
            index,
            originalPath: path.slice(0, renameAt),
            path: path.slice(renameAt + renameSeparator.length),
            worktree,
        };
    }
    diffRange(options) {
        if (options.base !== undefined && options.head !== undefined) {
            return `${options.base}..${options.head}`;
        }
        return options.base ?? options.head;
    }
}
