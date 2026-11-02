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
  const prose = withoutFences(markdown)
  const candidates = [...prose.matchAll(/`([^`\n]+)`|\]\(([^)\s]+)\)/g)].map((match) => match[1] ?? match[2] ?? '')
  const paths = candidates
    .map((candidate) => candidate.trim().replace(/^\.\//, '').replace(/\/+$/, ''))
    .filter((candidate) => /^[\w.-]+(\/[\w.-]+)*$/.test(candidate) && !candidate.split('/').includes('..'))
  return [...new Set(paths)]
}

/** The docs that define words rather than record decisions, so no status can leave them out. */
export const NOT_RECORDS: readonly string[] = [...GLOSSARIES, CONTEXT_MAP]

/** The statuses that take a record out of decision evidence. No status, or any other, counts as accepted. */
const NOT_IN_FORCE: readonly string[] = ['proposed', 'rejected', 'superseded', 'deprecated']

/**
 * True unless the record says it is proposed, rejected, superseded or
 * deprecated. The status is the first word of a `status:` key in YAML
 * frontmatter, else of a `Status:` line (bold or a list item allowed), else
 * of the first line under a `## Status` heading, outside fenced blocks. A
 * record with none of these, as agent-kit's own ADRs, is in force.
 */
export function inForce(record: string): boolean {
  const status = recordStatus(record)
  return status === undefined || !NOT_IN_FORCE.includes(status)
}

function recordStatus(record: string): string | undefined {
  const frontmatter = /^---\r?\n([\s\S]*?)\r?\n---/.exec(record)?.[1] ?? ''
  const prose = withoutFences(record)
  const match =
    /^status[ \t]*:[ \t]*["']?([A-Za-z]+)/im.exec(frontmatter) ??
    /^(?:[-*+][ \t]+)?(?:\*\*|__)?status(?:\*\*|__)?[ \t]*:[ \t]*(?:\*\*|__)?[ \t]*([A-Za-z]+)/im.exec(prose) ??
    /^#{1,6}[ \t]+status[ \t]*\r?\n\s*[^A-Za-z\n]*([A-Za-z]+)/im.exec(prose)
  return match?.[1]?.toLowerCase()
}

function withoutFences(markdown: string): string {
  return markdown.replace(/^(```|~~~)[^\n]*\n[\s\S]*?^\1[^\n]*$/gm, '')
}
