import { lstat, readdir, readFile, realpath } from 'node:fs/promises'
import { isAbsolute, join, relative, sep } from 'node:path'

import {
  cappedAnswer,
  clipLine,
  offeredSearchTools,
  symbolMatches,
  type SearchLimits,
  type SearchToolName,
} from '../../domain/packet-search.js'
import { REPO_MAP_PATH } from '../../domain/repo-map.js'
import { matchesGlob } from '../../domain/triage.js'

/** A tool call's arguments, as the client sent them. */
export type SearchArguments = Readonly<Record<string, unknown>>

/** A tool answer: its text, and whether the call itself was wrong (a bad pattern, a missing argument). */
export interface SearchAnswer {
  readonly text: string
  readonly isError: boolean
}

const OUTSIDE = 'Nothing: that path is outside the packet, or is not in it.'

/**
 * The search over one packet copy. Every path resolves inside the root:
 * an absolute path or one that climbs out finds nothing, no symlink is
 * followed or listed, and a path whose real location is outside the root
 * finds nothing either. It reads files and nothing else.
 */
export class PacketSearchEngine {
  private constructor(
    /** The packet root, with its own symlinks resolved, so containment is checked on real paths. */
    private readonly root: string,
    private readonly limits: SearchLimits,
    readonly tools: readonly SearchToolName[],
  ) {}

  /** The engine for `root`, which must be a directory; `symbol` is offered when it holds a repo map that is a plain file. */
  static async open(root: string, limits: SearchLimits): Promise<PacketSearchEngine> {
    const real = await realpath(root)
    if (!(await lstat(real)).isDirectory()) throw new Error(`${root} is not a directory`)
    const map = await lstat(join(real, REPO_MAP_PATH)).catch(() => undefined)
    return new PacketSearchEngine(real, limits, offeredSearchTools(map?.isFile() === true))
  }

  async call(tool: string, args: SearchArguments): Promise<SearchAnswer> {
    if (!(this.tools as readonly string[]).includes(tool)) return { isError: true, text: `unknown tool ${tool}` }
    try {
      if (tool === 'grep') return { isError: false, text: await this.grep(args) }
      if (tool === 'list') return { isError: false, text: await this.list(args) }
      return { isError: false, text: await this.symbol(args) }
    } catch (error) {
      return { isError: true, text: error instanceof Error ? error.message : String(error) }
    }
  }

  private async grep(args: SearchArguments): Promise<string> {
    const flags = args.ignore_case === true ? 'i' : ''
    const pattern = new RegExp(requiredString(args, 'pattern'), flags)
    const glob = optionalString(args, 'glob')
    const scope = optionalString(args, 'path')
    const start = scope === undefined ? this.root : await this.inside(scope)
    if (start === undefined) return OUTSIDE
    const hits: string[] = []
    for (const path of await this.files(start)) {
      if (glob !== undefined && !matchesGlob(path, glob)) continue
      const lines = (await readFile(join(this.root, path), 'utf8')).split('\n')
      lines.forEach((line, index) => {
        if (pattern.test(line)) hits.push(`${path}:${String(index + 1)}: ${clipLine(line)}`)
      })
    }
    return cappedAnswer(hits, this.limits, 'matches', 'No matches.')
  }

  private async list(args: SearchArguments): Promise<string> {
    const glob = requiredString(args, 'glob')
    const paths = (await this.files(this.root)).filter((path) => matchesGlob(path, glob))
    return cappedAnswer(paths, this.limits, 'paths', 'No paths match.')
  }

  private async symbol(args: SearchArguments): Promise<string> {
    const map = await readFile(join(this.root, REPO_MAP_PATH), 'utf8')
    return cappedAnswer(symbolMatches(map, requiredString(args, 'name')), this.limits, 'matches', 'No symbol of that name.')
  }

  /**
   * The absolute path `path` names inside the root, or undefined when it is
   * absolute, climbs out, passes through a symlink or does not exist.
   */
  private async inside(path: string): Promise<string | undefined> {
    if (isAbsolute(path)) return undefined
    const target = join(this.root, path)
    if (!within(this.root, target)) return undefined
    const real = await realpath(target).catch(() => undefined)
    if (real !== target) return undefined
    return target
  }

  /** Every plain file under `dir`, relative to the root, sorted; symlinks are skipped, never followed. */
  private async files(dir: string): Promise<readonly string[]> {
    const stat = await lstat(dir)
    if (stat.isFile()) return [relative(this.root, dir)]
    const found: string[] = []
    for (const entry of await readdir(dir, { withFileTypes: true })) {
      const path = join(dir, entry.name)
      if (entry.isDirectory()) found.push(...(await this.files(path)))
      else if (entry.isFile()) found.push(relative(this.root, path))
    }
    return found.sort()
  }
}

function within(root: string, path: string): boolean {
  const rel = relative(root, path)
  return rel === '' || (rel !== '..' && !rel.startsWith(`..${sep}`) && !isAbsolute(rel))
}

function requiredString(args: SearchArguments, key: string): string {
  const value = args[key]
  if (typeof value !== 'string' || value.length === 0) throw new Error(`${key} must be a non-empty string`)
  return value
}

function optionalString(args: SearchArguments, key: string): string | undefined {
  const value = args[key]
  if (value === undefined) return undefined
  if (typeof value !== 'string') throw new Error(`${key} must be a string`)
  return value
}
