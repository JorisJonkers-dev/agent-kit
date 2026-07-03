import { constants } from 'node:fs'
import { appendFile, mkdir, open, readFile, unlink } from 'node:fs/promises'
import { dirname, join } from 'node:path'

import {
  amendmentEvent,
  planEventAppend,
  reviewVerdictEvent,
  routingVerdictEvent,
  RUNSTORE_EVENTS_FILE,
  type RunStoreEvent,
} from '../../../../domain/runstore/index.js'
import type { ClockPort } from '../../../../ports/index.js'
import type { Amendment, ReviewVerdict, RoutingVerdict } from '../../../../domain/contracts/index.js'
import { syncFile } from './atomic-writer.js'
import {
  assertAmendment,
  assertPathSegment,
  assertReviewVerdict,
  assertRoutingVerdict,
  assertRunStoreEvent,
  isErrno,
  parseJson,
} from './artifact-codec.js'

export class EventLog {
  private readonly clock: ClockPort
  private readonly lockRetryMs: number
  private readonly lockTimeoutMs: number
  private readonly root: string

  constructor(root: string, clock: ClockPort, lockRetryMs: number, lockTimeoutMs: number) {
    this.clock = clock
    this.lockRetryMs = lockRetryMs
    this.lockTimeoutMs = lockTimeoutMs
    this.root = root
  }

  async appendReviewVerdict(runId: string, verdict: ReviewVerdict): Promise<void> {
    assertReviewVerdict(verdict)
    await this.append(runId, reviewVerdictEvent(verdict))
  }

  async appendRoutingVerdict(runId: string, verdict: RoutingVerdict): Promise<void> {
    assertRoutingVerdict(verdict)
    await this.append(runId, routingVerdictEvent(verdict))
  }

  async appendAmendment(runId: string, amendment: Amendment): Promise<void> {
    assertAmendment(amendment)
    await this.append(runId, amendmentEvent(amendment))
  }

  async read(runId: string, eventPath: string): Promise<readonly RunStoreEvent[]> {
    assertPathSegment('runId', runId)
    const text = await readFile(eventPath, 'utf8')
    return text
      .split('\n')
      .filter((line) => line.length > 0)
      .map((line) => assertRunStoreEvent(parseJson(line)))
  }

  eventPath(runId: string): string {
    assertPathSegment('runId', runId)
    return join(this.root, runId, RUNSTORE_EVENTS_FILE)
  }

  private async append(runId: string, event: RunStoreEvent): Promise<void> {
    const plan = planEventAppend(runId, event)
    const lockPath = join(this.root, plan.lockPath)
    await mkdir(dirname(lockPath), { recursive: true })
    const lock = await this.acquireLock(lockPath)
    try {
      const eventPath = join(this.root, plan.eventPath)
      await appendFile(eventPath, plan.bytes, 'utf8')
      await syncFile(eventPath)
    } finally {
      await lock.close()
      await unlink(lockPath)
    }
  }

  private async acquireLock(lockPath: string): Promise<Awaited<ReturnType<typeof open>>> {
    const start = this.clock.monotonicMs()
    for (;;) {
      try {
        return await open(lockPath, constants.O_CREAT | constants.O_EXCL | constants.O_RDWR)
      } catch (error) {
        if (!isErrno(error, 'EEXIST') || this.clock.monotonicMs() - start >= this.lockTimeoutMs) throw error
        await this.clock.sleep(this.lockRetryMs)
      }
    }
  }
}
