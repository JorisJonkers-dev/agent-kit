/**
 * What a caller inside this process adds to one run's client command. The
 * gateway holds no caller's rules: it applies these to the command, and
 * refuses the ones a client cannot apply. A caller that starts its own MCP
 * server, switches a client's home servers off, or needs a sandboxed client
 * says so here. Only an in-process caller sends them: the command line's
 * `run` refuses the field.
 */

/** One MCP server a run attaches to its client. */
export interface AttachedServer {
  readonly name: string
  readonly command: string
  readonly args: readonly string[]
}

/** The sandbox profiles a launcher maps to its client's own flags. */
export type SandboxProfile = 'read-only'

/** Every extra a run may name, in the order a refusal lists them. */
export const EXTRA_NAMES = ['agent', 'disable_home_mcp', 'env', 'mcp_servers', 'sandbox', 'schema', 'tools'] as const
export type ExtraName = (typeof EXTRA_NAMES)[number]

/** What a caller adds to one run, every part of it optional. */
export interface RunExtras {
  /** The agent the client runs, where it names one (Vibe's `--agent`); the model is named beside it. */
  readonly agent?: string
  /** Switch off every MCP server the client's home declares, so only the attached ones can start. */
  readonly disable_home_mcp?: boolean
  /** Extra environment for the client's process, under the launcher's own isolation, which wins. */
  readonly env?: Readonly<Record<string, string>>
  /** The MCP servers to attach to the client. */
  readonly mcp_servers?: readonly AttachedServer[]
  /** The sandbox profile the launcher maps to the client's own flags. */
  readonly sandbox?: SandboxProfile
  /** A JSON schema the client enforces on its structured output, where it can. */
  readonly schema?: string
  /** The tools the run may use; a client enables these and denies the rest. */
  readonly tools?: readonly string[]
}
