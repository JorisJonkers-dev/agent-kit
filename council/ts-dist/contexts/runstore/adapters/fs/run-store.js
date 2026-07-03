import { readFile } from 'node:fs/promises';
import { join } from 'node:path';
import { planStateWrite, planTasksWrite } from '../../../runstore/index.js';
import { AtomicWriter } from './atomic-writer.js';
import { DESIGN_LEDGER_FILE, RESULT_FILE, STORY_FILE, WORKERS_DIR, assertDesignLedger, assertPathSegment, assertRunState, assertStory, assertTasks, assertWorkerResult, parseJson, } from './artifact-codec.js';
import { EventLog } from './event-log.js';
class DefaultClock {
    now() {
        return new Date();
    }
    monotonicMs() {
        return Number(process.hrtime.bigint() / 1000000n);
    }
    async sleep(ms) {
        await new Promise((resolve) => {
            setTimeout(resolve, ms);
        });
    }
}
export class FsRunStoreAdapter {
    atomicWriter;
    eventLog;
    root;
    tempId;
    tempCounter = 0;
    constructor(root, options = {}) {
        const clock = options.clock ?? new DefaultClock();
        this.tempId =
            options.tempId ??
                (() => `tmp-${String(clock.now().getTime())}-${String(process.pid)}-${String(++this.tempCounter)}`);
        this.atomicWriter = new AtomicWriter(root, this.tempId);
        this.eventLog = new EventLog(root, clock, options.lockRetryMs ?? 25, options.lockTimeoutMs ?? 30_000);
        this.root = root;
    }
    async readState(runId) {
        return assertRunState(await this.readJson(runId, 'state.json'));
    }
    async writeState(runId, state) {
        assertRunState(state);
        await this.atomicWriter.executeJsonPlan(planStateWrite(runId, state, this.nextTempId()));
    }
    async readTasks(runId) {
        return assertTasks(await this.readJson(runId, 'tasks.json'));
    }
    async writeTasks(runId, tasks) {
        assertTasks(tasks);
        await this.atomicWriter.executeJsonPlan(planTasksWrite(runId, tasks, this.nextTempId()));
    }
    async readStory(runId) {
        return assertStory(await this.readJson(runId, STORY_FILE));
    }
    async writeStory(runId, story) {
        assertStory(story);
        await this.atomicWriter.writeJson(this.runFile(runId, STORY_FILE), story);
    }
    async readDesignLedger(runId) {
        return assertDesignLedger(await this.readJson(runId, DESIGN_LEDGER_FILE));
    }
    async writeDesignLedger(runId, ledger) {
        assertDesignLedger(ledger);
        await this.atomicWriter.writeJson(this.runFile(runId, DESIGN_LEDGER_FILE), ledger);
    }
    async appendReviewVerdict(runId, verdict) {
        await this.eventLog.appendReviewVerdict(runId, verdict);
    }
    async appendRoutingVerdict(runId, verdict) {
        await this.eventLog.appendRoutingVerdict(runId, verdict);
    }
    async appendAmendment(runId, amendment) {
        await this.eventLog.appendAmendment(runId, amendment);
    }
    async readEvents(runId) {
        return this.eventLog.read(runId, this.eventLog.eventPath(runId));
    }
    async readWorkerResult(runId, taskId) {
        return assertWorkerResult(await this.readJson(runId, WORKERS_DIR, taskId, RESULT_FILE), taskId);
    }
    async writeWorkerResult(runId, taskId, result) {
        assertPathSegment('taskId', taskId);
        assertWorkerResult(result, taskId);
        await this.atomicWriter.writeJson(this.runFile(runId, WORKERS_DIR, taskId, RESULT_FILE), result);
    }
    nextTempId() {
        return this.tempId();
    }
    async readJson(runId, ...pathSegments) {
        assertPathSegment('runId', runId);
        pathSegments.forEach((segment) => {
            if (segment !== WORKERS_DIR)
                assertPathSegment('path segment', segment);
        });
        return parseJson(await readFile(this.runFile(runId, ...pathSegments), 'utf8'));
    }
    runFile(runId, ...pathSegments) {
        assertPathSegment('runId', runId);
        return join(this.root, runId, ...pathSegments);
    }
}
