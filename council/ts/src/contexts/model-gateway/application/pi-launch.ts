import { basename, join } from 'node:path'

import { messageOf } from './json-file.js'

import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import { isObject, type JsonObject, type ToolList, type ToolScope } from '../domain/config.js'
import type { HomePorts } from './home-mcp.js'
import type { PiConfinement } from './launchers.js'

/** The longest the secret store may take to give the key; it reads a local file and calls no model. */
const KEY_TIMEOUT_MS = 20_000

/** What preparing a confined Pi launch reaches the system through. */
export interface PiLaunchPorts extends HomePorts {
  readonly files: HomePorts['files'] & {
    /** Writes a file only its owner can read, creating its parent directories. */
    writeTextPrivate?(path: string, content: string): Promise<void>
    /** Removes a file or a directory with everything in it. */
    remove?(path: string): Promise<void>
    /** Copies a file, or a folder with everything in it, to `to`, making its parent first; symlinks are copied as what they point at. */
    copyTree?(from: string, to: string): Promise<void>
  }
  readonly process: { exec(command: ProcessCommand): Promise<ProcessResult> }
}

/** Why a Pi run cannot start without the launcher. */
export const NO_LAUNCHER =
  'pi: agent-launcher is not on PATH, and a Pi delegate only runs confined to its working directory; re-run agent-kit setup, which installs it'

const FIX = 're-run agent-kit setup so it writes the LiteLLM providers into ~/.pi/agent/models.json'

/** The home's Pi directory: `PI_CODING_AGENT_DIR`, else `~/.pi/agent`. */
export function homeAgentDir(ports: HomePorts): string {
  const own = ports.env.get('PI_CODING_AGENT_DIR') ?? ''
  return own.length > 0 ? own : join(ports.env.get('HOME') ?? '', '.pi', 'agent')
}

/** The port of an `http` or `https` URL: the one after the host, else the scheme's own. */
export function urlPort(url: string): number | undefined {
  const scheme = url.startsWith('https://') ? 'https' : url.startsWith('http://') ? 'http' : undefined
  if (scheme === undefined) return undefined
  const authority = url.slice(scheme.length + 3).split('/')[0] ?? ''
  const colon = authority.lastIndexOf(':')
  if (colon < 0) return scheme === 'https' ? 443 : 80
  const port = authority.slice(colon + 1)
  return /^[0-9]{1,5}$/.test(port) && Number(port) > 0 && Number(port) < 65536 ? Number(port) : undefined
}

/** What a confined launch needs of the route's provider in the home's `models.json`. */
interface PiRoute {
  readonly baseUrl: string
  readonly api: string
  readonly apiKey: string
  readonly model: JsonObject
  readonly port: number
}

/** The route's provider as the home's `models.json` holds it, or why a confined launch cannot use it. */
async function routeOf(ports: HomePorts, provider: string, model: string): Promise<PiRoute | string> {
  const path = join(homeAgentDir(ports), 'models.json')
  let parsed: unknown
  try {
    parsed = JSON.parse(await ports.files.readText(path)) as unknown
  } catch {
    return `pi: ${path} cannot be read; ${FIX}`
  }
  const providers = isObject(parsed) && isObject(parsed.providers) ? parsed.providers : {}
  const entry = providers[provider]
  if (!isObject(entry)) return `pi: ${path} has no ${provider} provider; ${FIX}`
  const { api, apiKey, baseUrl } = entry
  if (typeof baseUrl !== 'string' || typeof api !== 'string' || typeof apiKey !== 'string') {
    return `pi: the ${provider} provider in ${path} lacks a baseUrl, api or apiKey; ${FIX}`
  }
  const port = urlPort(baseUrl)
  if (port === undefined) return `pi: the ${provider} provider's baseUrl is not an http or https URL with a port the confinement can allow`
  const models: readonly unknown[] = Array.isArray(entry.models) ? (entry.models as unknown[]) : []
  const routed = models.find((candidate): candidate is JsonObject => isObject(candidate) && candidate.id === model)
  return { api, apiKey, baseUrl, model: routed ?? { id: model }, port }
}

/** The key: a literal as it is, or `!<program> get <NAME>` run without a shell; never a shell command. */
async function keyOf(ports: PiLaunchPorts, provider: string, apiKey: string): Promise<{ value: string } | { error: string }> {
  if (!apiKey.startsWith('!')) return { value: apiKey }
  const words = apiKey.slice(1).trim().split(/\s+/)
  const [program = '', get = '', name = ''] = words
  if (words.length !== 3 || get !== 'get') {
    return { error: `pi: the ${provider} key is a shell command the gateway does not run; setup writes it as !agent-kit-secret get <NAME>` }
  }
  let said: string
  try {
    const read = await ports.process.exec({ args: ['get', name], command: program, timeoutMs: KEY_TIMEOUT_MS })
    const value = read.timedOut === true || read.exitCode !== 0 ? '' : read.stdout.trim()
    if (value.length > 0) return { value }
    said = firstLine(read.stderr) ?? `exit ${String(read.exitCode)}`
  } catch (error) {
    said = error instanceof Error ? error.message : String(error)
  }
  // Only the error stream is shown: a failed read never puts anything of the key in a message.
  return { error: `pi: the ${provider} key could not be read from the secret store (${name}): ${said}` }
}

function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}

/** The launcher that confines a launch: `AGENT_KIT_LAUNCHER_BIN`, else `agent-launcher` on `PATH`. */
async function launcherOf(ports: HomePorts): Promise<string | undefined> {
  const named = ports.env.get('AGENT_KIT_LAUNCHER_BIN') ?? ''
  if (named.length > 0) return named
  for (const dir of (ports.env.get('PATH') ?? '').split(':').filter((entry) => entry.length > 0)) {
    if (await ports.files.exists(join(dir, 'agent-launcher'))) return 'agent-launcher'
  }
  return undefined
}

/**
 * What a scope puts in a Pi launch's home: each skill and plugin the scope
 * names, with the place in the operator's home it is copied from. Nothing the
 * scope does not list is here, so nothing else is copied.
 */
export interface PiScope {
  /** A folder holding a `SKILL.md`. */
  readonly skills: readonly { readonly name: string; readonly from: string }[]
  /** A Pi extension, a file or a folder. */
  readonly plugins: readonly { readonly name: string; readonly from: string }[]
}

/** The skills and plugins a scope lists, found in the operator's home, or why one is not there. */
export async function resolveScope(ports: HomePorts, tools: ToolScope): Promise<PiScope | string> {
  // The scope has been checked by `unenforceable`, which refuses `any`; only the names it lists are looked for.
  const names = (list: ToolList): readonly string[] => (list === 'any' ? [] : list)
  const find = async (candidates: readonly string[]): Promise<string | undefined> => {
    for (const path of candidates) if (await ports.files.exists(path)) return path
    return undefined
  }
  const skills: { name: string; from: string }[] = []
  const homes = [join(homeAgentDir(ports), 'skills'), join(ports.env.get('HOME') ?? '', '.agents', 'skills')]
  for (const name of names(tools.skills)) {
    const marked = homes.map((home) => join(home, name, 'SKILL.md'))
    const at = await find(marked)
    if (at === undefined) return `skill ${name} is not in Pi's home: looked for ${marked.join(' and ')}`
    skills.push({ from: at.slice(0, -'/SKILL.md'.length), name })
  }
  const plugins: { name: string; from: string }[] = []
  const extensions = join(homeAgentDir(ports), 'extensions')
  for (const name of names(tools.plugins)) {
    const candidates = [join(extensions, name), join(extensions, `${name}.ts`), join(extensions, `${name}.js`)]
    const at = await find(candidates)
    if (at === undefined) return `plugin ${name} is not in Pi's home: looked for ${candidates.slice(0, 2).join(', ')} and ${candidates[2] ?? ''}`
    plugins.push({ from: at, name })
  }
  return { plugins, skills }
}

/**
 * The launch's own agent directory: a `models.json` holding only the route's
 * provider, its key read from the secret store just now and written in plain
 * (mode 0600), and an empty `settings.json`; and what the confined launch runs
 * through. A string is why no launch can start: the home has no provider for
 * the route, the key cannot be read, or the operating system has no launcher
 * to confine with, and a Pi delegate never runs unconfined. With a `scope`,
 * the directory also holds a copy of each skill and plugin it lists and
 * nothing else of the operator's, and the launch loads only those.
 */
export async function preparePi(ports: PiLaunchPorts, provider: string, model: string, agentDir: string, scope?: ToolScope): Promise<PiConfinement | string> {
  const route = await routeOf(ports, provider, model)
  if (typeof route === 'string') return route
  const launcher = await launcherOf(ports)
  if (launcher === undefined) return NO_LAUNCHER
  const listed = scope === undefined ? { plugins: [], skills: [] } : await resolveScope(ports, scope)
  if (typeof listed === 'string') return listed
  const key = await keyOf(ports, provider, route.apiKey)
  if ('error' in key) return key.error
  const files = ports.files
  if (files.writeTextPrivate === undefined) return 'pi: the launch has no way to write its private agent directory'
  const entry = { api: route.api, apiKey: key.value, baseUrl: route.baseUrl, models: [route.model] }
  try {
    await files.writeTextPrivate(join(agentDir, 'settings.json'), '{}\n')
  } catch (error) {
    return `pi: cannot write the launch's agent directory ${agentDir}: ${messageOf(error)}`
  }
  const copy = async (from: string, kind: string, name: string, folder: string): Promise<string | { to: string }> => {
    const to = join(agentDir, folder, basename(from))
    if (files.copyTree === undefined) return `pi: the launch has no way to copy ${kind} ${name} into its agent directory`
    try {
      await files.copyTree(from, to)
      return { to }
    } catch (error) {
      return `pi: cannot copy ${kind} ${name} into the launch's agent directory ${agentDir}: ${messageOf(error)}`
    }
  }
  const skills: string[] = []
  for (const skill of listed.skills) {
    const done = await copy(skill.from, 'skill', skill.name, 'skills')
    if (typeof done === 'string') return done
    skills.push(done.to)
  }
  const extensions: string[] = []
  for (const plugin of listed.plugins) {
    const done = await copy(plugin.from, 'plugin', plugin.name, 'extensions')
    if (typeof done === 'string') return done
    extensions.push(done.to)
  }
  // The key goes in last, so a write or a copy that fails leaves no key behind.
  try {
    await files.writeTextPrivate(join(agentDir, 'models.json'), `${JSON.stringify({ providers: { [provider]: entry } }, null, 2)}\n`)
  } catch (error) {
    return `pi: cannot write the launch's agent directory ${agentDir}: ${messageOf(error)}`
  }
  const bin = join(homeAgentDir(ports), 'bin')
  const path = ports.env.get('PATH')
  return {
    agentDir,
    launcher,
    port: route.port,
    ...((await ports.files.exists(bin)) ? { bin } : {}),
    ...(path === undefined ? {} : { path }),
    ...(skills.length === 0 ? {} : { skills }),
    ...(extensions.length === 0 ? {} : { extensions }),
  }
}
