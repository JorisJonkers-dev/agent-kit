export function chooseMilestone(title, milestones) {
    const milestone = milestones.find((candidate) => candidate.title === title);
    if (milestone) {
        return { kind: 'reuse', title, milestone };
    }
    return { kind: 'create', title, create: { title } };
}
export function taskMarker(runId, taskId) {
    return `<!-- council-task-id: ${runId}/${taskId} -->`;
}
export function findIssueByTaskMarker(runId, taskId, issues) {
    const marker = taskMarker(runId, taskId);
    const matches = issues.filter((issue) => issue.body?.includes(marker) ?? false);
    const [issue, ...duplicates] = matches;
    return issue ? { marker, issue, duplicates } : { marker, duplicates };
}
export function renderTaskIssueBody(input) {
    const specRef = input.specRef ?? input.task.spec_ref;
    const sections = [
        taskMarker(input.runId, input.task.id),
        `# ${input.task.id}: ${input.task.title}`,
        renderTaskSummary(input.task),
        renderStory(input.story),
        renderEdges(input.task, input.edgeIssueNumbers),
        specRef ? `## Spec\n\nspec_ref: ${specRef}` : '',
    ].filter((section) => section.length > 0);
    return `${sections.join('\n\n')}\n`;
}
export function selectBestFitLabels(input) {
    const candidates = [
        ...(input.preferred ?? []),
        ...statusLabelCandidates(input.status),
        ...taskLabelCandidates(input.task),
    ];
    return unique(candidates.flatMap((candidate) => existingLabel(candidate, input.existingLabels)));
}
export function buildStatusLabelTransition(currentLabels, existingLabels, nextStatus) {
    const currentStatusLabels = currentLabels
        .map(labelName)
        .filter((name) => name.startsWith('council/status:'));
    const nextStatusLabel = findExistingLabel(`council/status:${slug(nextStatus)}`, existingLabels);
    const remove = currentStatusLabels.filter((name) => name !== nextStatusLabel);
    const add = nextStatusLabel && !currentStatusLabels.includes(nextStatusLabel)
        ? [nextStatusLabel]
        : [];
    const labels = unique([
        ...currentLabels.map(labelName).filter((name) => !remove.includes(name)),
        ...add,
    ]);
    return { add, remove, labels };
}
export function issueStateAfterMirror(status, landed) {
    return landed && slug(status) === 'landed' ? 'closed' : 'open';
}
export function renderPullRequestBody(input) {
    const closingIssueNumbers = uniqueNumbers(input.closingIssueNumbers ?? []);
    const referenceIssueNumbers = uniqueNumbers(input.referenceIssueNumbers ?? []).filter((number) => !closingIssueNumbers.includes(number));
    const issueLines = [
        ...closingIssueNumbers.map((number) => `Closes #${String(number)}`),
        ...referenceIssueNumbers.map((number) => `refs #${String(number)}`),
    ];
    const sections = [
        input.summary.trim(),
        issueLines.join('\n'),
        ...(input.extraSections ?? []).map((section) => section.trim()),
    ].filter((section) => section.length > 0);
    return `${sections.join('\n\n')}\n`;
}
function renderTaskSummary(task) {
    const lines = [
        '## Task',
        '',
        `Objective: ${task.objective}`,
        `Output: ${task.output_format}`,
        `Paths: ${task.paths.join(', ') || 'none'}`,
        `Verify: ${task.verify}`,
        `Boundaries: ${task.boundaries}`,
        ...renderList('Acceptance criteria', task.acceptance_criteria),
    ];
    if (task.dev_notes) {
        lines.push(`Dev notes: ${task.dev_notes}`);
    }
    return lines.join('\n');
}
function renderStory(story) {
    if (!story) {
        return '';
    }
    return [
        '## Story',
        '',
        `Status: ${story.status}`,
        `Goal: ${story.goal}`,
        [
            `User value: As ${story.user_value.actor},`,
            `${story.user_value.capability},`,
            `so ${story.user_value.outcome}.`,
        ].join(' '),
        `Context: ${story.context}`,
        ...renderList('Story acceptance criteria', story.acceptance_criteria),
        ...renderList('In scope', story.scope.in_scope),
        ...renderList('Out of scope', story.scope.out_of_scope),
        ...renderList('Implementation files', story.implementation_notes.files),
        ...renderList('Implementation patterns', story.implementation_notes.patterns),
        ...renderList('Dependencies', story.implementation_notes.dependencies),
        ...renderList('Data/config/migration', story.implementation_notes.data_config_migration),
        ...renderList('Unit tests', story.tests.unit),
        ...renderList('Integration tests', story.tests.integration),
        ...renderList('Manual/workflow tests', story.tests.manual_or_workflow),
        ...renderList('Definition of done', story.definition_of_done),
    ].join('\n');
}
function renderEdges(task, issueNumbers) {
    const lines = [
        ...task.depends_on.flatMap((taskId) => renderEdge('Blocked by', taskId, issueNumbers)),
        ...renderOptionalEdge('discovered-from', task.discovered_from, issueNumbers),
        ...(task.supersedes ?? []).flatMap((taskId) => renderEdge('supersedes', taskId, issueNumbers)),
    ];
    return lines.length > 0 ? ['## Edges', '', ...lines].join('\n') : '';
}
function renderOptionalEdge(label, taskId, issueNumbers) {
    return taskId ? renderEdge(label, taskId, issueNumbers) : [];
}
function renderEdge(label, taskId, issueNumbers) {
    const issueNumber = issueNumbers?.get(taskId);
    return [`${label} ${issueNumber ? `#${String(issueNumber)}` : taskId}`];
}
function renderList(label, values) {
    if (!values || values.length === 0) {
        return [];
    }
    return ['', `${label}:`, ...values.map((value) => `- ${value}`)];
}
function statusLabelCandidates(status) {
    return status ? [`council/status:${slug(status)}`] : [];
}
function taskLabelCandidates(task) {
    if (!task) {
        return [];
    }
    const pathSegments = task.paths.flatMap((path) => {
        const segment = path.split('/').find((part) => part.length > 0);
        return segment ? [`area:${slug(segment)}`, `path:${slug(segment)}`] : [];
    });
    return [
        task.difficulty,
        `difficulty:${task.difficulty}`,
        `model:${task.model}`,
        ...optionalLabelPair('archetype', task.archetype),
        ...optionalLabelPair('context', task.context_profile),
        ...pathSegments,
    ];
}
function optionalLabelPair(prefix, value) {
    return value
        ? [`${prefix}:${slug(value)}`, `council/${prefix}:${slug(value)}`]
        : [];
}
function existingLabel(candidate, existingLabels) {
    const label = findExistingLabel(candidate, existingLabels);
    return label ? [label] : [];
}
function findExistingLabel(candidate, existingLabels) {
    const normalizedCandidate = candidate.toLowerCase();
    const label = existingLabels.find((existing) => labelName(existing).toLowerCase() === normalizedCandidate);
    return label ? labelName(label) : undefined;
}
function labelName(label) {
    return typeof label === 'string' ? label : label.name;
}
function slug(value) {
    return value
        .trim()
        .toLowerCase()
        .replace(/[^a-z0-9]+/g, '-')
        .replace(/^-|-$/g, '');
}
function unique(values) {
    return [...new Set(values)];
}
function uniqueNumbers(values) {
    return [...new Set(values)];
}
