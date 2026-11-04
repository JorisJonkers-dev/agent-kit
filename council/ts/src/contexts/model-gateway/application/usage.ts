import { sortedClients } from '../domain/models.js'
import { readUsageInput, usageReport, type ClientReading, type UsageResult } from '../domain/usage.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import type { ModelGatewayCommand, ModelGatewayDependencies } from './ports.js'

/**
 * What every client on this machine can do now and what it has spent: its
 * availability, found passively from PATH, its own status command and the
 * ledger's recent outcomes; its usage over the last days, by day and by
 * workflow; its own quota where it reports one; and its plan windows against
 * the ledger. It never sends a prompt to find any of it out.
 */
export class UsageCommand implements ModelGatewayCommand<unknown, UsageResult> {
  constructor(private readonly deps: ModelGatewayDependencies) {}

  async run(raw: unknown): Promise<UsageResult> {
    const input = readUsageInput(raw)
    const config = await loadConfig(this.deps.files, this.deps.defaultsPath, overridePath(this.deps.env))
    const readings: ClientReading[] = []
    for (const [id, client] of sortedClients(config)) {
      const installed = await this.deps.process.which(client.binary)
      const status = installed ? await this.deps.statuses[id]?.read() : undefined
      readings.push({ client: id, installed, status })
    }
    return usageReport(input, config, readings, await this.deps.ledger.read(), this.deps.clock.now())
  }
}
