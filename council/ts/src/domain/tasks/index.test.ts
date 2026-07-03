import { describe, expect, it } from 'vitest'

import {
  assertTasksBijection,
  assertTasksJsonSchema,
  parseTasksMd,
  renderTasksMd,
  TASKS_JSON_SCHEMA,
  validateTasks,
  validateTasksJsonSchema,
} from './index.js'
import type { JsonRecord } from '../contracts/common.js'

const fullTask: JsonRecord = {
  acceptance_criteria: ['keeps bijection'],
  archetype: 'implementation',
  boundaries: 'Stay in scope',
  content_hash: 'sha256:abc',
  context_profile: 'focused',
  context_refs: ['ctx-1'],
  depends_on: [],
  dev_notes: 'note',
  difficulty: 'moderate',
  discovered_from: 'planner',
  engine: { cli: 'codex', label: 'worker', model: 'gpt-5' },
  id: 'T1',
  model: 'haiku',
  model_tier: 'cheap',
  objective: 'Change exactly one thing',
  output_format: 'Code edits',
  paths: ['council/ts/src/domain/tasks'],
  spec_ref: '007-sdd-aware-council',
  supersedes: ['ck-abcd'],
  title: 'Implement one thing',
  verify: 'npm test',
}

const dependentTask: JsonRecord = {
  boundaries: 'Stay in scope',
  depends_on: ['T1'],
  difficulty: 'trivial',
  id: 'T2',
  model: 'sonnet',
  objective: 'Follow up',
  output_format: 'Code edits',
  paths: ['council/ts/src/domain/tasks'],
  title: 'Follow up',
  verify: 'npm test',
}

const independentTask: JsonRecord = {
  boundaries: 'Stay in scope',
  depends_on: [],
  difficulty: 'trivial',
  id: 'T3',
  model: 'opus',
  objective: 'Independent',
  output_format: 'Code edits',
  paths: ['council/ts/src/domain/tasks'],
  title: 'Independent',
  verify: 'npm test',
}

describe('tasks markdown bijection', () => {
  it('renders sorted embedded JSON and round-trips a task with every optional field', () => {
    const markdown = renderTasksMd([fullTask], { name: '007-sdd-aware-council' })

    expect(markdown).toBe(`# Tasks: 007-sdd-aware-council

<!-- council-tasks-format: v1 -->

## T1: Implement one thing
<!-- council-task-id: T1 -->
\`\`\`json
{
  "acceptance_criteria": [
    "keeps bijection"
  ],
  "archetype": "implementation",
  "boundaries": "Stay in scope",
  "content_hash": "sha256:abc",
  "context_profile": "focused",
  "context_refs": [
    "ctx-1"
  ],
  "depends_on": [],
  "dev_notes": "note",
  "difficulty": "moderate",
  "discovered_from": "planner",
  "engine": {
    "cli": "codex",
    "label": "worker",
    "model": "gpt-5"
  },
  "id": "T1",
  "model": "haiku",
  "model_tier": "cheap",
  "objective": "Change exactly one thing",
  "output_format": "Code edits",
  "paths": [
    "council/ts/src/domain/tasks"
  ],
  "spec_ref": "007-sdd-aware-council",
  "supersedes": [
    "ck-abcd"
  ],
  "title": "Implement one thing",
  "verify": "npm test"
}
\`\`\`
`)
    expect(parseTasksMd(markdown)).toEqual([fullTask])
    expect(() => { assertTasksBijection([fullTask], markdown); }).not.toThrow()
  })

  it('uses council and id fallbacks while preserving non-ASCII as JSON escapes', () => {
    const task: JsonRecord = {
      depends_on: [],
      engine: {},
      flag: true,
      id: 'T1',
      model: 'haiku',
      nullable: null,
      objective: 42,
      paths: [],
      summary: 'café',
      verify: 'npm test',
    }

    expect(renderTasksMd([task])).toContain('# Tasks: council')
    expect(renderTasksMd([task])).toContain('## T1: T1')
    expect(renderTasksMd([task])).toContain('"engine": {}')
    expect(renderTasksMd([task])).toContain('"flag": true')
    expect(renderTasksMd([task])).toContain('"nullable": null')
    expect(renderTasksMd([task])).toContain('"objective": 42')
    expect(renderTasksMd([task])).toContain('"summary": "caf\\u00e9"')
    expect(renderTasksMd([{ ...task, id: null }])).toContain('## null: null')
    expect(renderTasksMd([{ ...fullTask, id: { nested: 'id' } }])).toContain('## {\n  "nested": "id"\n}')
  })

  it('rejects malformed task markdown and mismatched objects', () => {
    const markdown = renderTasksMd([fullTask])
    expect(() => parseTasksMd('no task blocks')).toThrow('no council task JSON blocks found in tasks.md')
    expect(() =>
      parseTasksMd(markdown.replace('<!-- council-task-id: T1 -->', '<!-- council-task-id: T2 -->')),
    ).toThrow("task marker mismatch: header 'T1', marker 'T2'")
    expect(() => parseTasksMd(markdown.replace('"id": "T1"', '"id": '))).toThrow(
      "task 'T1' JSON block is invalid:",
    )
    expect(() => parseTasksMd(markdown.replace(/```json\n[\s\S]*?\n```/, '```json\n[]\n```'))).toThrow(
      "task 'T1' JSON block must be an object",
    )
    expect(() => parseTasksMd(markdown.replace('"id": "T1"', '"id": "T2"'))).toThrow(
      "task 'T1' JSON id does not match marker",
    )
    expect(() =>
      parseTasksMd(markdown.replace(/ {2}"id": "T1",\n/, '')),
    ).toThrow("task 'T1' JSON id does not match marker")
    expect(() => parseTasksMd(`${markdown}\n${markdown}`)).toThrow('duplicate task id in tasks.md: T1')
    expect(() => { assertTasksBijection([fullTask], markdown.replace('Change exactly one thing', 'Change more')); }).toThrow(
      'tasks.md does not match tasks.json',
    )
    expect(() =>
      { assertTasksBijection([fullTask], markdown.replace('"depends_on": []', '"depends_on": ["T9"]')); },
    ).toThrow("task 'T1' depends on unknown task 'T9'")
  })

  it('fails when asked to render non-JSON values', () => {
    const badTask = {
      ...fullTask,
      extra: undefined,
    } as unknown as JsonRecord

    expect(() => renderTasksMd([badTask])).toThrow('cannot serialize non-JSON value: undefined')
  })
})

describe('validateTasks', () => {
  it('accepts schema-loose payloads as the authoritative structural DAG gate', () => {
    const schemaLoose = {
      depends_on: [],
      extra: 'schema rejects this',
      id: 'custom',
      model: 'custom-model',
      objective: 42,
      paths: 'not a schema array',
      verify: 'npm test',
    }

    expect(() => { validateTasks([schemaLoose]); }).not.toThrow()
    expect(validateTasksJsonSchema([schemaLoose])).toMatchObject({ valid: false })
  })

  it('rejects empty, non-object, missing, duplicate, and unchecked tasks', () => {
    const warnings: string[] = []

    expect(() => { validateTasks([]); }).toThrow('consolidator returned no tasks')
    expect(() => { validateTasks([null]); }).toThrow('task ? must be an object')
    expect(() => { validateTasks([{ id: 'T1' }]); }).toThrow(
      "task T1 missing fields: ['objective', 'depends_on', 'paths', 'model', 'verify']",
    )
    expect(() => { validateTasks([fullTask, fullTask]); }).toThrow('duplicate task id: T1')
    expect(() => { validateTasks([{ ...fullTask, verify: '  ' }], { onWarning: (message) => warnings.push(message) }); })
      .not.toThrow()
    expect(warnings).toEqual([
      'warning: task T1 has no verify command - its result is unchecked except by the adversarial verifier',
    ])
  })

  it('rejects invalid dependency structure, unknown dependencies, and cycles', () => {
    expect(() => { validateTasks([{ ...fullTask, depends_on: 'T2' }]); }).toThrow(
      "task 'T1' depends_on must be an array",
    )
    expect(() => { validateTasks([{ ...fullTask, depends_on: ['T9'] }]); }).toThrow(
      "task 'T1' depends on unknown task 'T9'",
    )
    expect(() => { validateTasks([{ ...fullTask, depends_on: ['T2'] }, dependentTask]); }).toThrow(
      "dependency cycle among tasks: ['T1', 'T2']",
    )
    expect(() => { validateTasks([dependentTask, independentTask, fullTask]); }).not.toThrow()
  })
})

describe('tasks JSON Schema layer', () => {
  it('exports the secondary schema and accepts fully shaped tasks', () => {
    expect(TASKS_JSON_SCHEMA.$comment).toContain('SECONDARY')
    expect(validateTasksJsonSchema([fullTask, dependentTask])).toEqual({ valid: true, errors: [] })
    expect(() => { assertTasksJsonSchema([fullTask]); }).not.toThrow()
  })

  it('reports schema errors separately from validateTasks', () => {
    const result = validateTasksJsonSchema([
      null,
      {
        ...fullTask,
        acceptance_criteria: 'wrong',
        context_refs: 'wrong',
        depends_on: ['bad'],
        difficulty: 'huge',
        extra: true,
        id: 'bad',
        model: 'llama',
        paths: [1],
        supersedes: ['bad'],
        title: 1,
      },
    ])

    expect(result.valid).toBe(false)
    expect(result.errors).toEqual([
      '$[0] must be an object',
      '$[1].extra is not allowed by schema',
      '$[1].title must be a string',
      '$[1].paths must be an array of strings',
      '$[1].acceptance_criteria must be an array of strings',
      '$[1].context_refs must be an array of strings',
      '$[1].id must match a council task id',
      '$[1].depends_on[0] must match a council task id',
      '$[1].supersedes[0] must match a council task id',
      '$[1].difficulty must be trivial, moderate, or hard',
      '$[1].model must be haiku, sonnet, or opus',
    ])
    expect(validateTasksJsonSchema('not array')).toEqual({ valid: false, errors: ['tasks must be an array'] })
    expect(() => { assertTasksJsonSchema([{ ...fullTask, id: 'bad' }]); }).toThrow(
      'tasks JSON Schema validation failed: $[0].id must match a council task id',
    )
  })

  it('reports missing required fields', () => {
    expect(validateTasksJsonSchema([{ id: 'T1' }]).errors).toEqual([
      '$[0].title is required',
      '$[0].objective is required',
      '$[0].output_format is required',
      '$[0].paths is required',
      '$[0].depends_on is required',
      '$[0].difficulty is required',
      '$[0].model is required',
      '$[0].verify is required',
      '$[0].boundaries is required',
    ])
  })
})
