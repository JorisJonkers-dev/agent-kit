import { sortedClients } from '../domain/models.js'
import { readUsageInput, usageReport, type ClientReading, type UsageResult } from '../domain/usage.js'
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
    const readings: ClientReading[] = []
    for (const [id, client] of sortedClients(config)) {
      const installed = await this.deps.process.which(client.binary)
      const status = installed ? await this.deps.statuses[id]?.read() : undefined
      readings.push({ client: id, installed, status })
    }
    return usageReport(input, config, readings, await this.deps.ledger.read(), now)
  }
}
