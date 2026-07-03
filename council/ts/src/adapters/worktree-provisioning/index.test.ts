import { mkdtemp, readFile, lstat, mkdir, symlink, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { join, resolve } from 'node:path'

import { describe, expect, it } from 'vitest'

import {
  CopyNodeModulesWorktreeDependencyProvisioner,
  createWorktreeDependencyProvisioner,
  type WorktreeDependencyProvisionerPort,
} from './index.js'

const makeSandbox = async (): Promise<{ repoRoot: string; worktreePath: string }> => {
  const root = await mkdtemp(join(tmpdir(), 'council-worktree-provisioning-'))
  const repoRoot = join(root, 'repo')
  const worktreePath = join(root, 'worktree')
  await mkdir(repoRoot)
  await mkdir(worktreePath)
  return { repoRoot, worktreePath }
}

describe('createWorktreeDependencyProvisioner', () => {
  it('uses the copy strategy by default', async () => {
    const { repoRoot, worktreePath } = await makeSandbox()
    const packagePath = join(repoRoot, 'node_modules', 'package-a')
    await mkdir(packagePath, { recursive: true })
    await symlink(resolve(packagePath), join(repoRoot, 'node_modules', 'package-a-link'))

    const result = await createWorktreeDependencyProvisioner().provision({
      repoRoot,
      worktreePath,
    })

    expect(result).toEqual({
      destination: join(worktreePath, 'node_modules'),
      source: join(repoRoot, 'node_modules'),
      status: 'copied',
      strategy: 'copy',
    })
    const destination = await lstat(join(worktreePath, 'node_modules'))
    expect(destination.isDirectory()).toBe(true)
    expect(destination.isSymbolicLink()).toBe(false)
  })

  it('can skip dependency provisioning explicitly', async () => {
    const { repoRoot, worktreePath } = await makeSandbox()

    const result = await createWorktreeDependencyProvisioner({ strategy: 'none' }).provision({
      repoRoot,
      worktreePath,
    })

    expect(result).toEqual({
      reason: 'disabled',
      status: 'skipped',
      strategy: 'none',
    })
    await expect(lstat(join(worktreePath, 'node_modules'))).rejects.toMatchObject({
      code: 'ENOENT',
    })
  })

  it('can delegate to an injected custom provisioner', async () => {
    const { repoRoot, worktreePath } = await makeSandbox()
    const provisioner: WorktreeDependencyProvisionerPort = {
      provision(request) {
        return Promise.resolve({
          metadata: {
            repoRoot: request.repoRoot,
            worktreePath: request.worktreePath,
          },
          status: 'skipped',
          strategy: 'custom',
        })
      },
    }

    await expect(
      createWorktreeDependencyProvisioner({ provisioner, strategy: 'custom' }).provision({
        repoRoot,
        worktreePath,
      }),
    ).resolves.toEqual({
      metadata: {
        repoRoot,
        worktreePath,
      },
      status: 'skipped',
      strategy: 'custom',
    })
  })
})

describe('CopyNodeModulesWorktreeDependencyProvisioner', () => {
  it('copies repo-root node_modules into the worktree without creating a node_modules symlink', async () => {
    const { repoRoot, worktreePath } = await makeSandbox()
    const sourcePackagePath = join(repoRoot, 'node_modules', 'package-a')
    await mkdir(sourcePackagePath, { recursive: true })
    await mkdir(join(sourcePackagePath, 'nested'))
    await writeFile(join(sourcePackagePath, 'nested', 'index.js'), 'export const value = 1\n')

    const result = await new CopyNodeModulesWorktreeDependencyProvisioner().provision({
      repoRoot,
      worktreePath,
    })

    expect(result.status).toBe('copied')
    await expect(
      readFile(join(worktreePath, 'node_modules', 'package-a', 'nested', 'index.js'), 'utf8'),
    ).resolves.toBe('export const value = 1\n')
    const destination = await lstat(join(worktreePath, 'node_modules'))
    expect(destination.isDirectory()).toBe(true)
    expect(destination.isSymbolicLink()).toBe(false)
  })

  it('reports a missing repo-root node_modules directory explicitly', async () => {
    const { repoRoot, worktreePath } = await makeSandbox()

    await expect(
      new CopyNodeModulesWorktreeDependencyProvisioner().provision({
        repoRoot,
        worktreePath,
      }),
    ).resolves.toEqual({
      destination: join(worktreePath, 'node_modules'),
      reason: 'source-missing',
      source: join(repoRoot, 'node_modules'),
      status: 'skipped',
      strategy: 'copy',
    })
    await expect(lstat(join(worktreePath, 'node_modules'))).rejects.toMatchObject({
      code: 'ENOENT',
    })
  })
})
