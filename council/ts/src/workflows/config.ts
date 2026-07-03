import {
  coerceConfigValue,
  CONFIG_KEYS,
  parseCouncilConfig,
  resolveCouncilConfig,
  writeCouncilConfig,
} from '../contexts/config/index.js'
import type { CouncilConfig, ResolvedCouncilConfig } from '../contexts/config/index.js'

export interface ConfigPaths {
  readonly project: string
  readonly user: string
}

export interface ConfigCommandInput {
  readonly action: 'show' | 'get' | 'set' | 'unset' | 'path'
  readonly key?: string
  readonly value?: string
  readonly project?: boolean
  readonly paths: ConfigPaths
}

export interface ConfigCommandResult {
  readonly config?: CouncilConfig
  readonly key?: string
  readonly paths: ConfigPaths
  readonly resolved?: ResolvedCouncilConfig
  readonly target?: string
  readonly value?: unknown
}

export interface ConfigWorkflowDeps {
  readonly readText: (path: string) => Promise<string>
  readonly writeText: (path: string, text: string) => Promise<void>
}

export async function configWorkflow(
  input: ConfigCommandInput,
  deps: ConfigWorkflowDeps,
): Promise<ConfigCommandResult> {
  if (input.action === 'path') {
    return { paths: input.paths }
  }

  const user = await readOptionalConfig(input.paths.user, deps.readText)
  const project = await readOptionalConfig(input.paths.project, deps.readText)
  const target = input.project ? input.paths.project : input.paths.user
  const current = input.project ? project : user

  if (input.action === 'show') {
    return {
      config: current,
      paths: input.paths,
      resolved: resolveCouncilConfig({ project, user }),
      target,
    }
  }

  if (input.action === 'get') {
    const key = requireConfigKey(input.key)
    const resolved = resolveCouncilConfig({ project, user })
    return { key, paths: input.paths, resolved, value: resolved[key], target }
  }

  if (input.action === 'set') {
    const key = requireConfigKey(input.key)
    if (input.value === undefined) throw new Error('config set requires <key> <value>')
    const next = { ...current, [key]: coerceConfigValue(key, input.value) }
    await writeConfig(target, next, deps, undefined)
    return { config: next, key, paths: input.paths, target, value: next[key] }
  }

  const key = requireConfigKey(input.key)
  const next = omitKey(current, key)
  await writeConfig(target, next, deps, key)
  return { config: next, key, paths: input.paths, target }
}

async function readOptionalConfig(path: string, readText: (path: string) => Promise<string>): Promise<CouncilConfig> {
  try {
    return parseCouncilConfig(await readText(path))
  } catch (error) {
    if (isErrno(error, 'ENOENT')) return {}
    throw error
  }
}

async function writeConfig(
  path: string,
  next: CouncilConfig,
  deps: ConfigWorkflowDeps,
  unsetKey: string | undefined,
): Promise<void> {
  let source = ''
  try {
    source = await deps.readText(path)
  } catch (error) {
    if (!isErrno(error, 'ENOENT')) throw error
  }
  const writableSource = unsetKey === undefined ? source : removeRootAssignment(source, unsetKey)
  await deps.writeText(path, writeCouncilConfig(writableSource, next))
}

function requireConfigKey(key: string | undefined): (typeof CONFIG_KEYS)[number] {
  if (key === undefined) throw new Error('config action requires a key')
  if (!(CONFIG_KEYS as readonly string[]).includes(key)) {
    throw new Error(`unknown key ${key}; choose from ${CONFIG_KEYS.join(', ')}`)
  }
  return key as (typeof CONFIG_KEYS)[number]
}

function omitKey<T extends object>(object: T, key: string): T {
  return Object.fromEntries(Object.entries(object).filter(([k]) => k !== key)) as T
}

function removeRootAssignment(source: string, key: string): string {
  const lines = source.replace(/\r\n/gu, '\n').split('\n')
  let inTable = false
  const kept = lines.filter((line) => {
    if (/^\s*\[/.test(line)) inTable = true
    return inTable || !new RegExp(`^\\s*${escapeRegExp(key)}\\s*=`).test(line)
  })
  return kept.join('\n')
}

function escapeRegExp(value: string): string {
  return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&')
}

function isErrno(error: unknown, code: string): boolean {
  return error instanceof Error && 'code' in error && error.code === code
}
