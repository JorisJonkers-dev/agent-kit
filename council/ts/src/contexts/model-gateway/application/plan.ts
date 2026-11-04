import { describeWindow, planWindows, warningsOf, type PlanWindowUsage } from '../domain/usage.js'
import { loadConfig } from './config.js'
import { overridePath } from './locations.js'
import type { ModelGatewayDependencies } from './ports.js'

/**
 * What the gateway says about one client's plan limit, in its own words: each
 * window of the plan with what the ledger says is spent in it, the warning
 * for every window at or above the warning ratio, and why the client cannot
 * start at all. A client the machine file gives no plan limit is unwatched:
 * no windows, no warnings, and it can start.
 */
export interface PlanLimitReading {
  readonly client: string
  readonly windows: readonly PlanWindowUsage[]
  /** One line per window at or above the warning ratio, as `usage` words it. */
  readonly warnings: readonly string[]
  /** Why the client cannot start, in the gateway's words; undefined while it can. */
  readonly unable_to_start?: string
}

/**
 * One client's plan limit, read the way `usage` reads it: the shipped
 * defaults with this machine's override over them, and the machine-wide
 * ledger's spend in each window. A caller that runs a client itself, such as
 * critique's runner, asks this before it plans, so it counts the client the
 * way the gateway does.
 */
export async function readPlanLimit(
  deps: Pick<ModelGatewayDependencies, 'clock' | 'defaultsPath' | 'env' | 'files' | 'ledger'>,
  client: string,
): Promise<PlanLimitReading> {
  const config = await loadConfig(deps.files, deps.defaultsPath, overridePath(deps.env))
  const windows = planWindows(
    client,
    config.clients[client]?.plan_limits ?? [],
    await deps.ledger.read(),
    deps.clock.now(),
    config.settings.plan_warning_ratio,
  )
  const exhausted = windows.find((window) => window.state === 'exhausted')
  return {
    client,
    warnings: warningsOf(client, windows),
    windows,
    ...(exhausted === undefined ? {} : { unable_to_start: `at 100% of ${describeWindow(exhausted)}` }),
  }
}
