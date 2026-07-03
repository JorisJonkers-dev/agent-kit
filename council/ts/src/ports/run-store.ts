import type {
  Amendment,
  DesignLedger,
  ReviewVerdict,
  RoutingVerdict,
  RunState,
  Story,
  Task,
} from '../shared-kernel/index.js'

export interface RunStorePort {
  readState(runId: string): Promise<RunState>
  writeState(runId: string, state: RunState): Promise<void>
  readTasks(runId: string): Promise<readonly Task[]>
  writeTasks(runId: string, tasks: readonly Task[]): Promise<void>
  readStory(runId: string): Promise<Story>
  writeStory(runId: string, story: Story): Promise<void>
  readDesignLedger(runId: string): Promise<DesignLedger>
  writeDesignLedger(runId: string, ledger: DesignLedger): Promise<void>
  appendReviewVerdict(runId: string, verdict: ReviewVerdict): Promise<void>
  appendRoutingVerdict(runId: string, verdict: RoutingVerdict): Promise<void>
  appendAmendment(runId: string, amendment: Amendment): Promise<void>
}
