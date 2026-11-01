import { join } from 'node:path'

import { isJsonObject } from '../domain/json.js'
import { mergePatch } from '../domain/merge-patch.js'
import { CritiqueError } from '../domain/outcomes.js'
import { parseRouting, type Routing } from '../domain/routing.js'
import { decoded, readJsonFile } from './decode.js'
import { routingOverridePath } from './locations.js'
import type { CritiqueDeps } from './ports.js'

/**
 * The routing a run uses: the routing agent-kit ships, with this machine's
 * override applied key by key when there is one. The override is a JSON
 * merge patch, so it names only what it changes, and `null` removes a key.
 * The merged routing is checked as a whole, so an override cannot leave it
 * malformed.
 */
export class RoutingSource {
  constructor(private readonly deps: Pick<CritiqueDeps, 'assetsRoot' | 'env' | 'files'>) {}

  async load(): Promise<Routing> {
    const shippedPath = join(this.deps.assetsRoot, 'routing.json')
    const shipped = await readJsonFile(this.deps.files, shippedPath, 'invalid-routing', (text) => JSON.parse(text) as unknown)
    const overridePath = routingOverridePath(this.deps.env)
    if (!(await this.deps.files.exists(overridePath))) {
      return decoded('invalid-routing', shippedPath, () => parseRouting(JSON.stringify(shipped)))
    }
    const override = await readJsonFile(this.deps.files, overridePath, 'invalid-routing', (text) => JSON.parse(text) as unknown)
    if (!isJsonObject(override)) throw new CritiqueError('invalid-routing', `${overridePath}: the override must be a JSON object`)
    return decoded('invalid-routing', `${shippedPath} with the override ${overridePath}`, () =>
      parseRouting(JSON.stringify(mergePatch(shipped, override))),
    )
  }
}
