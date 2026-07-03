import { constants } from 'node:fs';
import { appendFile, mkdir, open, readFile, unlink } from 'node:fs/promises';
import { dirname, join } from 'node:path';
import { amendmentEvent, planEventAppend, reviewVerdictEvent, routingVerdictEvent, RUNSTORE_EVENTS_FILE, } from '../../../runstore/index.js';
import { syncFile } from './atomic-writer.js';
import { assertAmendment, assertPathSegment, assertReviewVerdict, assertRoutingVerdict, assertRunStoreEvent, isErrno, parseJson, } from './artifact-codec.js';
export class EventLog {
    clock;
    lockRetryMs;
    lockTimeoutMs;
    root;
    constructor(root, clock, lockRetryMs, lockTimeoutMs) {
        this.clock = clock;
        this.lockRetryMs = lockRetryMs;
        this.lockTimeoutMs = lockTimeoutMs;
        this.root = root;
    }
    async appendReviewVerdict(runId, verdict) {
        assertReviewVerdict(verdict);
        await this.append(runId, reviewVerdictEvent(verdict));
    }
    async appendRoutingVerdict(runId, verdict) {
        assertRoutingVerdict(verdict);
        await this.append(runId, routingVerdictEvent(verdict));
    }
    async appendAmendment(runId, amendment) {
        assertAmendment(amendment);
        await this.append(runId, amendmentEvent(amendment));
    }
    async read(runId, eventPath) {
        assertPathSegment('runId', runId);
        const text = await readFile(eventPath, 'utf8');
        return text
            .split('\n')
            .filter((line) => line.length > 0)
            .map((line) => assertRunStoreEvent(parseJson(line)));
    }
    eventPath(runId) {
        assertPathSegment('runId', runId);
        return join(this.root, runId, RUNSTORE_EVENTS_FILE);
    }
    async append(runId, event) {
        const plan = planEventAppend(runId, event);
        const lockPath = join(this.root, plan.lockPath);
        await mkdir(dirname(lockPath), { recursive: true });
        const lock = await this.acquireLock(lockPath);
        try {
            const eventPath = join(this.root, plan.eventPath);
            await appendFile(eventPath, plan.bytes, 'utf8');
            await syncFile(eventPath);
        }
        finally {
            await lock.close();
            await unlink(lockPath);
        }
    }
    async acquireLock(lockPath) {
        const start = this.clock.monotonicMs();
        for (;;) {
            try {
                return await open(lockPath, constants.O_CREAT | constants.O_EXCL | constants.O_RDWR);
            }
            catch (error) {
                if (!isErrno(error, 'EEXIST') || this.clock.monotonicMs() - start >= this.lockTimeoutMs)
                    throw error;
                await this.clock.sleep(this.lockRetryMs);
            }
        }
    }
}
