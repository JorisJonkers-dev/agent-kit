import { join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'

/** Per-machine files live in the owner's home, never in a repository. */
export function allowlistPath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique-allowlist.json')
}

/** This machine's routing override, applied key by key over the routing agent-kit ships. */
export function routingOverridePath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique-routing.json')
}

/** This machine's critique configuration: its `disabled` flag is the per-machine kill switch. */
export function critiqueConfigPath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique.json')
}

/** Where the gate logs each time it failed open, one JSON line each, beside the ledger. */
export function gateLogPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'gate.log.jsonl')
}

/** One ledger per user, in agent-kit's data directory. */
export function ledgerPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl')
}

export function manifestPath(runDir: string): string {
  return join(runDir, 'manifest.json')
}

export function findingsPath(runDir: string): string {
  return join(runDir, 'findings.json')
}

/** Written by record, so its presence means the run's verdicts are recorded. */
export function reportPath(runDir: string): string {
  return join(runDir, 'report.md')
}
