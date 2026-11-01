/**
 * Which decision docs a spec is checked against, read the way Pocock's
 * `setup-matt-pocock-skills` lays them out.
 */

/** The doc Pocock's setup writes to say where a repository's domain docs live. */
export const DOMAIN_DOC = 'docs/agents/domain.md'

/** Tried in order when there is no domain doc; the first that exists is the glossary. */
export const GLOSSARIES = ['GLOSSARY.md', 'CONTEXT.md'] as const

/** Where the ADRs live when there is no domain doc. */
export const ADR_DIRECTORY = 'docs/adr'

/** The file Pocock's multi-context layout uses to point at one `CONTEXT.md` per context. */
export const CONTEXT_MAP = 'CONTEXT-MAP.md'

/**
 * The repository paths a markdown doc names, in order and without repeats:
 * every inline code span and link target outside fenced blocks that reads as
 * a relative path. Placeholders such as `src/<context>/docs/adr/`, absolute
 * paths, `..` and skill names such as `/domain-modeling` are left out.
 */
export function namedPaths(markdown: string): readonly string[] {
  const prose = markdown.replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, '')
  const candidates = [...prose.matchAll(/`([^`\n]+)`|\]\(([^)\s]+)\)/g)].map((match) => match[1] ?? match[2] ?? '')
  const paths = candidates
    .map((candidate) => candidate.trim().replace(/^\.\//, '').replace(/\/+$/, ''))
    .filter((candidate) => /^[\w.-]+(\/[\w.-]+)*$/.test(candidate) && !candidate.split('/').includes('..'))
  return [...new Set(paths)]
}
