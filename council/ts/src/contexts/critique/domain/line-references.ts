/** A file and line, or range of lines, that a finding cites, as `path:line` or `path:first-last`. */
export interface LineReference {
  /** The path as the critic wrote it, without a leading `./`. */
  readonly path: string
  readonly first: number
  readonly last: number
}

const URL = /\b[a-z][a-z0-9+.-]*:\/\/\S*/gi
const REFERENCE = /(?<![\w./@+-])((?:\.\/)?(?:[\w@+.-]+\/)*[\w@+.-]+):(\d+)(?:\s*[-–]\s*(\d+))?/g
/** A path counts only when it has a directory or a file extension, so `10:30` is a time, not a file. */
const LOOKS_LIKE_A_FILE = /\/|\.[A-Za-z]\w*$/

/**
 * Every `path:line` and `path:first-last` in a location or a trace step. A
 * URL is not a file, so `https://example.com:443` cites nothing.
 */
export function lineReferences(text: string): readonly LineReference[] {
  return [...text.replace(URL, ' ').matchAll(REFERENCE)].flatMap((match) => {
    const path = (match[1] ?? '').replace(/^\.\//, '')
    if (!LOOKS_LIKE_A_FILE.test(path)) return []
    const first = Number(match[2])
    return [{ first, last: match[3] === undefined ? first : Number(match[3]), path }]
  })
}

/**
 * The packet paths a cited path may mean, most literal first: the packet
 * path itself, the repository path under `tree/`, and a path with a diff's
 * `a/` or `b/` prefix under `tree/`.
 */
export function packetCandidates(path: string, treeDir: string): readonly string[] {
  const diffSide = /^[ab]\/(.+)$/.exec(path)
  return [path, `${treeDir}/${path}`, ...(diffSide === null ? [] : [`${treeDir}/${diffSide[1] ?? ''}`])]
}

/** How many lines a file has: a final newline ends the last line rather than starting another. */
export function lineCount(text: string): number {
  if (text.length === 0) return 0
  return text.split('\n').length - (text.endsWith('\n') ? 1 : 0)
}
