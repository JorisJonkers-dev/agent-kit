import type { ModelGatewayGitPort, ModelGatewayProcessPort } from '../../src/contexts/model-gateway/index.js'
import type { EnvPort, ProcessCommand, ProcessResult } from '../../src/ports/index.js'
import type { GitRemote } from '../../src/shared-kernel/index.js'

/** The environment a gateway test gives it: exactly the variables it names. */
export class FakeEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.get(name)
    if (value === undefined) throw new Error(`required environment variable is missing: ${name}`)
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

/** A PATH that holds exactly the programs the test names, and the listings they answer. */
export class FakeProcess implements ModelGatewayProcessPort {
  readonly asked: string[] = []
  readonly ran: ProcessCommand[] = []

  constructor(
    private readonly installed: readonly string[],
    private readonly listings: Readonly<Record<string, ProcessResult>> = {},
  ) {}

  which(binary: string): Promise<boolean> {
    this.asked.push(binary)
    return Promise.resolve(this.installed.includes(binary))
  }

  exec(command: ProcessCommand): Promise<ProcessResult> {
    this.ran.push(command)
    const result = this.listings[command.command]
    if (result === undefined) return Promise.reject(new Error(`cannot run ${command.command}`))
    return Promise.resolve(result)
  }
}

/**
 * One path the fake git knows: a repository with its root and its remotes, or
 * a path git cannot read, with the message it fails with.
 */
export type FakeRepository =
  | { readonly root: string; readonly remotes: readonly GitRemote[] }
  | { readonly error: string }

/** A git that knows exactly the paths the test names; any other path is in no repository. */
export class FakeGit implements ModelGatewayGitPort {
  constructor(private readonly repositories: Readonly<Record<string, FakeRepository>>) {}

  root(path: string): Promise<string | undefined> {
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.resolve(undefined)
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    return Promise.resolve(entry.root)
  }

  remotes(path: string): Promise<readonly GitRemote[]> {
    const entry = this.repositories[path]
    if (entry === undefined) return Promise.resolve([])
    if ('error' in entry) return Promise.reject(new Error(entry.error))
    return Promise.resolve(entry.remotes)
  }
}
