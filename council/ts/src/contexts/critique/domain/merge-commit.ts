/** A manifest's sha256 pin line, as `manifest.yaml` writes it; the digest is what a rebuild changes. */
const PIN = /^(\s*(?:-\s+)?sha256:\s*)[0-9a-f]{64}(\s*)$/

/** Whether a path names a manifest whose sha256 pins a merge may rewrite. */
export function isManifest(path: string): boolean {
  return path === 'manifest.yaml' || path.endsWith('/manifest.yaml')
}

/**
 * Whether a merge's version of a manifest differs from the clean
 * auto-merge's only in sha256 pins. The auto-merge may hold conflicts, in
 * git's default style: a conflict whose two sides differ only in their pins
 * counts as resolved by either. Every other line must be the same, so an
 * entry added, dropped or edited by hand is not a pin.
 */
export function differsOnlyInPins(autoMerged: string, merged: string): boolean {
  const resolved = resolvePinConflicts(masked(autoMerged))
  return resolved !== null && resolved.join('\n') === masked(merged).join('\n')
}

/** The lines of a text, each pin's digest replaced by the same placeholder. */
function masked(text: string): readonly string[] {
  return text.split('\n').map((line) => line.replace(PIN, '$1<pin>$2'))
}

/** The lines with each conflict replaced by its one side, when both sides are the same; null when any conflict's sides differ. */
function resolvePinConflicts(lines: readonly string[]): readonly string[] | null {
  const out: string[] = []
  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] ?? ''
    if (!line.startsWith('<<<<<<<')) {
      out.push(line)
      continue
    }
    const middle = lines.indexOf('=======', i + 1)
    const end = lines.findIndex((candidate, n) => n > middle && candidate.startsWith('>>>>>>>'))
    const ours = lines.slice(i + 1, middle)
    if (middle === -1 || end === -1 || ours.join('\n') !== lines.slice(middle + 1, end).join('\n')) return null
    out.push(...ours)
    i = end
  }
  return out
}

/** The sections of a unified diff (`diffCommits`' form) for the files `paths` names, a rename's section by either of its paths. */
export function diffOfPaths(diff: string, paths: readonly string[]): string {
  const sections = diff.split(/^(?=diff --git )/m)
  return sections
    .filter((section) => {
      const header = section.split('\n', 1)[0] ?? ''
      return paths.some((path) => header.startsWith(`diff --git a/${path} `) || header.endsWith(` b/${path}`))
    })
    .join('')
}
