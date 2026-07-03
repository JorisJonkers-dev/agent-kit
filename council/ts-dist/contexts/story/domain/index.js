const PLACEHOLDER = '_Not recorded yet._';
const VAGUE_OBJECTIVES = new Set(['fix stuff', 'misc', 'cleanup', 'improve things', 'make better']);
const VALID_PATH_RE = /^[A-Za-z0-9._/@+-][A-Za-z0-9._/@+\- ]*$/;
const PINNED_VERSION_RE = /^(?:v?\d+\.\d+\.\d+(?:[-+][0-9A-Za-z.-]+)?|\^[0-9]+\.[0-9]+\.[0-9]+|~[0-9]+\.[0-9]+\.[0-9]+|[<>=~^]+[0-9]+\.[0-9]+\.[0-9]+)$/;
const LIBRARY_ASSUMPTION_RE = /\b(?:use|uses|using|install|add|depend(?:s|ing)? on)\s+(@?[a-z0-9][a-z0-9._/-]*)(?:@([0-9][0-9A-Za-z.+-]*|latest|next|canary))?/gi;
export function renderStoryMarkdown(input) {
    const { task } = input;
    const specSections = selectSpecSections(task.spec_ref, input.specSections ?? []);
    const contextSnippets = selectContextSnippets(task.context_refs ?? [], input.contextPack);
    const structureNotes = selectStructureNotes(task.paths, input.structureNotes ?? []);
    const acceptanceCriteria = nonEmptyItems(task.acceptance_criteria ?? []);
    return [
        `# Story: ${inline(task.title)}`,
        '',
        '## Story',
        '',
        `Task ${task.id}: ${inline(task.objective)}`,
        '',
        '## Acceptance Criteria',
        '',
        numberedList(acceptanceCriteria),
        '',
        '## Tasks-Subtasks',
        '',
        checkboxList([`Implement ${task.id}: ${task.title}`, ...acceptanceCriteria.map((item) => `Verify: ${item}`)]),
        '',
        '## Dev Notes',
        '',
        bulletList([
            `Output format: ${task.output_format}`,
            `Verification: ${task.verify}`,
            `Boundaries: ${task.boundaries}`,
            task.dev_notes !== undefined && task.dev_notes.trim().length > 0
                ? `Task notes: ${task.dev_notes}`
                : '',
            task.spec_ref !== undefined && task.spec_ref.trim().length > 0
                ? `Cited spec sections: ${task.spec_ref}`
                : '',
            ...(specSections.length > 0 ? ['Spec excerpts:', ...quoteBlocks(specSections)] : []),
            ...contextSummary(input.contextPack, contextSnippets),
        ]),
        '',
        '## Structure Notes',
        '',
        bulletList([
            ...task.paths.map((path) => `Allowed path: ${path}`),
            ...structureNotes.map((note) => `${note.path}: ${note.note}`),
        ]),
        '',
        '## Dev Agent Record',
        '',
        PLACEHOLDER,
        '',
        '## File List',
        '',
        bulletList(task.paths),
        '',
    ].join('\n');
}
export function validateStoryReadiness(input) {
    const pathsToCheck = uniqueStrings([...input.task.paths, ...storyPathCandidates(input.storyMarkdown)]);
    const issues = [
        ...vagueObjectiveIssues(input.task.objective),
        ...missingAcceptanceCriteriaIssues(input.task.acceptance_criteria, input.storyMarkdown),
        ...wrongPathIssues(pathsToCheck, input.knownPaths),
        ...unpinnedLibraryIssues(input.libraryAssumptions ?? inferredLibraries(input)),
    ];
    const ready = issues.every((issue) => !issue.blocking);
    return {
        ready,
        issues,
        revision: revisionSignal(ready, issues, input.revisionRound ?? 0),
    };
}
function selectSpecSections(specRef, sections) {
    const citedRefs = refSet(specRef !== undefined ? [specRef] : []);
    return sections.filter((section) => citedRefs.has(section.ref));
}
function selectContextSnippets(refs, contextPack) {
    const citedRefs = refSet(refs);
    return (contextPack?.snippets ?? []).filter((snippet) => citedRefs.has(snippet.ref));
}
function selectStructureNotes(paths, notes) {
    const pathSet = new Set(paths);
    return notes.filter((note) => pathSet.has(note.path));
}
function contextSummary(pack, snippets) {
    if (pack === undefined || snippets.length === 0) {
        return [];
    }
    return [
        `Context pack summary: ${pack.summary}`,
        'Context excerpts:',
        ...snippets.map((snippet) => quoteBlock(snippet.ref, snippet.path, snippet.text)),
    ];
}
function refSet(refs) {
    const expanded = refs.flatMap((ref) => ref
        .split(/[,\n]/u)
        .map((part) => part.trim())
        .filter((part) => part.length > 0));
    return new Set(expanded);
}
function numberedList(items) {
    if (items.length === 0) {
        return PLACEHOLDER;
    }
    return items.map((item, index) => `${String(index + 1)}. ${inline(item)}`).join('\n');
}
function checkboxList(items) {
    const cleaned = nonEmptyItems(items);
    return cleaned.map((item) => `- [ ] ${inline(item)}`).join('\n');
}
function bulletList(items) {
    const cleaned = nonEmptyItems(items);
    if (cleaned.length === 0) {
        return PLACEHOLDER;
    }
    return cleaned.map((item) => `- ${inline(item)}`).join('\n');
}
function quoteBlocks(sections) {
    return sections.map((section) => quoteBlock(section.ref, section.title, section.text));
}
function quoteBlock(ref, label, text) {
    const heading = label !== undefined && label.length > 0 ? `${ref} (${label})` : ref;
    const body = text
        .trim()
        .split(/\r?\n/u)
        .map((line) => `  ${line}`)
        .join('\n');
    return `${heading}\n${body}`;
}
function inline(value) {
    return value.replace(/\s+/gu, ' ').trim();
}
function nonEmptyItems(items) {
    return items.map(inline).filter((item) => item.length > 0);
}
function vagueObjectiveIssues(objective) {
    const normalized = inline(objective).toLowerCase();
    const tooShort = normalized.length < 16;
    const explicitlyVague = VAGUE_OBJECTIVES.has(normalized) || /\b(?:tbd|todo|whatever)\b/u.test(normalized);
    if (!tooShort && !explicitlyVague) {
        return [];
    }
    return [
        {
            code: 'vague-objective',
            message: 'Objective must name the concrete behavior or artifact to deliver.',
            blocking: true,
        },
    ];
}
function missingAcceptanceCriteriaIssues(taskAcceptanceCriteria, storyMarkdown) {
    if (nonEmptyItems(taskAcceptanceCriteria ?? []).length > 0 || hasNumberedAcceptanceCriteria(storyMarkdown)) {
        return [];
    }
    return [
        {
            code: 'missing-acceptance-criteria',
            message: 'Story needs at least one observable acceptance criterion.',
            blocking: true,
        },
    ];
}
function hasNumberedAcceptanceCriteria(storyMarkdown) {
    const section = sectionBody(storyMarkdown, 'Acceptance Criteria');
    return section !== undefined ? /^\s*\d+\.\s+\S/mu.test(section) : false;
}
function storyPathCandidates(storyMarkdown) {
    return ['Structure Notes', 'File List'].flatMap((sectionName) => sectionPaths(sectionBody(storyMarkdown, sectionName)));
}
function sectionBody(storyMarkdown, sectionName) {
    return storyMarkdown.match(sectionPattern(sectionName))?.groups?.body;
}
function sectionPattern(sectionName) {
    return new RegExp(`## ${escapeRegExp(sectionName)}\\s*\\n(?<body>[\\s\\S]*?)(?:\\n## |\\s*$)`, 'u');
}
function sectionPaths(section) {
    if (section === undefined || section.length === 0) {
        return [];
    }
    return section
        .split(/\r?\n/u)
        .flatMap((line) => pathFromListItem(line))
        .filter((path) => path !== PLACEHOLDER);
}
function pathFromListItem(line) {
    const path = (/^\s*-\s+(?:Allowed path:\s*)?(?<path>\S.*)$/u.exec(line))?.groups?.path?.trim();
    if (path === undefined || path.length === 0) {
        return [];
    }
    const noteIndex = path.indexOf(': ');
    return [noteIndex === -1 ? path : path.slice(0, noteIndex)];
}
function wrongPathIssues(paths, knownPaths) {
    return paths.flatMap((path) => {
        const malformed = path.trim() !== path || !VALID_PATH_RE.test(path) || path.includes('..');
        const unknown = knownPaths !== undefined ? !knownPaths.has(path) : false;
        if (!malformed && !unknown) {
            return [];
        }
        return [
            {
                code: 'wrong-path',
                message: unknown ? `Path is not known in this repository: ${path}` : `Path is malformed: ${path}`,
                blocking: true,
            },
        ];
    });
}
function inferredLibraries(input) {
    const text = [input.task.objective, input.task.dev_notes ?? '', input.storyMarkdown].join('\n');
    const assumptions = [];
    for (const match of text.matchAll(LIBRARY_ASSUMPTION_RE)) {
        const name = match[1];
        const version = match[2];
        if (name !== undefined && !isLocalPath(name)) {
            assumptions.push(version !== undefined ? { name, version } : { name });
        }
    }
    return assumptions;
}
function isLocalPath(value) {
    return value.startsWith('.') || value.startsWith('/') || (!value.startsWith('@') && value.includes('/'));
}
function unpinnedLibraryIssues(assumptions) {
    return assumptions.flatMap((assumption) => {
        const version = assumption.version?.trim();
        if (version !== undefined &&
            version.length > 0 &&
            version !== 'latest' &&
            version !== 'next' &&
            PINNED_VERSION_RE.test(version)) {
            return [];
        }
        return [
            {
                code: 'unpinned-library-assumption',
                message: `Library assumption must be pinned to a concrete version: ${assumption.name}`,
                blocking: true,
            },
        ];
    });
}
function revisionSignal(ready, issues, revisionRound) {
    if (ready) {
        return { kind: 'none', reason: 'ready' };
    }
    if (revisionRound >= 1) {
        return { kind: 'none', reason: 'revision-budget-exhausted' };
    }
    return {
        kind: 'revise-once',
        round: 1,
        issueCodes: uniqueIssueCodes(issues),
        message: 'Revise the story once to resolve the blocking readiness issues.',
    };
}
function uniqueIssueCodes(issues) {
    return [...new Set(issues.filter((issue) => issue.blocking).map((issue) => issue.code))];
}
function uniqueStrings(values) {
    return [...new Set(values)];
}
function escapeRegExp(value) {
    return value.replace(/[.*+?^${}()|[\]\\]/gu, '\\$&');
}
