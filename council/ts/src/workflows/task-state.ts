export interface ResumableTaskState {
  readonly taskId: string
  readonly stepsDone: readonly string[]
  readonly nextStep: string
  readonly watchedSha?: string
  readonly watchedPr?: string
  readonly monitorName?: string
  readonly updatedAt: string // ISO8601
}
