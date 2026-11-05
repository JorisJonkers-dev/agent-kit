import { isAbsolute, join } from 'node:path'

import type { EnvPort } from '../../../ports/index.js'

/**
 * Per-machine files live in the owner's home, never in a repository:
 * `$XDG_CONFIG_HOME/agent-kit/`, or `~/.config/agent-kit/` when
 * XDG_CONFIG_HOME is unset or not an absolute path.
 */
function configDir(env: EnvPort): string {
  const xdg = env.get('XDG_CONFIG_HOME')
  if (xdg !== undefined && isAbsolute(xdg)) return join(xdg, 'agent-kit')
  return join(env.require('HOME'), '.config', 'agent-kit')
}

/** The family allowlist the model gateway reads too, so both resolve it the same way. */
export function allowlistPath(env: EnvPort): string {
  return join(configDir(env), 'critique-allowlist.json')
}

/** This machine's routing override, applied key by key over the routing agent-kit ships. */
export function routingOverridePath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique-routing.json')
}

/** This machine's critique configuration: its `disabled` flag is the per-machine kill switch. */
export function critiqueConfigPath(env: EnvPort): string {
  return join(env.require('HOME'), '.config', 'agent-kit', 'critique.json')
}

/** Claude Code's config root, where its settings.json wires hooks: `CLAUDE_CONFIG_DIR`, or `~/.claude`. */
export function claudeHomePath(env: EnvPort): string {
  return env.get('CLAUDE_CONFIG_DIR') ?? join(env.require('HOME'), '.claude')
}

/** Codex's home, where hooks.json wires hooks and config.toml records their trust: `CODEX_HOME`, or `~/.codex`. */
export function codexHomePath(env: EnvPort): string {
  return env.get('CODEX_HOME') ?? join(env.require('HOME'), '.codex')
}

/** Where the gate logs each time it failed open, one JSON line each, beside the ledger. */
export function gateLogPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'gate.log.jsonl')
}

/** One ledger per user, in agent-kit's data directory. */
export function ledgerPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'ledger.jsonl')
}

/** This machine's setup results, one per check, beside the ledger. */
export function setupResultsPath(env: EnvPort): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'machine-setup.json')
}

/** Where setup's trivial prompt to one critic client runs: a packet of one note, the prompt and what the client writes. */
export function setupProbeDir(env: EnvPort, client: string): string {
  return join(env.require('HOME'), '.local', 'share', 'agent-kit', 'critique', 'setup-checks', client)
}

export function manifestPath(runDir: string): string {
  return join(runDir, 'manifest.json')
}

export function findingsPath(runDir: string): string {
  return join(runDir, 'findings.json')
}

/** Written by refute: the refute round's result for each accepted critical finding, which record applies. */
export function refutePath(runDir: string): string {
  return join(runDir, 'refute.json')
}

/** Written by record, so its presence means the run's verdicts are recorded. */
export function reportPath(runDir: string): string {
  return join(runDir, 'report.md')
}
