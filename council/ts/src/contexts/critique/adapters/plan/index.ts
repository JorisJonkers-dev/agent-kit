import type { ClockPort, EnvPort } from '../../../../ports/index.js'
import {
  NodeModelGatewayFiles,
  NodeModelGatewayLedger,
  readPlanLimit,
  type PlanLimitReading,
} from '../../../model-gateway/index.js'
import type { PlanLimitPort } from '../../application/ports.js'

/**
 * A client's plan limit, read by the model gateway's own code: the shipped
 * defaults with this machine's override over them, and the machine-wide
 * ledger's spend in each window. Critique asks this instead of holding a
 * dollar cap of its own, so a Vibe critic is counted against what the plan
 * actually allows.
 */
export class GatewayPlanLimit implements PlanLimitPort {
  private readonly files = new NodeModelGatewayFiles()
  private readonly ledger: NodeModelGatewayLedger

  constructor(
    private readonly env: EnvPort,
    private readonly clock: ClockPort,
    /** The gateway's shipped defaults, beside its bundle. */
    private readonly defaultsPath: string,
  ) {
    this.ledger = new NodeModelGatewayLedger(env)
  }

  read(client: string): Promise<PlanLimitReading> {
    return readPlanLimit({ clock: this.clock, defaultsPath: this.defaultsPath, env: this.env, files: this.files, ledger: this.ledger }, client)
  }
}
