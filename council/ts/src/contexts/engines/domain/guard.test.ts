import { describe, expect, it } from 'vitest'

import { commandEngine, modelPicks, wrappedEngine, EngineGuardError, engineWorkflowName, GUARDED_ENGINES, isGuardedEngine, parseEngineSpec } from './guard.js'

describe('guarded engines', () => {
  it('guards claude and codex and leaves every other engine to council', () => {
    expect(GUARDED_ENGINES).toEqual(['claude', 'codex'])
    expect(isGuardedEngine('codex')).toBe(true)
    expect(isGuardedEngine('claude')).toBe(true)
    expect(isGuardedEngine('generic')).toBe(false)
  })
})

describe('parseEngineSpec', () => {
  it('reads an engine and its model', () => {
    expect(parseEngineSpec('codex:gpt-5.5')).toEqual({ model: 'gpt-5.5', name: 'codex' })
    expect(parseEngineSpec('claude:claude-haiku-5-5')).toEqual({ model: 'claude-haiku-5-5', name: 'claude' })
  })

  it('keeps everything after the first colon as the model', () => {
    expect(parseEngineSpec('generic:a:b')).toEqual({ model: 'a:b', name: 'generic' })
  })

  it('refuses an engine without a model, or a model without an engine', () => {
    expect(() => parseEngineSpec('codex')).toThrow('--engine must name an engine and its model')
    expect(() => parseEngineSpec('codex:')).toThrow('--engine must name an engine and its model')
    expect(() => parseEngineSpec(':gpt-5.5')).toThrow('--engine must name an engine and its model')
  })
})

describe('engineWorkflowName', () => {
  it('keeps a run id the gateway already accepts', () => {
    expect(engineWorkflowName('run-2026.11.04_a:b/c')).toBe('run-2026.11.04_a:b/c')
  })

  it('turns characters the gateway refuses into dashes and drops a leading one', () => {
    expect(engineWorkflowName('my run (1)')).toBe('my-run--1-')
    expect(engineWorkflowName('..hidden')).toBe('hidden')
  })

  it('cuts a long run id to the gateway’s 80 characters', () => {
    expect(engineWorkflowName('a'.repeat(120))).toBe('a'.repeat(80))
  })

  it('falls back to a name of its own when nothing of the id is usable', () => {
    expect(engineWorkflowName('')).toBe('council')
    expect(engineWorkflowName('---')).toBe('council')
  })
})

describe('commandEngine', () => {
  it('names the guarded engine a command is, by its file name, and no engine for any other command', () => {
    expect(commandEngine('codex')).toBe('codex')
    expect(commandEngine('/usr/local/bin/claude')).toBe('claude')
    expect(commandEngine('sh')).toBeUndefined()
    expect(commandEngine('/opt/codex-wrapper/run')).toBeUndefined()
  })
})

describe('EngineGuardError', () => {
  it('carries the reason council refused for', () => {
    const error = new EngineGuardError('no-engine-guard', 'no guard')

    expect(error).toMatchObject({ code: 'no-engine-guard', message: 'no guard', name: 'EngineGuardError' })
  })
})

describe('wrappedEngine', () => {
  it.each([
    ['env', ['codex', 'exec']],
    ['env', ['FOO=1', '/usr/local/bin/claude']],
    ['nohup', ['codex']],
    ['xargs', ['claude']],
    ['sudo', ['-u', 'x', 'codex']],
    ['command', ['codex']],
    ['exec', ['claude']],
    ['time', ['codex']],
    ['npx', ['codex']],
    ['node', ['/opt/bin/codex']],
    ['python3', ['codex']],
    ['git', ['codex']],
    ['/bin/ls', ['/x/claude']],
  ])('names the engine %s would start from %j', (command, args) => {
    expect(wrappedEngine(command, args)).toBeDefined()
  })

  it.each([
    ['sh', ['-c', 'codex exec -'], 'codex'],
    ['bash', ['-lc', 'FOO=1 claude -p'], 'claude'],
    ['zsh', ['-c', 'cd /repo && codex exec'], 'codex'],
    ['env', ['sh', '-c', 'exec claude'], 'claude'],
    ['sudo', ['bash', '-c', 'codex'], 'codex'],
  ])('finds the word in the shell string %s %j', (command, args, engine) => {
    expect(wrappedEngine(command, args)).toBe(engine)
  })

  it('leaves a command that starts no engine alone', () => {
    expect(wrappedEngine('node', ['-e', 'console.log(1)'])).toBeUndefined()
    expect(wrappedEngine('sh', ['-c', 'npm test'])).toBeUndefined()
    expect(wrappedEngine('git', ['status'])).toBeUndefined()
    expect(wrappedEngine('git', ['commit', '-m', 'ask codexes'])).toBeUndefined()
  })
})

describe('modelPicks', () => {
  it('reads every way a command picks its model', () => {
    expect(modelPicks('codex', ['exec', '-m', 'a', '--model', 'b', '--model=c']).models).toEqual(['a', 'b', 'c'])
    expect(modelPicks('codex', ['-c', 'model=d', '--config', 'model="e"', "--config=model='f'", '-c', ' model = g ']).models).toEqual(['d', 'e', 'f', 'g'])
  })

  it('takes a config that sets something else, or no key at all, as no pick', () => {
    expect(modelPicks('codex', ['-c', 'model_reasoning_effort=high', '-c', 'nothing', '-c']).models).toEqual([])
    expect(modelPicks('codex', ['-m']).models).toEqual([])
  })

  it('calls a profile and a provider override what they are', () => {
    expect(modelPicks('codex', ['-p', 'fast', '--profile', 'x', '--profile=y']).overrides).toEqual(['-p', '--profile', '--profile=y'])
    expect(modelPicks('codex', ['-c', 'model_provider=oss', '--config=model_provider="x"']).overrides).toEqual(['model_provider=oss', 'model_provider="x"'])
  })

  it('leaves claude’s -p, which is print mode, alone but refuses its --profile', () => {
    expect(modelPicks('claude', ['-p', '--model', 'haiku'])).toEqual({ models: ['haiku'], overrides: [] })
    expect(modelPicks('claude', ['--profile', 'x']).overrides).toEqual(['--profile'])
  })
})
