import type { EngineDef, Task } from '../../../domain/contracts/index.js'

export type ContextFragmentKind = 'summary' | 'ref' | 'file' | 'snippet' | 'spec-section'

export interface ContextPackSnippet {
  readonly ref: string
  readonly path?: string
  readonly text: string
  readonly content_hash?: string
}

export interface ContextPack {
  readonly summary: string
  readonly refs?: readonly string[]
  readonly files?: readonly string[]
  readonly snippets?: readonly ContextPackSnippet[]
  readonly profile?: string
  readonly engine?: EngineDef
  readonly model_tier?: string
  readonly content_hash?: string
  readonly source?: string
  readonly built_at?: string
}

export interface ContextFragment {
  readonly key: string
  readonly kind: ContextFragmentKind
  readonly source: string
  readonly built_at?: string
  readonly ref?: string
  readonly path?: string
  readonly text: string
  readonly content_hash?: string
}

export interface ContextPackIndex {
  readonly source: string
  readonly built_at?: string
  readonly profile?: string
  readonly fragments: readonly ContextFragment[]
  readonly by_key: Readonly<Record<string, ContextFragment>>
  readonly duplicate_keys: readonly string[]
}

export interface ContextPackStaleness {
  readonly stale: boolean
  readonly reasons: readonly string[]
}

export interface InclusionQuery {
  readonly refs?: readonly string[]
  readonly paths?: readonly string[]
  readonly spec_refs?: readonly string[]
  readonly terms?: readonly string[]
  readonly include_summary?: boolean
}

export interface ContextSlice {
  readonly keys: readonly string[]
  readonly fragments: readonly ContextFragment[]
  readonly summary: string
}

export interface ContextSelectionOptions {
  readonly maxFragments?: number
}

export interface SpecSection {
  readonly ref: string
  readonly title: string
  readonly text: string
}

const DEFAULT_SOURCE = '.council/context/pack.json'

export function indexContextPack(
  pack: ContextPack,
  source = pack.source ?? DEFAULT_SOURCE,
): ContextPackIndex {
  const builtAt = pack.built_at ?? parseBuiltAt(pack.summary)
  const fragments: ContextFragment[] = []
  const byKey: Record<string, ContextFragment> = {}
  const duplicateKeys: string[] = []

  addFragment(fragments, byKey, duplicateKeys, {
    key: 'summary',
    kind: 'summary',
    source,
    text: pack.summary,
    ...optionalStamp(builtAt),
    ...optionalContentHash(pack.content_hash),
  })

  for (const ref of pack.refs ?? []) {
    addFragment(fragments, byKey, duplicateKeys, {
      key: ref,
      kind: 'ref',
      source,
      ref,
      text: ref,
      ...optionalStamp(builtAt),
    })
  }

  for (const file of pack.files ?? []) {
    addFragment(fragments, byKey, duplicateKeys, {
      key: file,
      kind: 'file',
      source,
      path: file,
      text: file,
      ...optionalStamp(builtAt),
    })
  }

  for (const snippet of pack.snippets ?? []) {
    addFragment(fragments, byKey, duplicateKeys, {
      key: snippet.ref,
      kind: 'snippet',
      source,
      ref: snippet.ref,
      text: snippet.text,
      ...optionalStamp(builtAt),
      ...optionalPath(snippet.path),
      ...optionalContentHash(snippet.content_hash),
    })
  }

  return {
    source,
    fragments,
    by_key: byKey,
    duplicate_keys: duplicateKeys,
    ...optionalIndexStamp(builtAt),
    ...optionalProfile(pack.profile),
  }
}

export function checkContextPackStaleness(
  index: Pick<ContextPackIndex, 'built_at' | 'duplicate_keys'>,
  now: Date,
  staleAfterMs: number,
): ContextPackStaleness {
  const reasons: string[] = []

  if (!index.built_at) {
    reasons.push('missing-built-at')
  } else {
    const builtAtMs = Date.parse(index.built_at)
    if (Number.isNaN(builtAtMs)) {
      reasons.push('invalid-built-at')
    } else if (now.getTime() - builtAtMs > staleAfterMs) {
      reasons.push('expired')
    }
  }

  if (index.duplicate_keys.length > 0) {
    reasons.push('duplicate-keys')
  }

  return {
    stale: reasons.length > 0,
    reasons,
  }
}

export function seedContextPackIfAbsent(
  existing: ContextPack | undefined,
  seed: ContextPack,
): ContextPack {
  return existing ?? seed
}

export function createTaskInclusionQuery(
  task: Pick<Task, 'context_refs' | 'paths' | 'spec_ref'>,
): InclusionQuery {
  return {
    paths: task.paths,
    include_summary: true,
    ...(task.context_refs ? { refs: task.context_refs } : {}),
    ...(task.spec_ref ? { spec_refs: [task.spec_ref] } : {}),
  }
}

export function selectContextSlice(
  index: ContextPackIndex,
  task: Pick<Task, 'context_refs' | 'paths' | 'spec_ref'>,
  options: ContextSelectionOptions = {},
): ContextSlice {
  return buildSlice(selectFragments(index.fragments, createTaskInclusionQuery(task), options))
}

export function selectSpecSections(
  sections: readonly SpecSection[],
  query: InclusionQuery,
  options: ContextSelectionOptions = {},
): ContextSlice {
  const fragments = sections.map((section): ContextFragment => ({
    key: section.ref,
    kind: 'spec-section',
    source: 'spec',
    ref: section.ref,
    text: `## ${section.title}\n\n${section.text}`,
  }))

  return buildSlice(selectFragments(fragments, query, options))
}

export function selectFragments(
  fragments: readonly ContextFragment[],
  query: InclusionQuery,
  options: ContextSelectionOptions = {},
): readonly ContextFragment[] {
  const selected = fragments.filter((fragment) => matchesInclusion(fragment, query))
  return typeof options.maxFragments === 'number' ? selected.slice(0, options.maxFragments) : selected
}

export function parseSpecSections(markdown: string): readonly SpecSection[] {
  const sections: SpecSection[] = []
  let currentTitle: string | undefined
  let currentRef: string | undefined
  let currentLines: string[] = []

  for (const line of markdown.split(/\r?\n/u)) {
    const heading = /^(#{1,6})\s+(.+)$/.exec(line)
    if (heading) {
      pushSection(sections, currentRef, currentTitle, currentLines)
      currentTitle = heading[2] ?? ''
      currentRef = titleToRef(currentTitle)
      currentLines = []
    } else {
      currentLines.push(line)
    }
  }

  pushSection(sections, currentRef, currentTitle, currentLines)
  return sections
}

function addFragment(
  fragments: ContextFragment[],
  byKey: Record<string, ContextFragment>,
  duplicateKeys: string[],
  fragment: ContextFragment,
): void {
  const existing = byKey[fragment.key]
  if (existing) {
    const existingIndex = fragments.findIndex((candidate) => candidate.key === fragment.key)
    if (existingIndex >= 0) {
      fragments[existingIndex] = fragment
    }

    if (!isResolvedRefEnrichment(existing, fragment)) {
      duplicateKeys.push(fragment.key)
    }

    byKey[fragment.key] = fragment
    return
  }

  fragments.push(fragment)
  byKey[fragment.key] = fragment
}

function isResolvedRefEnrichment(existing: ContextFragment, fragment: ContextFragment): boolean {
  return (
    existing.kind === 'ref' &&
    fragment.kind === 'snippet' &&
    (Boolean(fragment.path) || Boolean(fragment.content_hash))
  )
}

function optionalStamp(builtAt: string | undefined): Pick<ContextFragment, 'built_at'> | object {
  return builtAt ? { built_at: builtAt } : {}
}

function optionalIndexStamp(builtAt: string | undefined): Pick<ContextPackIndex, 'built_at'> | object {
  return builtAt ? { built_at: builtAt } : {}
}

function optionalProfile(profile: string | undefined): Pick<ContextPackIndex, 'profile'> | object {
  return profile ? { profile } : {}
}

function optionalPath(path: string | undefined): Pick<ContextFragment, 'path'> | object {
  return path ? { path } : {}
}

function optionalContentHash(
  contentHash: string | undefined,
): Pick<ContextFragment, 'content_hash'> | object {
  return contentHash ? { content_hash: contentHash } : {}
}

function buildSlice(fragments: readonly ContextFragment[]): ContextSlice {
  return {
    keys: fragments.map((fragment) => fragment.key),
    fragments,
    summary: fragments.map((fragment) => fragment.text).join('\n\n'),
  }
}

function matchesInclusion(fragment: ContextFragment, query: InclusionQuery): boolean {
  if (query.include_summary && fragment.kind === 'summary') {
    return true
  }

  const refs = normalizeAll(query.refs)
  const paths = normalizeAll(query.paths)
  const specRefs = normalizeAll(query.spec_refs)
  const terms = normalizeAll(query.terms)
  const fragmentKey = normalize(fragment.key)
  const fragmentRef = normalize(fragment.ref)
  const fragmentPath = normalize(fragment.path)
  const fragmentText = normalize(fragment.text)

  return (
    contains(refs, fragmentKey) ||
    contains(refs, fragmentRef) ||
    contains(specRefs, fragmentKey) ||
    contains(specRefs, fragmentRef) ||
    pathIntersects(paths, fragmentPath) ||
    containsTerm(terms, fragmentText)
  )
}

function contains(values: readonly string[], candidate: string): boolean {
  return candidate.length > 0 && values.includes(candidate)
}

function pathIntersects(paths: readonly string[], candidate: string): boolean {
  return (
    candidate.length > 0 &&
    paths.some(
      (path) => path === candidate || path.startsWith(`${candidate}/`) || candidate.startsWith(`${path}/`),
    )
  )
}

function containsTerm(terms: readonly string[], text: string): boolean {
  return text.length > 0 && terms.some((term) => term.length > 0 && text.includes(term))
}

function normalizeAll(values: readonly string[] | undefined): readonly string[] {
  return (values ?? []).map(normalize).filter(Boolean)
}

function normalize(value: string | undefined): string {
  return value?.trim().toLowerCase() ?? ''
}

function parseBuiltAt(summary: string): string | undefined {
  return /\bBuilt at\s+([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z)\b/u.exec(summary)?.[1]
}

function titleToRef(title: string): string {
  return title
    .trim()
    .toLowerCase()
    .replace(/[^a-z0-9]+/gu, '-')
    .replace(/^-|-$/gu, '')
}

function pushSection(
  sections: SpecSection[],
  ref: string | undefined,
  title: string | undefined,
  lines: readonly string[],
): void {
  if (ref && title) {
    sections.push({
      ref,
      title,
      text: lines.join('\n').trim(),
    })
  }
}
