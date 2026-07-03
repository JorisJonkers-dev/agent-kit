import type { CouncilConfig, CouncilPreset, CouncilRoleKey, TomlTable } from './config-value-objects.js'
import {
  omitUndefined,
  optionalBoolean,
  optionalNumber,
  optionalObject,
  optionalString,
  optionalStringArray,
  optionalStringRecord,
  optionalStringRecordTable,
} from './config-value-objects.js'
import { optionalCodexEffort, optionalIntensity, ROLE_KEYS } from './presets.js'

export function normalizeCouncilConfig(data: TomlTable): CouncilConfig {
  return omitUndefined({
    intensity: optionalIntensity(data.intensity),
    planner_a: optionalString(data.planner_a),
    planner_b: optionalString(data.planner_b),
    consolidator: optionalString(data.consolidator),
    worker: optionalString(data.worker),
    verifier: optionalString(data.verifier),
    codex_effort: optionalCodexEffort(data.codex_effort),
    rounds: optionalNumber(data.rounds),
    max_workers: optionalNumber(data.max_workers),
    watchdog: optionalObject(data.watchdog, (watchdog) =>
      omitUndefined({
        stall_after_s: optionalNumber(watchdog.stall_after_s),
        window: optionalNumber(watchdog.window),
        repeat_limit: optionalNumber(watchdog.repeat_limit),
        max_restarts: optionalNumber(watchdog.max_restarts),
        escalate_model: optionalString(watchdog.escalate_model),
        disk_cap_gib: optionalNumber(watchdog.disk_cap_gib),
      }),
    ),
    design: optionalObject(data.design, (design) =>
      omitUndefined({
        lenses: optionalStringArray(design.lenses),
        rounds: optionalNumber(design.rounds),
        stages: optionalStringRecordTable(design.stages, (stage) =>
          omitUndefined({
            engine: optionalString(stage.engine),
            effort: optionalString(stage.effort),
          }),
        ),
      }),
    ),
    review: optionalObject(data.review, (review) =>
      omitUndefined({
        council: optionalBoolean(review.council),
        max_fix_rounds: optionalNumber(review.max_fix_rounds),
        difficulty: optionalStringRecord(review.difficulty),
      }),
    ),
    github: optionalObject(data.github, (github) =>
      omitUndefined({
        enabled: optionalBoolean(github.enabled),
        assignee: optionalString(github.assignee),
      }),
    ),
    engines: optionalStringRecordTable(data.engines, (engine) =>
      omitUndefined({
        argv: optionalStringArray(engine.argv),
        stream_format: optionalString(engine.stream_format),
        result_extraction: optionalString(engine.result_extraction),
      }),
    ),
    triage: optionalObject(data.triage, (triage) =>
      omitUndefined({
        matrix_overrides: optionalStringRecord(triage.matrix_overrides),
      }),
    ),
    context: optionalObject(data.context, (context) =>
      omitUndefined({
        pack_stale_after_s: optionalNumber(context.pack_stale_after_s),
      }),
    ),
    model_matrix: optionalObject(data.model_matrix, (modelMatrix) =>
      omitUndefined({
        roles: optionalRoleRecord(modelMatrix.roles),
        intensity: optionalStringRecordTable(modelMatrix.intensity, (preset) =>
          omitUndefined({
            rounds: optionalNumber(preset.rounds),
            codex_effort: optionalCodexEffort(preset.codex_effort),
            worker: optionalString(preset.worker),
            max_workers: optionalNumber(preset.max_workers),
          }) as Partial<CouncilPreset>,
        ),
      }),
    ),
  })
}

function optionalRoleRecord(value: unknown): Partial<Record<CouncilRoleKey, string>> | undefined {
  const record = optionalStringRecord(value)
  if (!record) {
    return undefined
  }
  return Object.fromEntries(Object.entries(record).filter(([key]) => (ROLE_KEYS as readonly string[]).includes(key)))
}
