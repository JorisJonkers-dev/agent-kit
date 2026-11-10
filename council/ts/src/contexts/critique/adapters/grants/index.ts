import type { ClockPort, EnvPort } from '../../../../ports/index.js'
import { nodeGrantGuard } from '../../../model-gateway/index.js'
import type { GrantsPort } from '../../application/ports.js'

/**
 * Whether a model-gateway grant covers a workflow, read by the gateway's own
 * code from the grants, workflows and ledger it keeps on this machine, so a
 * critic is allowed to Codex under exactly the grant a delegation would be.
 */
export class GatewayGrants implements GrantsPort {
  private readonly guard

  constructor(
    env: EnvPort,
    private readonly clock: ClockPort,
  ) {
    this.guard = nodeGrantGuard({ env })
  }

  async covers(workflow: string, client: string): Promise<boolean> {
    return (await this.guard.live(workflow, client, this.clock.now())) !== undefined
  }
}
