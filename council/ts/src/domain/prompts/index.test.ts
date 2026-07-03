import { describe, expect, it } from 'vitest'

import {
  EMBEDDED_CONSTITUTION,
  MAX_CONSTITUTION_CHARS,
  assertConstitutionTokenPolicy,
  constitutionPath,
  loadPrompt,
  promptPath,
  readConstitutionContext,
  render,
  renderCouncilPrompt,
  requiresConstitution,
  type ConstitutionAssets,
  type PromptAssets,
} from './index.js'

describe('prompt rendering', () => {
  it('replaces every provided token and leaves missing tokens intact', () => {
    expect(
      render('{{greeting}}, {{name}}. {{greeting}} again. {{missing}}', {
        greeting: 'hello',
        name: 'council',
      }),
    ).toBe('hello, council. hello again. {{missing}}')
  })
})

describe('prompt loading', () => {
  it('loads markdown prompts from the injected prompts directory', () => {
    const paths: string[] = []
    const assets: PromptAssets = {
      promptsDir: 'council/prompts',
      readText: (path) => {
        paths.push(path)
        return `loaded ${path}`
      },
    }

    expect(loadPrompt(assets, 'planner')).toBe('loaded council/prompts/planner.md')
    expect(paths).toEqual(['council/prompts/planner.md'])
  })

  it('joins prompt paths predictably and rejects names that escape the asset set', () => {
    expect(promptPath('council/prompts/', '_baseline')).toBe('council/prompts/_baseline.md')
    expect(() => promptPath('council/prompts', '../worker')).toThrow(
      'invalid prompt name: ../worker',
    )
  })
})

describe('constitution context', () => {
  it('builds the repository constitution path', () => {
    expect(constitutionPath('/repo')).toBe('/repo/.specify/memory/constitution.md')
  })

  it('reads and trims a concrete constitution file', () => {
    const assets: ConstitutionAssets = {
      repoRoot: '/repo',
      readTextIfExists: (path) =>
        path === '/repo/.specify/memory/constitution.md'
          ? '\n# Constitution\n\nShip small, verified changes.\n\n'
          : undefined,
    }

    expect(readConstitutionContext(assets)).toBe(
      '# Constitution\n\nShip small, verified changes.',
    )
  })

  it('falls back to the embedded constitution when no project file exists', () => {
    const assets: ConstitutionAssets = {
      repoRoot: '/repo',
      readTextIfExists: () => undefined,
    }

    expect(readConstitutionContext(assets)).toBe(EMBEDDED_CONSTITUTION.trim())
  })

  it('bounds constitution context to the Python limit plus truncation marker', () => {
    const assets: ConstitutionAssets = {
      repoRoot: '/repo',
      readTextIfExists: () => `${'x'.repeat(MAX_CONSTITUTION_CHARS)}   y`,
    }

    expect(readConstitutionContext(assets)).toBe(
      `${'x'.repeat(MAX_CONSTITUTION_CHARS)}\n\n[truncated]`,
    )
  })
})

describe('role constitution policy', () => {
  it('classifies reasoning and execution prompt roles', () => {
    expect(requiresConstitution('planner')).toBe(true)
    expect(requiresConstitution('worker')).toBe(false)
  })

  it('accepts Python self-test constitution token placement', () => {
    expect(() => { assertConstitutionTokenPolicy('planner', '{{constitution}}'); }).not.toThrow()
    expect(() => { assertConstitutionTokenPolicy('verifier', '{{schema}}'); }).not.toThrow()
  })

  it('rejects missing constitution tokens for reasoning roles', () => {
    expect(() => { assertConstitutionTokenPolicy('critic', '{{schema}}'); }).toThrow(
      'critic prompt must include {{constitution}}',
    )
  })

  it('rejects constitution tokens for worker and verifier roles', () => {
    expect(() => { assertConstitutionTokenPolicy('worker', '{{constitution}}'); }).toThrow(
      'worker prompt must not include {{constitution}}',
    )
    expect(() => { assertConstitutionTokenPolicy('verifier', '{{constitution}}'); }).toThrow(
      'verifier prompt must not include {{constitution}}',
    )
  })
})

describe('council prompt rendering', () => {
  const promptAssets = (templates: Readonly<Record<string, string>>): PromptAssets => ({
    promptsDir: 'council/prompts',
    readText: (path) => {
      const name = path.slice('council/prompts/'.length, -'.md'.length)
      const template = templates[name]
      if (template === undefined) {
        throw new Error(`missing template ${name}`)
      }
      return template
    },
  })

  it('renders reasoning role prompts with injected baseline and constitution context', () => {
    const result = renderCouncilPrompt({
      role: 'consolidator',
      promptAssets: promptAssets({
        _baseline: 'baseline rules',
        consolidator: '{{topic}}\n{{baseline}}\n{{constitution}}',
      }),
      constitutionAssets: {
        repoRoot: '/repo',
        readTextIfExists: () => '# Constitution\n\nConcrete rules.',
      },
      values: {
        baseline: 'caller baseline',
        constitution: 'caller constitution',
        topic: 'merge plans',
      },
    })

    expect(result).toBe('merge plans\nbaseline rules\n# Constitution\n\nConcrete rules.')
  })

  it('requires constitution assets for reasoning role prompts', () => {
    expect(() =>
      renderCouncilPrompt({
        role: 'reviser',
        promptAssets: promptAssets({
          _baseline: 'baseline rules',
          reviser: '{{baseline}}\n{{constitution}}',
        }),
        values: {},
      }),
    ).toThrow('reviser prompt requires constitution assets')
  })

  it('renders worker and verifier prompts without constitution context', () => {
    const result = renderCouncilPrompt({
      role: 'worker',
      promptAssets: promptAssets({
        _baseline: 'baseline rules',
        worker: '{{title}}\n{{baseline}}',
      }),
      values: {
        constitution: 'must not leak',
        title: 'Execute T1',
      },
    })

    expect(result).toBe('Execute T1\nbaseline rules')
  })
})
