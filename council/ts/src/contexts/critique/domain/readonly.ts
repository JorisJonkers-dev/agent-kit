/**
 * The third layer of read-only enforcement compares what a critic could have
 * written to before and after it ran. A packet's state maps each file to a
 * hash of its content; a file whose hash differs, appears or disappears is a
 * change.
 */
export type PacketState = ReadonlyMap<string, string>

/** Every change between two states of a packet, as `<packet>: <added|changed|removed> <path>`, by path. */
export function packetChanges(packet: string, before: PacketState, after: PacketState): readonly string[] {
  return changedPaths(before, after).map((path) => {
    const change = !before.has(path) ? 'added' : !after.has(path) ? 'removed' : 'changed'
    return `${packet}: ${change} ${path}`
  })
}

/** One path `git status` lists: its status code, and a hash of what is at the path now. */
export interface WorkingTreeEntry {
  readonly code: string
  readonly hash: string
}

/** The working tree as `git status` sees it: only the paths it lists. */
export type WorkingTreeState = ReadonlyMap<string, WorkingTreeEntry>

/**
 * Every path whose status or content changed, as `working tree: <status> <path>`
 * with the status after the critic ran (`clean` when git no longer lists it).
 */
export function workingTreeChanges(before: WorkingTreeState, after: WorkingTreeState): readonly string[] {
  const same = (a: WorkingTreeEntry | undefined, b: WorkingTreeEntry | undefined) => a?.code === b?.code && a?.hash === b?.hash
  return changedPaths(before, after, same).map((path) => `working tree: ${after.get(path)?.code.trim() ?? 'clean'} ${path}`)
}

function changedPaths<V>(
  before: ReadonlyMap<string, V>,
  after: ReadonlyMap<string, V>,
  same: (a: V | undefined, b: V | undefined) => boolean = (a, b) => a === b,
): readonly string[] {
  return [...new Set([...before.keys(), ...after.keys()])].sort().filter((path) => !same(before.get(path), after.get(path)))
}
