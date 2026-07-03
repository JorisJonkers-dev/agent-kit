import { cp, mkdir, stat } from 'node:fs/promises'
import { join } from 'node:path'

export type WorktreeDependencyProvisioningStrategy = 'copy' | 'custom' | 'none'

export type WorktreeDependencyProvisioningStatus = 'copied' | 'skipped'

export type WorktreeDependencyProvisioningSkipReason = 'disabled' | 'source-missing'

export interface WorktreeDependencyProvisioningRequest {
  readonly repoRoot: string
  readonly worktreePath: string
}

export interface WorktreeDependencyProvisioningResult {
  readonly strategy: WorktreeDependencyProvisioningStrategy
  readonly status: WorktreeDependencyProvisioningStatus
  readonly reason?: WorktreeDependencyProvisioningSkipReason
  readonly source?: string
  readonly destination?: string
  readonly metadata?: Readonly<Record<string, string>>
}

export interface WorktreeDependencyProvisionerPort {
  provision(
    request: WorktreeDependencyProvisioningRequest,
  ): Promise<WorktreeDependencyProvisioningResult>
}

export type WorktreeDependencyProvisionerOptions =
  | { readonly strategy?: 'copy' }
  | { readonly strategy: 'none' }
  | {
      readonly strategy: 'custom'
      readonly provisioner: WorktreeDependencyProvisionerPort
    }

export class CopyNodeModulesWorktreeDependencyProvisioner
  implements WorktreeDependencyProvisionerPort
{
  async provision(
    request: WorktreeDependencyProvisioningRequest,
  ): Promise<WorktreeDependencyProvisioningResult> {
    const source = join(request.repoRoot, 'node_modules')
    const destination = join(request.worktreePath, 'node_modules')

    if (!(await pathExists(source))) {
      return {
        destination,
        reason: 'source-missing',
        source,
        status: 'skipped',
        strategy: 'copy',
      }
    }

    await mkdir(request.worktreePath, { recursive: true })
    await cp(source, destination, {
      dereference: true,
      force: true,
      recursive: true,
    })

    return {
      destination,
      source,
      status: 'copied',
      strategy: 'copy',
    }
  }
}

export class NoopWorktreeDependencyProvisioner implements WorktreeDependencyProvisionerPort {
  provision(): Promise<WorktreeDependencyProvisioningResult> {
    return Promise.resolve({
      reason: 'disabled',
      status: 'skipped',
      strategy: 'none',
    })
  }
}

export const createWorktreeDependencyProvisioner = (
  options: WorktreeDependencyProvisionerOptions = {},
): WorktreeDependencyProvisionerPort => {
  if (options.strategy === 'custom') {
    return options.provisioner
  }

  if (options.strategy === 'none') {
    return new NoopWorktreeDependencyProvisioner()
  }

  return new CopyNodeModulesWorktreeDependencyProvisioner()
}

const pathExists = async (path: string): Promise<boolean> => {
  try {
    await stat(path)
    return true
  } catch {
    return false
  }
}
