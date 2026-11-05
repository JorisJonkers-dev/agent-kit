import type { ClientConfig, GatewayConfig } from '../domain/config.js'
import { sortedClients } from '../domain/models.js'
import { readUsageInput, usageReport, type Availability, type ClientReading, type UsageResult } from '../domain/usage.js'
import { CapGuard } from './caps.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/**
 * What every client on this machine can do now and what it has spent: its
 * availability, found passively from PATH, its own status command and the
 * ledger's recent outcomes; its usage over the last days, by day and by
 * workflow; its own quota where it reports one; its plan windows against the
 * ledger; and what live reservations hold for it. It never sends a prompt to
 * find any of it out.
 *
 * It settles the reservations whose runs died first, so a report never shows a
 * reservation as held after the moment it lapsed.
 */
export class UsageCommand implements ModelGatewayCommand<unknown, UsageResult> {
  private readonly caps: CapGuard

  constructor(private readonly deps: ModelGatewayDependencies) {
    this.caps = new CapGuard(deps)
  }

  async run(raw: unknown): Promise<UsageResult> {
    const input = readUsageInput(raw)
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const now = this.deps.clock.now()
    await this.caps.settleStale(now)
    const readings = await readClients(this.deps, sortedClients(config))
    return usageReport(input, config, readings, await this.deps.ledger.read(), now)
  }
}

/**
 * Whether one client can start now: the availability `usage` reports for it,
 * read from the same computation but asking only that client's own status
 * command, so a delegation never waits on the status of clients it does not
 * use. A client the gateway does not know has none.
 */
export async function clientAvailability(
  deps: ModelGatewayDependencies,
  config: GatewayConfig,
  client: string,
): Promise<Availability | undefined> {
  const known = config.clients[client]
  const readings = await readClients(deps, known === undefined ? [] : [[client, known]])
  const report = usageReport({ days: 1 }, config, readings, await deps.ledger.read(), deps.clock.now())
  return report.clients.find((entry) => entry.client === client)?.availability
}

/** What the gateway finds out about each of these clients without sending a prompt: PATH, and the client's own status command. */
async function readClients(
  deps: ModelGatewayDependencies,
  clients: readonly (readonly [string, ClientConfig])[],
): Promise<readonly ClientReading[]> {
  const readings: ClientReading[] = []
  for (const [id, client] of clients) {
    const installed = await deps.process.which(client.binary)
    const status = installed ? await deps.statuses[id]?.read() : undefined
    readings.push({ client: id, installed, status })
  }
  return readings
}
