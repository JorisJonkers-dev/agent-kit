import { join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'

/** Per-machine files live in the owner's home, never in a repository. */
export function allowlistPath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique-allowlist.json')
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
