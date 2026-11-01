import { join } from 'node:path'

import { sha256, type RunManifest } from '../domain/manifest.js'
import { CritiqueError } from '../domain/outcomes.js'
import {
  packetChanges,
  workingTreeChanges,
  type PacketState,
  type WorkingTreeEntry,
  type WorkingTreeState,
} from '../domain/readonly.js'
import type { CritiqueDeps } from './ports.js'

/** One critic's tripwire, set before it starts. */
export interface ArmedTripwire {
  /**
   * Compares everything the critic could have written to with its state when
   * the tripwire was set. On any change it restores the packet, and returns
   * the changes; it returns none when the critic left everything as it was.
   */
  check(): Promise<readonly string[]>
}

interface States {
  readonly packet: PacketState
  readonly runPacket: PacketState
  readonly workingTree: WorkingTreeState
}

/**
 * The third layer of read-only enforcement (#167). Each critic works in its
 * own copy of the packet, so a critic that writes to its packet is caught
 * without voiding the others running beside it. Around each critic the
 * tripwire also watches the run's packet, which every copy is made from, and
 * the repository's working tree; a change to either during a critic's run
 * voids that critic, and every other critic running at the time, since the
 * runner cannot tell which of them made it.
 */
export class ReadOnlyTripwire {
  private constructor(
    private readonly deps: Pick<CritiqueDeps, 'files' | 'git'>,
    private readonly manifest: RunManifest,
    /** The packet as prepare froze it, path by path. */
    private readonly frozen: ReadonlyMap<string, string>,
  ) {}

  /** Reads the run's packet, which must still be what prepare froze. */
  static async load(deps: Pick<CritiqueDeps, 'files' | 'git'>, manifest: RunManifest): Promise<ReadOnlyTripwire> {
    const frozen = new Map<string, string>()
    for (const file of manifest.packet_files) {
      const text = await deps.files.readText(join(manifest.packet_dir, file.path))
      if (sha256(text) !== file.sha256) {
        throw new CritiqueError('invalid-run', `${file.path} in the packet no longer matches the manifest`)
      }
      frozen.set(file.path, text)
    }
    return new ReadOnlyTripwire(deps, manifest, frozen)
  }

  /** Writes the critic's copy of the packet to `packetDir` and records the state it starts from. */
  async arm(packetDir: string): Promise<ArmedTripwire> {
    await this.restore(packetDir)
    const before = await this.states(packetDir)
    return {
      check: async () => {
        const after = await this.states(packetDir)
        const changes = [
          ...packetChanges('packet', before.packet, after.packet),
          ...packetChanges('run packet', before.runPacket, after.runPacket),
          ...workingTreeChanges(before.workingTree, after.workingTree),
        ]
        if (changes.length > 0) {
          await this.restore(packetDir)
          await this.restore(this.manifest.packet_dir)
        }
        return changes
      },
    }
  }

  private async states(packetDir: string): Promise<States> {
    return {
      packet: await this.packetState(packetDir),
      runPacket: await this.packetState(this.manifest.packet_dir),
      workingTree: await this.workingTreeState(),
    }
  }

  private async packetState(dir: string): Promise<PacketState> {
    const state = new Map<string, string>()
    for (const path of await this.deps.files.listFiles(dir)) {
      state.set(path, await this.fingerprint(join(dir, path)))
    }
    return state
  }

  /**
   * The porcelain status, plus a hash of each listed file, so a critic that
   * edits a file the owner had already changed is caught too.
   */
  private async workingTreeState(): Promise<WorkingTreeState> {
    const root = this.manifest.repo_root
    const state = new Map<string, WorkingTreeEntry>()
    for (const entry of await this.deps.git.porcelainStatus(root)) {
      state.set(entry.path, { code: entry.code, hash: await this.fingerprint(join(root, entry.path)) })
    }
    return state
  }

  private async fingerprint(path: string): Promise<string> {
    return this.deps.files.readBytes(path).then(sha256, () => 'unreadable')
  }

  /** Puts back every frozen file and removes every other file. */
  private async restore(dir: string): Promise<void> {
    for (const path of await this.deps.files.listFiles(dir)) {
      if (!this.frozen.has(path)) await this.deps.files.remove(join(dir, path))
    }
    for (const [path, text] of this.frozen) await this.deps.files.writeText(join(dir, path), text)
  }
}
