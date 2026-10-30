/** One remote of a repository, as `git config` names it. */
export interface GitRemote {
  readonly name: string
  readonly url: string
}
