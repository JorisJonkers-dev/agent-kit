import { describe, expect, it } from 'vitest'

import { getEngineDef, parseEngineRegistryConfig } from './index.js'

const stdinArgv = ['engine', '--model', '{model}', '--effort', '{effort}', '--out', '{output}'] as const
const promptFileArgv = [
  'engine',
  '--model',
  '{model}',
  '--effort',
  '{effort}',
  '--prompt-file',
  '{prompt_file}',
  '--out',
  '{output}',
] as const

describe('parseEngineRegistryConfig', () => {
  it('pre-registers built-in claude and codex engines', () => {
    const registry = parseEngineRegistryConfig()

    expect(registry.engines.claude).toEqual({
      name: 'claude',
      argv: [
        'sh',
        '-lc',
        'COUNCIL_EFFORT={effort} claude -p --model {model} --output-format json --permission-mode plan < {prompt_file} > {output}',
      ],
      promptDelivery: 'prompt_file',
      streamFormat: 'json',
      resultExtraction: { mode: 'json_path', path: ['result'] },
    })
    expect(registry.engines.codex).toEqual({
      name: 'codex',
      argv: [
        'sh',
        '-lc',
        'codex exec -m {model} -c model_reasoning_effort={effort} --skip-git-repo-check -o {output} "$(cat {prompt_file})"',
      ],
      promptDelivery: 'prompt_file',
      streamFormat: 'text',
      resultExtraction: { mode: 'output_file' },
    })
  })

  it('accepts an empty root config and merges configured entries over built-ins', () => {
    const emptyRoot = parseEngineRegistryConfig({ engines: undefined })
    const registry = parseEngineRegistryConfig({
      engines: {
        claude: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          result_extraction: 'stdout',
          label: 'Claude via stdin',
        },
        local_runner: {
          argv: promptFileArgv,
          stream_format: 'json',
          result_extraction: 'json.message.content',
        },
      },
    })

    expect(emptyRoot.engines.codex?.name).toBe('codex')
    expect(registry.engines.claude).toEqual({
      name: 'claude',
      argv: stdinArgv,
      promptDelivery: 'stdin',
      streamFormat: 'text',
      resultExtraction: { mode: 'stdout' },
      label: 'Claude via stdin',
    })
    expect(registry.engines.local_runner).toEqual({
      name: 'local_runner',
      argv: promptFileArgv,
      promptDelivery: 'prompt_file',
      streamFormat: 'json',
      resultExtraction: { mode: 'json_path', path: ['message', 'content'] },
    })
  })

  it('accepts a direct entries object and rejects unknown engine lookup', () => {
    const registry = parseEngineRegistryConfig({
      custom: {
        argv: stdinArgv,
      },
    })

    expect(getEngineDef(registry, 'custom')).toEqual({
      name: 'custom',
      argv: stdinArgv,
      promptDelivery: 'stdin',
      streamFormat: 'text',
      resultExtraction: { mode: 'output_file' },
    })
    expect(() => getEngineDef(registry, 'missing')).toThrow('Unknown engine: missing')
  })

  it('requires object-shaped configs and engine maps', () => {
    expect(() => parseEngineRegistryConfig('bad')).toThrow('config must be an object')
    expect(() => parseEngineRegistryConfig({ engines: [] })).toThrow('engines must be an object')
  })

  it('validates engine names and entry objects', () => {
    expect(() =>
      parseEngineRegistryConfig({
        'bad:name': {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
        },
      }),
    ).toThrow('engines.bad:name has invalid engine name "bad:name"')
    expect(() => parseEngineRegistryConfig({ custom: null })).toThrow('engines.custom must be an object')
  })

  it('rejects unsupported engine config keys', () => {
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          timeout: 1,
        },
      }),
    ).toThrow('engines.custom.timeout is not a supported engine config key')
  })

  it('validates argv shape', () => {
    expect(() => parseEngineRegistryConfig({ custom: { prompt_delivery: 'stdin' } })).toThrow(
      'engines.custom.argv must be a non-empty string array',
    )
    expect(() => parseEngineRegistryConfig({ custom: { argv: [] } })).toThrow(
      'engines.custom.argv must be a non-empty string array',
    )
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', ''],
        },
      }),
    ).toThrow('engines.custom.argv[1] must be a non-empty string')
  })

  it('rejects unsafe and unsupported placeholders', () => {
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', '{prompt}', '{model}', '{effort}', '{output}'],
        },
      }),
    ).toThrow('must not inline prompts with {prompt}')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', '{model}', '{effort}', '{output}', '{cwd}'],
        },
      }),
    ).toThrow('contains unsupported placeholder {cwd}')
  })

  it('requires model, effort, and output placeholders', () => {
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', '{effort}', '{output}'],
        },
      }),
    ).toThrow('engines.custom.argv must include {model}')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', '{model}', '{output}'],
        },
      }),
    ).toThrow('engines.custom.argv must include {effort}')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: ['engine', '{model}', '{effort}'],
        },
      }),
    ).toThrow('engines.custom.argv must include {output}')
  })

  it('validates prompt delivery', () => {
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'file',
        },
      }),
    ).toThrow('engines.custom.prompt_delivery must be "prompt_file" or "stdin"')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'prompt_file',
        },
      }),
    ).toThrow('engines.custom.argv must include {prompt_file} when prompt_delivery is "prompt_file"')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: promptFileArgv,
          prompt_delivery: 'stdin',
        },
      }),
    ).toThrow('engines.custom.argv must not include {prompt_file} when prompt_delivery is "stdin"')
  })

  it('validates stream format, result extraction, and label', () => {
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          stream_format: 'xml',
        },
      }),
    ).toThrow('engines.custom.stream_format must be "json" or "text"')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          result_extraction: 'final_answer',
        },
      }),
    ).toThrow('engines.custom.result_extraction must be "output_file", "stdout", or a json.<field> path')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          result_extraction: 1,
        },
      }),
    ).toThrow('engines.custom.result_extraction must be "output_file", "stdout", or a json.<field> path')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          result_extraction: 'json.',
        },
      }),
    ).toThrow('engines.custom.result_extraction must be "output_file", "stdout", or a json.<field> path')
    expect(() =>
      parseEngineRegistryConfig({
        custom: {
          argv: stdinArgv,
          prompt_delivery: 'stdin',
          label: '',
        },
      }),
    ).toThrow('engines.custom.label must be a non-empty string')
  })
})
