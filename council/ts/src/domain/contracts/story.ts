export interface Story {
  readonly title: string
  readonly status: string
  readonly goal: string
  readonly user_value: StoryUserValue
  readonly context: string
  readonly acceptance_criteria: readonly string[]
  readonly scope: StoryScope
  readonly implementation_notes: StoryImplementationNotes
  readonly tests: StoryTests
  readonly definition_of_done: readonly string[]
}

export interface StoryUserValue {
  readonly actor: string
  readonly capability: string
  readonly outcome: string
}

export interface StoryScope {
  readonly in_scope: readonly string[]
  readonly out_of_scope: readonly string[]
}

export interface StoryImplementationNotes {
  readonly files: readonly string[]
  readonly patterns: readonly string[]
  readonly dependencies: readonly string[]
  readonly data_config_migration: readonly string[]
}

export interface StoryTests {
  readonly unit: readonly string[]
  readonly integration: readonly string[]
  readonly manual_or_workflow: readonly string[]
}
