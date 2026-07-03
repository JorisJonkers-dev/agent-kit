import { advanceEscalation, appendLoopLine, createEscalationState, createLoopDetectorState, createStallDetectorState, evaluateDiskUsageCap, evaluateStall, } from '../../../watchdog/index.js';
import { terminateActiveProcess } from './process-group-control.js';
import { isPromiseLike, joinPrompt, normalizeWatchdogConfig, spawnInput, thenMaybe, } from './session-support.js';
import { optional } from './types.js';
export class RunningWorker {
    result;
    supervisor;
    request;
    watchdog;
    pollIntervalMs;
    killGraceMs;
    active;
    run = 0;
    stdout = '';
    stderr = '';
    logBytes = 0;
    restarts = 0;
    modelTier;
    stallState;
    loopState = createLoopDetectorState();
    escalationState = createEscalationState();
    pendingLoopDetection = null;
    handlingDetection = false;
    finished = false;
    resolveResult;
    constructor(supervisor, request) {
        this.supervisor = supervisor;
        this.request = request;
        this.watchdog = normalizeWatchdogConfig(request.watchdog);
        this.pollIntervalMs = request.pollIntervalMs ?? 15_000;
        this.killGraceMs = request.killGraceMs ?? 5_000;
        this.modelTier = request.modelTier;
        this.stallState = createStallDetectorState(supervisor.now());
        this.result = new Promise((resolve) => {
            this.resolveResult = resolve;
        });
        this.active = this.spawn({ ...optional('modelTier', this.modelTier) });
        void this.poll();
    }
    async inject(turn) {
        if (this.finished) {
            throw new Error(`worker ${this.request.id} has already finished`);
        }
        if (this.request.supportsStreamingStdin === true && this.active.child.stdin?.writable === true) {
            this.active.child.stdin.write(`${turn}\n`);
            this.supervisor.emit({ mode: 'streaming-stdin', type: 'injected' });
            return { mode: 'streaming-stdin', restarted: false };
        }
        const preamble = joinPrompt(this.request.checkpointPreamble ?? 'Resume from checkpoint.', turn) ?? '';
        await this.restartWithPreamble(preamble, this.modelTier);
        this.supervisor.emit({ mode: 'checkpoint-and-resume', type: 'injected' });
        return { mode: 'checkpoint-and-resume', restarted: true };
    }
    async stop(reason = 'requested') {
        if (!this.finished) {
            await this.terminate(this.active, 'SIGTERM');
            this.finish('stopped', null, null, undefined);
            this.supervisor.emit({ reason, type: 'stopped' });
        }
    }
    async poll() {
        while (!this.finished) {
            await this.supervisor.sleep(this.pollIntervalMs);
            if (this.isFinished() || this.active.exited) {
                return;
            }
            const maybeDetection = this.pollDetection();
            const detection = isPromiseLike(maybeDetection) ? await maybeDetection : maybeDetection;
            if (detection !== null) {
                const handled = this.handleDetection(detection);
                if (isPromiseLike(handled)) {
                    await handled;
                }
            }
        }
    }
    pollDetection() {
        if (this.watchdog.diskCapBytes !== undefined) {
            return this.supervisor.diskUsageBytes(this.request.worktree).then((duBytes) => {
                const diskDetection = evaluateDiskUsageCap({
                    capBytes: this.watchdog.diskCapBytes ?? 0,
                    duBytes,
                });
                return diskDetection ?? this.pollLogDetection();
            });
        }
        return this.pollLogDetection();
    }
    pollLogDetection() {
        if (this.pendingLoopDetection !== null) {
            const detection = this.pendingLoopDetection;
            this.pendingLoopDetection = null;
            return detection;
        }
        const stall = evaluateStall(this.stallState, {
            logBytes: this.logBytes,
            nowMs: this.supervisor.now(),
            stallAfterS: this.watchdog.stallAfterS,
        });
        this.stallState = stall.state;
        return stall.detection;
    }
    handleDetection(detection) {
        if (!this.handlingDetection && !this.finished) {
            this.handlingDetection = true;
            this.supervisor.emit({ detection, type: 'detected' });
            const handled = detection.kind === 'disk-cap'
                ? thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
                    this.finish('disk-cap', null, null, detection);
                })
                : this.advanceRestartPolicy(detection);
            if (isPromiseLike(handled)) {
                return handled.finally(() => {
                    this.handlingDetection = false;
                });
            }
            this.handlingDetection = false;
        }
    }
    advanceRestartPolicy(detection) {
        const first = advanceEscalation(this.escalationState, {
            enableTierEscalation: this.watchdog.enableTierEscalation,
        });
        this.escalationState = first.state;
        if (first.action === 'terminate') {
            return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
                const next = advanceEscalation(this.escalationState, {
                    enableTierEscalation: this.watchdog.enableTierEscalation,
                });
                this.escalationState = next.state;
                return this.applyEscalationAction(next.action, detection);
            });
        }
        return this.applyEscalationAction(first.action, detection);
    }
    applyEscalationAction(action, detection) {
        if (action === 'retry-with-preamble') {
            return this.restartWithPreamble(this.restartPreamble(detection), this.modelTier, detection);
        }
        if (action === 'escalate-tier' && this.request.escalationModelTier !== undefined) {
            this.modelTier = this.request.escalationModelTier;
            this.supervisor.emit({ modelTier: this.modelTier, type: 'tier-escalated' });
            return this.restartWithPreamble(this.restartPreamble(detection), this.modelTier, detection);
        }
        return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
            this.finish('stalled', null, null, detection);
        });
    }
    restartWithPreamble(preamble, modelTier, detection) {
        return thenMaybe(this.terminate(this.active, 'SIGTERM'), () => {
            if (this.restarts >= this.watchdog.maxRestarts) {
                this.finish('stalled', null, null, detection);
                return;
            }
            this.restarts += 1;
            this.pendingLoopDetection = null;
            this.stallState = createStallDetectorState(this.supervisor.now(), this.logBytes);
            this.active = this.spawn(spawnInput(preamble, modelTier));
            this.supervisor.emit({ preamble, restart: this.restarts, type: 'restarted' });
        });
    }
    spawn(input) {
        this.run += 1;
        const run = this.run;
        const child = this.supervisor.createChild(this.request, run, input);
        const active = {
            child,
            exitCode: null,
            exited: false,
            exitedPromise: Promise.resolve(),
            signal: null,
            terminating: false,
        };
        active.exitedPromise = new Promise((resolve) => {
            child.once('exit', (exitCode, signal) => {
                active.exited = true;
                active.exitCode = exitCode;
                active.signal = signal;
                this.supervisor.emit({ exitCode, signal, type: 'exited' });
                if (this.active === active && !this.finished && !active.terminating) {
                    this.finish(exitCode === 0 ? 'completed' : 'failed', exitCode, signal, undefined);
                }
                resolve();
            });
        });
        child.stdout?.on('data', (chunk) => { this.recordOutput('stdout', chunk); });
        child.stderr?.on('data', (chunk) => { this.recordOutput('stderr', chunk); });
        return active;
    }
    recordOutput(stream, chunk) {
        const text = chunk.toString();
        this.logBytes += Buffer.byteLength(text);
        if (stream === 'stdout') {
            this.stdout += text;
            this.supervisor.emit({ chunk: text, type: 'stdout' });
        }
        else {
            this.stderr += text;
            this.supervisor.emit({ chunk: text, type: 'stderr' });
        }
        for (const line of text.split(/\r?\n/u)) {
            const loop = appendLoopLine(this.loopState, line, this.watchdog.loop);
            this.loopState = loop.state;
            this.pendingLoopDetection = this.pendingLoopDetection ?? loop.detection;
        }
    }
    terminate(active, signal) {
        return terminateActiveProcess(active, signal, this.supervisor, this.killGraceMs);
    }
    isFinished() {
        return this.finished;
    }
    restartPreamble(detection) {
        return joinPrompt(this.request.restartPreamble ?? 'Previous attempt was interrupted by the watchdog.', `Watchdog detection: ${detection.kind}.`) ?? '';
    }
    finish(status, exitCode, signal, detection) {
        if (!this.finished) {
            this.finished = true;
            this.resolveResult({
                ...optional('detection', detection),
                exitCode,
                id: this.request.id,
                ...optional('modelTier', this.modelTier),
                restarts: this.restarts,
                signal,
                status,
                stderr: this.stderr,
                stdout: this.stdout,
            });
        }
    }
}
