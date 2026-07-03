const DEFAULT_SOURCE = '.council/context/pack.json';
export function indexContextPack(pack, source = pack.source ?? DEFAULT_SOURCE) {
    const builtAt = pack.built_at ?? parseBuiltAt(pack.summary);
    const fragments = [];
    const byKey = {};
    const duplicateKeys = [];
    addFragment(fragments, byKey, duplicateKeys, {
        key: 'summary',
        kind: 'summary',
        source,
        text: pack.summary,
        ...optionalStamp(builtAt),
        ...optionalContentHash(pack.content_hash),
    });
    for (const ref of pack.refs ?? []) {
        addFragment(fragments, byKey, duplicateKeys, {
            key: ref,
            kind: 'ref',
            source,
            ref,
            text: ref,
            ...optionalStamp(builtAt),
        });
    }
    for (const file of pack.files ?? []) {
        addFragment(fragments, byKey, duplicateKeys, {
            key: file,
            kind: 'file',
            source,
            path: file,
            text: file,
            ...optionalStamp(builtAt),
        });
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
        });
    }
    return {
        source,
        fragments,
        by_key: byKey,
        duplicate_keys: duplicateKeys,
        ...optionalIndexStamp(builtAt),
        ...optionalProfile(pack.profile),
    };
}
export function checkContextPackStaleness(index, now, staleAfterMs) {
    const reasons = [];
    if (!index.built_at) {
        reasons.push('missing-built-at');
    }
    else {
        const builtAtMs = Date.parse(index.built_at);
        if (Number.isNaN(builtAtMs)) {
            reasons.push('invalid-built-at');
        }
        else if (now.getTime() - builtAtMs > staleAfterMs) {
            reasons.push('expired');
        }
    }
    if (index.duplicate_keys.length > 0) {
        reasons.push('duplicate-keys');
    }
    return {
        stale: reasons.length > 0,
        reasons,
    };
}
export function seedContextPackIfAbsent(existing, seed) {
    return existing ?? seed;
}
export function createTaskInclusionQuery(task) {
    return {
        paths: task.paths,
        include_summary: true,
        ...(task.context_refs ? { refs: task.context_refs } : {}),
        ...(task.spec_ref ? { spec_refs: [task.spec_ref] } : {}),
    };
}
export function selectContextSlice(index, task, options = {}) {
    return buildSlice(selectFragments(index.fragments, createTaskInclusionQuery(task), options));
}
export function selectSpecSections(sections, query, options = {}) {
    const fragments = sections.map((section) => ({
        key: section.ref,
        kind: 'spec-section',
        source: 'spec',
        ref: section.ref,
        text: `## ${section.title}\n\n${section.text}`,
    }));
    return buildSlice(selectFragments(fragments, query, options));
}
export function selectFragments(fragments, query, options = {}) {
    const selected = fragments.filter((fragment) => matchesInclusion(fragment, query));
    return typeof options.maxFragments === 'number' ? selected.slice(0, options.maxFragments) : selected;
}
export function parseSpecSections(markdown) {
    const sections = [];
    let currentTitle;
    let currentRef;
    let currentLines = [];
    for (const line of markdown.split(/\r?\n/u)) {
        const heading = /^(#{1,6})\s+(.+)$/.exec(line);
        if (heading) {
            pushSection(sections, currentRef, currentTitle, currentLines);
            currentTitle = heading[2] ?? '';
            currentRef = titleToRef(currentTitle);
            currentLines = [];
        }
        else {
            currentLines.push(line);
        }
    }
    pushSection(sections, currentRef, currentTitle, currentLines);
    return sections;
}
function addFragment(fragments, byKey, duplicateKeys, fragment) {
    const existing = byKey[fragment.key];
    if (existing) {
        const existingIndex = fragments.findIndex((candidate) => candidate.key === fragment.key);
        if (existingIndex >= 0) {
            fragments[existingIndex] = fragment;
        }
        if (!isResolvedRefEnrichment(existing, fragment)) {
            duplicateKeys.push(fragment.key);
        }
        byKey[fragment.key] = fragment;
        return;
    }
    fragments.push(fragment);
    byKey[fragment.key] = fragment;
}
function isResolvedRefEnrichment(existing, fragment) {
    return (existing.kind === 'ref' &&
        fragment.kind === 'snippet' &&
        (Boolean(fragment.path) || Boolean(fragment.content_hash)));
}
function optionalStamp(builtAt) {
    return builtAt ? { built_at: builtAt } : {};
}
function optionalIndexStamp(builtAt) {
    return builtAt ? { built_at: builtAt } : {};
}
function optionalProfile(profile) {
    return profile ? { profile } : {};
}
function optionalPath(path) {
    return path ? { path } : {};
}
function optionalContentHash(contentHash) {
    return contentHash ? { content_hash: contentHash } : {};
}
function buildSlice(fragments) {
    return {
        keys: fragments.map((fragment) => fragment.key),
        fragments,
        summary: fragments.map((fragment) => fragment.text).join('\n\n'),
    };
}
function matchesInclusion(fragment, query) {
    if (query.include_summary && fragment.kind === 'summary') {
        return true;
    }
    const refs = normalizeAll(query.refs);
    const paths = normalizeAll(query.paths);
    const specRefs = normalizeAll(query.spec_refs);
    const terms = normalizeAll(query.terms);
    const fragmentKey = normalize(fragment.key);
    const fragmentRef = normalize(fragment.ref);
    const fragmentPath = normalize(fragment.path);
    const fragmentText = normalize(fragment.text);
    return (contains(refs, fragmentKey) ||
        contains(refs, fragmentRef) ||
        contains(specRefs, fragmentKey) ||
        contains(specRefs, fragmentRef) ||
        pathIntersects(paths, fragmentPath) ||
        containsTerm(terms, fragmentText));
}
function contains(values, candidate) {
    return candidate.length > 0 && values.includes(candidate);
}
function pathIntersects(paths, candidate) {
    return (candidate.length > 0 &&
        paths.some((path) => path === candidate || path.startsWith(`${candidate}/`) || candidate.startsWith(`${path}/`)));
}
function containsTerm(terms, text) {
    return text.length > 0 && terms.some((term) => term.length > 0 && text.includes(term));
}
function normalizeAll(values) {
    return (values ?? []).map(normalize).filter(Boolean);
}
function normalize(value) {
    return value?.trim().toLowerCase() ?? '';
}
function parseBuiltAt(summary) {
    return /\bBuilt at\s+([0-9]{4}-[0-9]{2}-[0-9]{2}T[0-9:.]+Z)\b/u.exec(summary)?.[1];
}
function titleToRef(title) {
    return title
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/gu, '-')
        .replace(/^-|-$/gu, '');
}
function pushSection(sections, ref, title, lines) {
    if (ref && title) {
        sections.push({
            ref,
            title,
            text: lines.join('\n').trim(),
        });
    }
}
