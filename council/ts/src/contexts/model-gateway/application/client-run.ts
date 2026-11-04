import { join } from 'node:path'

import type { ProcessCommand, ProcessResult } from '../../../ports/index.js'
import { ModelGatewayError } from '../domain/config.js'
import { EXTRA_NAMES, type ExtraName, type RunExtras } from '../domain/extras.js'
import type { ClientOutput } from '../domain/outputs.js'
import type { RunClient } from '../domain/run.js'
import { homeMcpServers, type HomePorts } from './home-mcp.js'
import { LAUNCHERS, type LaunchExtras, type LaunchPaths } from './launchers.js'

/** What one client launch needs: the process it starts, the files it reads, and the home it reads a client's servers from. */
export interface ClientRunPorts extends HomePorts {
  readonly process: { exec(command: ProcessCommand): Promise<ProcessResult> }
}

/** One client launch, as a caller inside this process asks for it: its own prompt, its own directory, its own extras. */
export interface ClientRunRequest {
  readonly binary: string
  readonly client: RunClient
  readonly model: string
  /** The prompt text, which the client reads from `paths.promptPath` or from its arguments. */
  readonly prompt: string
  readonly paths: LaunchPaths
  readonly time_limit_s: number
  readonly max_price_usd?: number
  /** What the caller adds to the client's command; `{}` when it adds nothing. */
  readonly extras: RunExtras
}

/** What one client launch came to: the process's own result, what it exported, and what its output says. */
export interface ClientRunOutcome {
  readonly result: ProcessResult
  /** Parsed `export.json`, or undefined when the client wrote none or it does not parse. */
  readonly exported: unknown
  readonly output: ClientOutput
}

/**
 * Refuses an extra the client cannot apply, naming the extra and the client,
 * and an `extras.env` variable the launcher sets itself, naming the variable.
 * A caller that needs a server attached, a home switched off or a sandbox
 * profile gets a refusal rather than a run that quietly drops it, and one that
 * tries to lift the launcher's own isolation is refused rather than obeyed.
 */
export function requireSupportedExtras(client: RunClient, extras: RunExtras): void {
  const launcher = LAUNCHERS[client]
  const unsupported = EXTRA_NAMES.find((name: ExtraName) => extras[name] !== undefined && !launcher.extras.includes(name))
  if (unsupported !== undefined) {
    throw new ModelGatewayError('extras-not-supported', `${client} cannot apply the ${unsupported} extra`)
  }
  const protectedName = Object.keys(extras.env ?? {}).find((name) => launcher.protectedEnv.includes(name))
  if (protectedName !== undefined) {
    throw new ModelGatewayError('extras-not-supported', `${client} sets ${protectedName} itself, so extras.env may not name it`)
  }
}

/**
 * The gateway's launch path, for a caller inside this process: it builds the
 * client's command with the caller's extras, runs it, and reads what the
 * client exported. A caller that parses its own output, as critique does,
 * reads `result` and `exported`; a delegation reads `output` too.
 */
export class ClientRun {
  constructor(private readonly ports: ClientRunPorts) {}

  async launch(request: ClientRunRequest): Promise<ClientRunOutcome> {
    requireSupportedExtras(request.client, request.extras)
    const launcher = LAUNCHERS[request.client]
    const extras = await this.extrasOf(request)
    const result = await this.ports.process.exec(launcher.command(request.binary, request, request.paths, request.prompt, extras))
    const exported = launcher.writesExport ? await this.readExport(join(request.paths.exportDir, 'export.json')) : undefined
    return { exported, output: launcher.read(result.stdout, exported), result }
  }

  /** The caller's extras as the launcher applies them, with the client's home servers resolved. */
  private async extrasOf(request: ClientRunRequest): Promise<LaunchExtras> {
    const extras = request.extras
    return {
      env: extras.env ?? {},
      homeServers: extras.disable_home_mcp === true ? await homeMcpServers(this.ports) : [],
      mcp_servers: extras.mcp_servers ?? [],
      tools: extras.tools ?? [],
      ...(extras.agent === undefined ? {} : { agent: extras.agent }),
      ...(extras.sandbox === undefined ? {} : { sandbox: extras.sandbox }),
      ...(extras.schema === undefined ? {} : { schema: extras.schema }),
    }
  }

  /** Parsed `export.json`, or undefined when the client wrote none or it does not parse. */
  private async readExport(path: string): Promise<unknown> {
    if (!(await this.ports.files.exists(path))) return undefined
    try {
      return JSON.parse(await this.ports.files.readText(path)) as unknown
    } catch {
      return undefined
    }
  }
}
