import { spawn as nodeSpawn } from 'node:child_process';
import { readDuBytes } from './disk-usage.js';
import { defaultProcessKiller, killDetachedProcessGroup, } from './process-group-control.js';
import { RunningWorker } from './session.js';
import { joinPrompt } from './session-support.js';
export class WorkerSupervisorAdapter {
    spawnChild;
    killPid;
    sleepFor;
    nowMs;
    duBytes;
    onEvent;
    constructor(dependencies = {}) {
        this.spawnChild =
            dependencies.spawn ??
                ((command, args, options) => nodeSpawn(command, [...args], options));
        this.killPid = dependencies.kill ?? defaultProcessKiller;
        this.sleepFor = dependencies.sleep ?? sleep;
        this.nowMs = dependencies.nowMs ?? Date.now;
        this.duBytes = dependencies.duBytes ?? readDuBytes;
        this.onEvent = dependencies.onEvent ?? (() => undefined);
    }
    start(request) {
        return new RunningWorker(this, request);
    }
    createChild(request, run, input) {
        const env = {
            ...process.env,
            ...request.env,
            ...(input.modelTier === undefined ? {} : { COUNCIL_MODEL_TIER: input.modelTier }),
        };
        const child = this.spawnChild(request.command, request.args ?? [], {
            cwd: request.worktree,
            detached: true,
            env,
            stdio: ['pipe', 'pipe', 'pipe'],
        });
        this.onEvent({ pid: child.pid, restart: run, type: 'started' });
        writeInitialStdin(child, joinPrompt(input.preamble, request.stdin));
        return child;
    }
    emit(event) {
        this.onEvent(event);
    }
    now() {
        return this.nowMs();
    }
    sleep(ms) {
        return this.sleepFor(ms);
    }
    diskUsageBytes(path) {
        return this.duBytes(path);
    }
    killGroup(pid, signal) {
        killDetachedProcessGroup(pid, signal, this.killPid);
    }
}
function writeInitialStdin(child, input) {
    if (input === undefined || child.stdin === null) {
        return;
    }
    child.stdin.write(input.endsWith('\n') ? input : `${input}\n`);
}
function sleep(ms) {
    return new Promise((resolve) => {
        const timer = setTimeout(resolve, ms);
        timer.unref();
    });
}
