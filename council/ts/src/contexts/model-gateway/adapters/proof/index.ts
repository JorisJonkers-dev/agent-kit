import type { EnvPort } from '../../../../ports/index.js'
import { routingOverridePath, setupResultsPath } from '../../application/locations.js'
import type { ModelGatewayFilesPort, ModelGatewayProofPort } from '../../application/ports.js'
import type { ProofFiles } from '../../domain/run.js'

/** Where the read-only proofs are read from: the shipped routing, this machine's override, and setup's results. */
export interface ProofPaths {
  readonly env: EnvPort
  readonly files: ModelGatewayFilesPort
  /** Critique's shipped routing, beside the critique skill's runner. */
  readonly routingPath: string
}

/**
 * Reads what setup and critique left on this machine. It reads files only: it
 * runs no client and spends nothing, so a run can be refused before it
 * launches.
 */
export class NodeModelGatewayProof implements ModelGatewayProofPort {
  constructor(private readonly paths: ProofPaths) {}

  async read(): Promise<ProofFiles> {
    const [routing, routingOverride, setupResults] = await Promise.all([
      this.file(this.paths.routingPath),
      this.file(routingOverridePath(this.paths.env)),
      this.file(setupResultsPath(this.paths.env)),
    ])
    return { routing, routingOverride, setupResults }
  }

  /** The file's text, or undefined when it is not there. */
  private async file(path: string): Promise<string | undefined> {
    return (await this.paths.files.exists(path)) ? this.paths.files.readText(path) : undefined
  }
}
