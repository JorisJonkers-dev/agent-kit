export {
  ARCHETYPE_CONTEXT_DEFAULTS,
  NAMED_CONTEXT_PROFILES,
  normalizeArchetype,
  parseContextProfile,
  resolveContextProfile,
} from './profiles.js'
export type {
  ContextArchetype,
  ContextAxes,
  ContextProfileRequest,
  McpAxis,
  NetworkAxis,
  RepoContextAxis,
  ResolvedContextProfile,
  SkillsAxis,
  WorkspaceAxis,
} from './profiles.js'

export {
  MODEL_MATRIX,
  MODEL_TIER_ALIASES,
  normalizeModelTier,
  parseEngineSpec,
  parseRoleClass,
  resolveModelMatrix,
} from './model-matrix.js'
export type {
  ModelMatrixEntry,
  ModelMatrixRequest,
  ModelRoleClass,
  ModelTierName,
  ModelWorkload,
  ResolvedModel,
} from './model-matrix.js'

export {
  checkContextPackStaleness,
  createTaskInclusionQuery,
  indexContextPack,
  parseSpecSections,
  seedContextPackIfAbsent,
  selectContextSlice,
  selectFragments,
  selectSpecSections,
} from './packs.js'
export type {
  ContextFragment,
  ContextFragmentKind,
  ContextPack,
  ContextPackIndex,
  ContextPackSnippet,
  ContextPackStaleness,
  ContextSelectionOptions,
  ContextSlice,
  InclusionQuery,
  SpecSection,
} from './packs.js'
