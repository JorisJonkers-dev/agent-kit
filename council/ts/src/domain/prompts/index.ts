export const MAX_CONSTITUTION_CHARS = 6000

export const EMBEDDED_CONSTITUTION = `# Constitution

No project \`.specify/memory/constitution.md\` was found. Apply the repository's
agent guide, keep changes minimal, validate against real files, and preserve
human authorship.
`

export const REASONING_PROMPT_ROLES = [
  'planner',
  'critic',
  'reviser',
  'consolidator',
] as const

export const EXECUTION_PROMPT_ROLES = ['worker', 'verifier'] as const

export type ReasoningPromptRole = (typeof REASONING_PROMPT_ROLES)[number]

export type ExecutionPromptRole = (typeof EXECUTION_PROMPT_ROLES)[number]

export type CouncilPromptRole = ReasoningPromptRole | ExecutionPromptRole

export type PromptValues = Readonly<Record<string, string>>

export interface PromptAssets {
  readonly promptsDir: string
  readonly readText: (path: string) => string
}

export interface ConstitutionAssets {
  readonly repoRoot: string
  readonly readTextIfExists: (path: string) => string | undefined
}

export interface CouncilPromptRenderInput {
  readonly role: CouncilPromptRole
  readonly promptAssets: PromptAssets
  readonly constitutionAssets?: ConstitutionAssets
  readonly values: PromptValues
}

const CONSTITUTION_TOKEN = '{{constitution}}'

export function render(template: string, values: PromptValues): string {
  let rendered = template
  for (const [key, value] of Object.entries(values)) {
    rendered = rendered.replaceAll(`{{${key}}}`, value)
  }
  return rendered
}

export function promptPath(promptsDir: string, name: string): string {
  assertPromptName(name)
  return joinAssetPath(promptsDir, `${name}.md`)
}

export function loadPrompt(assets: PromptAssets, name: string): string {
  return assets.readText(promptPath(assets.promptsDir, name))
}

export function constitutionPath(repoRoot: string): string {
  return joinAssetPath(repoRoot, '.specify', 'memory', 'constitution.md')
}

export function readConstitutionContext(assets: ConstitutionAssets): string {
  const text =
    assets.readTextIfExists(constitutionPath(assets.repoRoot)) ?? EMBEDDED_CONSTITUTION
  return bounded(text.trim(), MAX_CONSTITUTION_CHARS)
}

export function requiresConstitution(role: CouncilPromptRole): role is ReasoningPromptRole {
  return REASONING_PROMPT_ROLES.some((reasoningRole) => reasoningRole === role)
}

export function assertConstitutionTokenPolicy(role: CouncilPromptRole, template: string): void {
  const hasToken = template.includes(CONSTITUTION_TOKEN)
  if (requiresConstitution(role)) {
    if (!hasToken) {
      throw new Error(`${role} prompt must include ${CONSTITUTION_TOKEN}`)
    }
    return
  }
  if (hasToken) {
    throw new Error(`${role} prompt must not include ${CONSTITUTION_TOKEN}`)
  }
}

export function renderCouncilPrompt(input: CouncilPromptRenderInput): string {
  const template = loadPrompt(input.promptAssets, input.role)
  assertConstitutionTokenPolicy(input.role, template)

  const baseline = loadPrompt(input.promptAssets, '_baseline')
  const values = managedValues(input.values, baseline)
  if (!requiresConstitution(input.role)) {
    return render(template, values)
  }
  if (input.constitutionAssets === undefined) {
    throw new Error(`${input.role} prompt requires constitution assets`)
  }
  return render(template, {
    ...values,
    constitution: readConstitutionContext(input.constitutionAssets),
  })
}

function assertPromptName(name: string): void {
  if (!/^[A-Za-z0-9_-]+$/.test(name)) {
    throw new Error(`invalid prompt name: ${name}`)
  }
}

function joinAssetPath(first: string, ...rest: readonly string[]): string {
  const base = first.endsWith('/') ? first.slice(0, -1) : first
  return [base, ...rest].join('/')
}

function bounded(text: string, limit: number): string {
  if (text.length <= limit) {
    return text
  }
  return `${text.slice(0, limit).trimEnd()}\n\n[truncated]`
}

function managedValues(values: PromptValues, baseline: string): PromptValues {
  const renderedValues: Record<string, string> = {}
  for (const [key, value] of Object.entries(values)) {
    if (key !== 'baseline' && key !== 'constitution') {
      renderedValues[key] = value
    }
  }
  renderedValues.baseline = baseline
  return renderedValues
}
