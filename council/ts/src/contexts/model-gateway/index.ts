export { createModelGatewayCommands, type ModelGatewayCommands } from './application/commands.js'
export { decidePolicyFor, resolveScope } from './application/policy.js'
export type {
  ModelGatewayDependencies,
  ModelGatewayFilesPort,
  ModelGatewayGitPort,
  ModelGatewayProcessPort,
  ModelSourcePort,
} from './application/ports.js'
export type { AllowlistEntry, FamilyAllowlist } from './domain/allowlist.js'
export { ModelGatewayError, type ModelGatewayErrorCode } from './domain/config.js'
export type { ClientListing, ClientModel, ListedModel, MissingClient, ModelListing, SkippedModel } from './domain/models.js'
export type {
  DataScope,
  ModelPermission,
  PermittedModel,
  PolicyDecision,
  PolicyListing,
  PolicySubject,
  SendPermission,
} from './domain/policy.js'
export { modelSources } from './adapters/clients/index.js'
export { NodeModelGatewayFiles } from './adapters/fs/index.js'
export { NodeModelGatewayGit } from './adapters/git/index.js'
export { NodeModelGatewayProcess } from './adapters/process/index.js'
