import type { ClockPort, EnvPort } from '../../../../ports/index.js'
import { EngineGuard } from '../../application/engine-guard.js'
import { NodeModelGatewayFiles } from '../fs/index.js'
import { NodeModelGatewayGit } from '../git/index.js'
import { NodeModelGatewayGrants } from '../grants/index.js'
import { NodeModelGatewayLedger } from '../ledger/index.js'
import { NodeModelGatewayWorkflows } from '../workflows/index.js'

/** What the real guard needs from its caller: the environment, the clock and where the shipped defaults are. */
export interface NodeEngineGuardOptions {
  readonly clock: ClockPort
  /** The shipped defaults, which sit in the model-gateway skill's own folder. */
  readonly defaultsPath: string
  readonly env: EnvPort
}

/**
 * The engine guard on this machine's real files: the machine-wide ledger, the
 * workflows and the grants the gateway itself keeps, and git for the data
 * policy. It starts no client and sends no prompt.
 */
export function nodeEngineGuard(options: NodeEngineGuardOptions): EngineGuard {
  const git = new NodeModelGatewayGit()
  return new EngineGuard({
    clock: options.clock,
    defaultsPath: options.defaultsPath,
    env: options.env,
    files: new NodeModelGatewayFiles(),
    git,
    grants: new NodeModelGatewayGrants(options.env, git),
    ledger: new NodeModelGatewayLedger(options.env),
    workflows: new NodeModelGatewayWorkflows(options.env),
  })
}
