/**
 * The repository's files ranked by code-nav's map, most referenced first, as
 * repository paths; or why there is no ranking, which a packet records
 * instead of failing.
 */
export type RepoRanking =
  | { readonly status: 'ranked'; readonly paths: readonly string[] }
  | { readonly status: 'unavailable'; readonly detail: string }

/** What a packet's facts index says about how code-nav's map shaped the tree sample. */
export function rankingNote(ranking: RepoRanking): string {
  return ranking.status === 'ranked'
    ? `code-nav's repository map ranked ${String(ranking.paths.length)} files by how often their symbols are referenced; the tree sample takes those files first within their tier, in that order.`
    : `code-nav's repository map was not used (${ranking.detail}), so the tree sample keeps its path order.`
}

/** The ranked paths, or none when there is no ranking. */
export function rankedPaths(ranking: RepoRanking | undefined): readonly string[] {
  return ranking?.status === 'ranked' ? ranking.paths : []
}

/** A path's place in the ranking, or past every ranked path. */
export function rankIndex(ranked: readonly string[]): (path: string) => number {
  const places = new Map(ranked.map((path, place) => [path, place] as const))
  return (path) => places.get(path) ?? Infinity
}
