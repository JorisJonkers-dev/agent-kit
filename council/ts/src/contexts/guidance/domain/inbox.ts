export type GuidanceSource = 'inject' | 'watchdog_restart' | 'review_fix_loop'

export type GuidanceInboxStatus = 'pending' | 'delivered' | 'dismissed'

export interface GuidanceInput {
  readonly id: string
  readonly task_id: string
  readonly source: GuidanceSource
  readonly message: string
  readonly created_at?: string
}

export interface GuidanceEntry extends GuidanceInput {
  readonly sequence: number
  readonly task_sequence: number
}

export interface GuidanceInboxItem {
  readonly guidance_id: string
  readonly task_id: string
  readonly sequence: number
  readonly status: GuidanceInboxStatus
}

export interface GuidanceState {
  readonly log: readonly GuidanceEntry[]
  readonly inbox: readonly GuidanceInboxItem[]
  readonly next_sequence: number
}

export interface GuidanceAppendResult {
  readonly state: GuidanceState
  readonly entry: GuidanceEntry
  readonly inbox_item: GuidanceInboxItem
}

export function emptyGuidanceState(): GuidanceState {
  return {
    log: [],
    inbox: [],
    next_sequence: 1,
  }
}

export function appendGuidance(state: GuidanceState, input: GuidanceInput): GuidanceAppendResult {
  if (state.log.some((entry) => entry.id === input.id)) {
    throw new Error(`guidance id already exists: ${input.id}`)
  }

  const entry: GuidanceEntry = {
    ...input,
    sequence: state.next_sequence,
    task_sequence: nextTaskSequence(state.log, input.task_id),
  }
  const inboxItem: GuidanceInboxItem = {
    guidance_id: entry.id,
    task_id: entry.task_id,
    sequence: entry.sequence,
    status: 'pending',
  }

  return {
    state: {
      log: [...state.log, entry],
      inbox: [...state.inbox, inboxItem],
      next_sequence: state.next_sequence + 1,
    },
    entry,
    inbox_item: inboxItem,
  }
}

export function guidanceForTask(state: GuidanceState, taskId: string): readonly GuidanceEntry[] {
  return state.log
    .filter((entry) => entry.task_id === taskId)
    .toSorted((left, right) => left.task_sequence - right.task_sequence)
}

export function pendingInbox(state: GuidanceState): readonly GuidanceInboxItem[] {
  return state.inbox
    .filter((item) => item.status === 'pending')
    .toSorted((left, right) => left.sequence - right.sequence)
}

export function updateInboxStatus(
  state: GuidanceState,
  guidanceId: string,
  status: GuidanceInboxStatus,
): GuidanceState {
  if (!state.inbox.some((item) => item.guidance_id === guidanceId)) {
    throw new Error(`guidance inbox item does not exist: ${guidanceId}`)
  }

  return {
    ...state,
    inbox: state.inbox.map((item) =>
      item.guidance_id === guidanceId
        ? {
            ...item,
            status,
          }
        : item,
    ),
  }
}

function nextTaskSequence(log: readonly GuidanceEntry[], taskId: string): number {
  const previousTaskEntries = log.filter((entry) => entry.task_id === taskId)
  if (previousTaskEntries.length === 0) {
    return 1
  }

  return Math.max(...previousTaskEntries.map((entry) => entry.task_sequence)) + 1
}
