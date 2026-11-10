import type { EnvPort } from '../../../../ports/index.js'
import { GrantGuard } from '../../application/grants.js'
import { NodeModelGatewayFiles } from '../fs/index.js'
import { NodeModelGatewayGit } from '../git/index.js'
import { NodeModelGatewayGrants } from '../grants/index.js'
import { NodeModelGatewayLedger } from '../ledger/index.js'
import { NodeModelGatewayWorkflows } from '../workflows/index.js'

/** What the real grant guard needs from its caller: the environment. */
export interface NodeGrantGuardOptions {
  readonly env: EnvPort
}

/**
 * The grant guard on this machine's real files: the grants, workflows and
 * machine-wide ledger the gateway itself keeps. It reads them to say whether
 * a live grant covers a workflow, and starts no client.
 */
export function nodeGrantGuard(options: NodeGrantGuardOptions): GrantGuard {
  return new GrantGuard({
    files: new NodeModelGatewayFiles(),
    grants: new NodeModelGatewayGrants(options.env, new NodeModelGatewayGit()),
    ledger: new NodeModelGatewayLedger(options.env),
    workflows: new NodeModelGatewayWorkflows(options.env),
  })
}
