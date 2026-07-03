export const MAX_CONSTITUTION_CHARS = 6000;
export const EMBEDDED_CONSTITUTION = `# Constitution

No project \`.specify/memory/constitution.md\` was found. Apply the repository's
agent guide, keep changes minimal, validate against real files, and preserve
human authorship.
`;
export const REASONING_PROMPT_ROLES = [
    'planner',
    'critic',
    'reviser',
    'consolidator',
];
export const EXECUTION_PROMPT_ROLES = ['worker', 'verifier'];
const CONSTITUTION_TOKEN = '{{constitution}}';
export function render(template, values) {
    let rendered = template;
    for (const [key, value] of Object.entries(values)) {
        rendered = rendered.replaceAll(`{{${key}}}`, value);
    }
    return rendered;
}
export function promptPath(promptsDir, name) {
    assertPromptName(name);
    return joinAssetPath(promptsDir, `${name}.md`);
}
export function loadPrompt(assets, name) {
    return assets.readText(promptPath(assets.promptsDir, name));
}
export function constitutionPath(repoRoot) {
    return joinAssetPath(repoRoot, '.specify', 'memory', 'constitution.md');
}
export function readConstitutionContext(assets) {
    const text = assets.readTextIfExists(constitutionPath(assets.repoRoot)) ?? EMBEDDED_CONSTITUTION;
    return bounded(text.trim(), MAX_CONSTITUTION_CHARS);
}
export function requiresConstitution(role) {
    return REASONING_PROMPT_ROLES.some((reasoningRole) => reasoningRole === role);
}
export function assertConstitutionTokenPolicy(role, template) {
    const hasToken = template.includes(CONSTITUTION_TOKEN);
    if (requiresConstitution(role)) {
        if (!hasToken) {
            throw new Error(`${role} prompt must include ${CONSTITUTION_TOKEN}`);
        }
        return;
    }
    if (hasToken) {
        throw new Error(`${role} prompt must not include ${CONSTITUTION_TOKEN}`);
    }
}
export function renderCouncilPrompt(input) {
    const template = loadPrompt(input.promptAssets, input.role);
    assertConstitutionTokenPolicy(input.role, template);
    const baseline = loadPrompt(input.promptAssets, '_baseline');
    const values = managedValues(input.values, baseline);
    if (!requiresConstitution(input.role)) {
        return render(template, values);
    }
    if (input.constitutionAssets === undefined) {
        throw new Error(`${input.role} prompt requires constitution assets`);
    }
    return render(template, {
        ...values,
        constitution: readConstitutionContext(input.constitutionAssets),
    });
}
function assertPromptName(name) {
    if (!/^[A-Za-z0-9_-]+$/.test(name)) {
        throw new Error(`invalid prompt name: ${name}`);
    }
}
function joinAssetPath(first, ...rest) {
    const base = first.endsWith('/') ? first.slice(0, -1) : first;
    return [base, ...rest].join('/');
}
function bounded(text, limit) {
    if (text.length <= limit) {
        return text;
    }
    return `${text.slice(0, limit).trimEnd()}\n\n[truncated]`;
}
function managedValues(values, baseline) {
    const renderedValues = {};
    for (const [key, value] of Object.entries(values)) {
        if (key !== 'baseline' && key !== 'constitution') {
            renderedValues[key] = value;
        }
    }
    renderedValues.baseline = baseline;
    return renderedValues;
}
