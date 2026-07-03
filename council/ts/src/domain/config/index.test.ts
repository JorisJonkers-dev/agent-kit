import { describe, expect, it } from 'vitest'

import type { EnvPort } from '../../ports/index.js'
import {
  coerceConfigValue,
  parseCouncilConfig,
  parseToml,
  resolveCouncilConfig,
  writeCouncilConfig,
  writeTomlUpdates,
  writeTomlValue,
} from './index.js'

class TestEnv implements EnvPort {
  constructor(private readonly values: Readonly<Record<string, string>>) {}

  get(name: string): string | undefined {
    return this.values[name]
  }

  require(name: string): string {
    const value = this.get(name)
    if (value === undefined) {
      throw new Error(`missing ${name}`)
    }
    return value
  }

  all(): Readonly<Record<string, string>> {
    return this.values
  }
}

const fullConfig = `# leading comment
intensity = "standard"
planner_a = "claude:opus"
planner_b = "codex:gpt-5.5"
rounds = 2

[watchdog]
stall_after_s = 300
window = 40
repeat_limit = 6
max_restarts = 1
escalate_model = "claude:opus"
disk_cap_gib = 2

[design]
lenses = ["architecture", "implementation", "risk"]
rounds = 2

[design.stages.survey]
engine = "claude:sonnet"
effort = "medium"

[review]
council = false
max_fix_rounds = 2

[review.difficulty]
trivial = "claude:haiku"
moderate = "claude:haiku"
hard = "claude:sonnet"

[github]
enabled = false
assignee = ""

[engines.claude]
argv = ["sh", "-lc", "echo # not a comment"]
stream_format = "json"
result_extraction = "json.result"

[triage.matrix_overrides]
trivial = "claude:haiku"

[context]
pack_stale_after_s = 86400

[model_matrix.roles]
planner_a = "claude:opus"
planner_b = "codex:gpt-5.5"
consolidator = "claude:opus"
verifier = "claude:sonnet"

[model_matrix.intensity.quick]
rounds = 1
codex_effort = "low"
worker = "claude:haiku"
max_workers = 4
`

describe('parseToml', () => {
  it('parses the council TOML surface with tables, arrays, booleans, strings, and integers', () => {
    const document = parseToml(fullConfig)

    expect(document.finalNewline).toBe(true)
    expect(document.source).toBe(fullConfig)
    expect(document.tables.map((table) => table.path.join('.'))).toContain('engines.claude')
    expect(document.assignments.some((assignment) => assignment.sourceKey === 'argv')).toBe(true)
    expect(document.data).toMatchObject({
      intensity: 'standard',
      watchdog: { stall_after_s: 300, disk_cap_gib: 2 },
      design: {
        lenses: ['architecture', 'implementation', 'risk'],
        stages: { survey: { engine: 'claude:sonnet', effort: 'medium' } },
      },
      review: { council: false, difficulty: { hard: 'claude:sonnet' } },
      github: { enabled: false, assignee: '' },
      engines: {
        claude: {
          argv: ['sh', '-lc', 'echo # not a comment'],
          stream_format: 'json',
          result_extraction: 'json.result',
        },
      },
      model_matrix: {
        roles: { planner_b: 'codex:gpt-5.5' },
        intensity: { quick: { rounds: 1, codex_effort: 'low' } },
      },
    })
  })

  it('parses dotted keys and arrays of tables', () => {
    const document = parseToml(
      `title = "demo \\"quoted\\""\nparent.child = 1\narray = ["a\\"b", ["nested", 2], "literal # hash"] # trailing\n\n[[engines.codex]]\nargv = ["codex"]\n\n[[engines.codex]]\nargv = []\n`,
    )

    expect(document.tables).toEqual([
      { lineIndex: 4, path: ['engines', 'codex'], array: true },
      { lineIndex: 7, path: ['engines', 'codex'], array: true },
    ])
    expect(document.data).toEqual({
      title: 'demo "quoted"',
      parent: { child: 1 },
      array: ['a"b', ['nested', 2], 'literal # hash'],
      engines: { codex: [{ argv: ['codex'] }, { argv: [] }] },
    })
  })

  it('rejects unsupported TOML syntax in the scoped parser', () => {
    expect(() => parseToml('bad line')).toThrow('invalid TOML assignment')
    expect(() => parseToml('bad key = "x"')).toThrow('unsupported TOML key path')
    expect(() => parseToml('value = 1.2')).toThrow('unsupported TOML value')
    expect(() => parseToml('value = "unterminated')).toThrow('invalid TOML string')
    expect(() => parseToml('value = [1')).toThrow('unterminated TOML array')
    expect(() => parseToml('[name]\nvalue = 1\n[[name]]')).toThrow('TOML array table conflicts')
    expect(() => parseToml('[[name]]\nvalue = 1\n[name]\nvalue = 2')).toThrow('TOML table conflicts')
    expect(() => parseToml('bad"\\=" = 1')).toThrow('unsupported TOML key path')
    expect(() => parseToml('bad[inner] = 1')).toThrow('unsupported TOML key path')
  })
})

describe('parseCouncilConfig', () => {
  it('normalizes known council config and drops unknown keys from typed output', () => {
    const config = parseCouncilConfig(`${fullConfig}\nunknown = "ignored"\n[unknown_table]\nvalue = true\n`)

    expect(config).toMatchObject({
      intensity: 'standard',
      planner_a: 'claude:opus',
      planner_b: 'codex:gpt-5.5',
      rounds: 2,
      watchdog: {
        stall_after_s: 300,
        window: 40,
        repeat_limit: 6,
        max_restarts: 1,
        escalate_model: 'claude:opus',
        disk_cap_gib: 2,
      },
      design: {
        lenses: ['architecture', 'implementation', 'risk'],
        rounds: 2,
        stages: { survey: { engine: 'claude:sonnet', effort: 'medium' } },
      },
      review: {
        council: false,
        max_fix_rounds: 2,
        difficulty: { trivial: 'claude:haiku', moderate: 'claude:haiku', hard: 'claude:sonnet' },
      },
      github: { enabled: false, assignee: '' },
      engines: {
        claude: {
          argv: ['sh', '-lc', 'echo # not a comment'],
          stream_format: 'json',
          result_extraction: 'json.result',
        },
      },
      triage: { matrix_overrides: { trivial: 'claude:haiku' } },
      context: { pack_stale_after_s: 86400 },
      model_matrix: {
        roles: {
          planner_a: 'claude:opus',
          planner_b: 'codex:gpt-5.5',
          consolidator: 'claude:opus',
          verifier: 'claude:sonnet',
        },
        intensity: {
          quick: { rounds: 1, codex_effort: 'low', worker: 'claude:haiku', max_workers: 4 },
        },
      },
    })
    expect(config).not.toHaveProperty('unknown')
  })

  it('omits typed fields that have the wrong TOML type or enum value', () => {
    const config = parseCouncilConfig(`
intensity = "bad"
rounds = "two"
codex_effort = "ludicrous"
[design]
lenses = ["risk", 1]
[review]
council = "yes"
[review.difficulty]
hard = 1
[model_matrix.roles]
planner_a = "claude:opus"
extra = "ignored"
`)

    expect(config).toEqual({ model_matrix: { roles: { planner_a: 'claude:opus' } } })
  })

  it('omits nested record fields when the TOML value is not a table', () => {
    const config = parseCouncilConfig(`
[review]
difficulty = "bad"

[model_matrix]
roles = "bad"
`)

    expect(config).toEqual({})
  })

  it('normalizes the latest table from array-of-table records', () => {
    const config = parseCouncilConfig(`
[[engines.codex]]
argv = ["old"]

[[engines.codex]]
argv = ["codex"]
stream_format = "text"
`)

    expect(config.engines).toEqual({ codex: { argv: ['codex'], stream_format: 'text' } })
    expect(parseCouncilConfig('[engines]\ncodex = "bad"\n')).toEqual({})
  })
})

describe('writeTomlUpdates and writeCouncilConfig', () => {
  it('updates existing values in place and preserves comments, unknown tables, and order', () => {
    const source = `# root
intensity = "standard" # keep
unknown = "same"

[unknown.table]
value = true

[engines.claude]
argv = ["old"]
stream_format = "json"
`

    const output = writeCouncilConfig(source, {
      intensity: 'thorough',
      engines: { claude: { argv: ['new', 'value'] } },
      review: { council: true },
    })

    expect(output).toBe(`# root
intensity = "thorough" # keep
unknown = "same"

[unknown.table]
value = true

[engines.claude]
argv = ["new", "value"]
stream_format = "json"

[review]
council = true
`)
  })

  it('inserts missing root keys before the first table and appends missing table keys inside the table', () => {
    const output = writeCouncilConfig('[github]\nenabled = false\n', {
      planner_a: 'claude:opus',
      planner_b: 'codex:gpt-5.5',
      github: { assignee: 'jane' },
    })

    expect(output).toBe(
      'planner_a = "claude:opus"\nplanner_b = "codex:gpt-5.5"\n[github]\nenabled = false\nassignee = "jane"\n',
    )
  })

  it('writes direct TOML updates and rejects unsupported inline table values', () => {
    const document = parseToml('name = "old"')
    const updates = new Map<string, string | number>([
      ['name', 'new'],
      ['nested.value', 1],
    ])

    expect(writeTomlUpdates(document, updates)).toBe('name = "new"\n\n[nested]\nvalue = 1\n')
    expect(writeTomlValue(false)).toBe('false')
    expect(() => writeTomlValue({ value: 'inline' })).toThrow('inline TOML tables')
    expect(() => writeTomlValue(Number.POSITIVE_INFINITY)).toThrow('finite')
  })

  it('updates the latest matching assignment for repeated table paths', () => {
    const output = writeCouncilConfig('[[engines.codex]]\nargv = ["old"]\n\n[[engines.codex]]\nargv = ["latest"]\n', {
      engines: { codex: { argv: ['new'], stream_format: 'text' } },
    })

    expect(output).toBe(
      '[[engines.codex]]\nargv = ["old"]\n\n[[engines.codex]]\nargv = ["new"]\nstream_format = "text"\n',
    )
  })

  it('groups multiple missing assignments under the same new table and rejects empty update keys', () => {
    const output = writeCouncilConfig('intensity = "quick"\n', {
      review: { difficulty: { trivial: 'claude:haiku', hard: 'claude:sonnet' } },
    })

    expect(output).toBe(`intensity = "quick"

[review.difficulty]
trivial = "claude:haiku"
hard = "claude:sonnet"
`)
    expect(() => writeTomlUpdates(parseToml(''), new Map([['', 'value']]))).toThrow(
      'cannot write empty TOML path',
    )
  })
})

describe('resolveCouncilConfig', () => {
  it('resolves preset, user, project, flags, and env precedence without process globals', () => {
    const env = new TestEnv({
      COUNCIL_CODEX_REASONING: 'medium',
      COUNCIL_PLAN_TIMEOUT_S: '10',
      COUNCIL_WORKER_TIMEOUT_S: '20',
      COUNCIL_VERIFY_TIMEOUT_S: '30',
    })

    const resolved = resolveCouncilConfig({
      preset: 'quick',
      user: {
        intensity: 'thorough',
        worker: 'claude:user-worker',
        max_workers: 5,
        design: { rounds: 1, stages: { survey: { engine: 'claude:user' } } },
      },
      project: {
        planner_b: 'codex:project',
        design: { stages: { survey: { effort: 'high' } } },
      },
      flags: {
        intensity: 'max',
        rounds: 4,
      },
      env,
    })

    expect(resolved).toMatchObject({
      intensity: 'max',
      planner_a: 'claude:opus',
      planner_b: 'codex:project',
      consolidator: 'claude:opus',
      verifier: 'claude:sonnet',
      worker: 'claude:user-worker',
      codex_effort: 'xhigh',
      rounds: 4,
      max_workers: 5,
      design: { rounds: 1, stages: { survey: { engine: 'claude:user', effort: 'high' } } },
      runtime: {
        codex_reasoning: 'medium',
        plan_timeout_s: 10,
        worker_timeout_s: 20,
        verify_timeout_s: 30,
      },
    })
  })

  it('uses default intensity and timeout values when optional layers are absent', () => {
    expect(resolveCouncilConfig()).toMatchObject({
      intensity: 'standard',
      planner_a: 'claude:opus',
      planner_b: 'codex:gpt-5.5',
      consolidator: 'claude:opus',
      worker: 'claude:haiku',
      verifier: 'claude:sonnet',
      codex_effort: 'high',
      rounds: 2,
      max_workers: 6,
      runtime: {
        codex_reasoning: 'high',
        plan_timeout_s: 1200,
        worker_timeout_s: 1800,
        verify_timeout_s: 600,
      },
    })
  })

  it('reports invalid intensity, env, and required resolved values', () => {
    expect(() => resolveCouncilConfig({ flags: { intensity: 'bad' as never } })).toThrow('unknown intensity')
    expect(() =>
      resolveCouncilConfig({ env: new TestEnv({ COUNCIL_PLAN_TIMEOUT_S: '12s' }) }),
    ).toThrow('COUNCIL_PLAN_TIMEOUT_S must be an integer')
    expect(() => resolveCouncilConfig({ flags: { planner_a: 1 as never } })).toThrow('planner_a must be a string')
    expect(() => resolveCouncilConfig({ flags: { rounds: 'many' as never } })).toThrow('rounds must be a number')
    expect(() => resolveCouncilConfig({ flags: { codex_effort: 'bad' as never } })).toThrow(
      'codex_effort must be one of',
    )
  })

  it('ignores undefined values while merging config layers', () => {
    const resolved = resolveCouncilConfig({
      user: { design: { rounds: 1 }, worker: 'claude:user-worker' },
      project: { design: { rounds: undefined as never }, worker: undefined as never },
    })

    expect(resolved.design).toEqual({ rounds: 1 })
    expect(resolved.worker).toBe('claude:user-worker')
  })
})

describe('coerceConfigValue', () => {
  it('coerces supported config set values', () => {
    expect(coerceConfigValue('intensity', 'quick')).toBe('quick')
    expect(coerceConfigValue('rounds', '3')).toBe(3)
    expect(coerceConfigValue('max_workers', '7')).toBe(7)
    expect(coerceConfigValue('codex_effort', 'xhigh')).toBe('xhigh')
    expect(coerceConfigValue('planner_a', 'claude:opus')).toBe('claude:opus')
    expect(coerceConfigValue('worker', 'codex:gpt-5.5')).toBe('codex:gpt-5.5')
  })

  it('rejects invalid config set values', () => {
    expect(() => coerceConfigValue('missing', 'value')).toThrow('unknown key')
    expect(() => coerceConfigValue('intensity', 'slow')).toThrow('unknown intensity')
    expect(() => coerceConfigValue('rounds', '3.5')).toThrow('rounds must be an integer')
    expect(() => coerceConfigValue('codex_effort', 'huge')).toThrow('codex_effort must be one of')
    expect(() => coerceConfigValue('verifier', 'gpt-5.5')).toThrow('verifier must be claude:<model>')
  })
})
