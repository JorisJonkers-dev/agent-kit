import { describe, expect, it } from 'vitest'

import type { ModelGatewayDependencies } from './ports.js'
import { WorktreesCleanCommand } from './worktrees.js'

/** The command on its own: the CLI rejects input that is not an object first, but other callers reach it directly. */
describe('WorktreesCleanCommand', () => {
  it('refuses input that is not a JSON object', async () => {
    const command = new WorktreesCleanCommand({ kept: { list: () => Promise.resolve([]) } } as unknown as ModelGatewayDependencies)

    await expect(command.run('spec-294')).rejects.toMatchObject({ code: 'invalid-input', message: 'the input must be a JSON object' })
  })
})
