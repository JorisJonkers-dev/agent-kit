import { describe, expect, it } from 'vitest'

import { agyOutput, claudeOutput, openCodeOutput, vibeOutput } from './outputs.js'

const NO_TOKENS = { cached: 0, input: 0, output: 0 }

describe('vibeOutput', () => {
  it('takes the answer from the journal, and the tokens and dollars from the export', () => {
    const journal = JSON.stringify([{ content: [{ text: 'done' }], role: 'assistant', type: 'message' }])

    expect(vibeOutput(journal, { cost_usd: 0.5, usage: { cached_input_tokens: 1, input_tokens: 2, output_tokens: 3 } })).toEqual({
      answer: 'done',
      charged: 0.5,
      tokens: { cached: 1, input: 2, output: 3 },
    })
  })

  it('has no tokens and no dollars without an export', () => {
    expect(vibeOutput('not a journal', undefined)).toEqual({ answer: undefined, charged: undefined, tokens: NO_TOKENS })
  })
})

describe('agyOutput', () => {
  it('reads the response and the usage, and prices nothing', () => {
    const stdout = JSON.stringify({
      response: 'pong\n',
      status: 'SUCCESS',
      usage: { cache_read_tokens: 5, input_tokens: 7661, output_tokens: 29, thinking_tokens: 28 },
    })

    expect(agyOutput(stdout)).toEqual({
      answer: 'pong\n',
      charged: undefined,
      failure: undefined,
      tokens: { cached: 5, input: 7661, output: 29 },
    })
  })

  it('reads a result with no usage as no tokens', () => {
    expect(agyOutput(JSON.stringify({ response: 'x' }))).toMatchObject({ answer: 'x', tokens: NO_TOKENS })
  })

  it('reports the failures agy hides behind a zero exit code', () => {
    expect(agyOutput(JSON.stringify({ finished_reason: 'print_timeout', response: 'x' })).failure).toBe('timeout')
    expect(agyOutput(JSON.stringify({ denied_actions: [{ action: 'write_file' }], response: 'x' })).failure).toBe('denied-action')
    expect(agyOutput(JSON.stringify({ response: 'x', status: 'ERROR' })).failure).toBe('client-error')
    expect(agyOutput(JSON.stringify({ denied_actions: [], response: 'x', status: 'SUCCESS' })).failure).toBeUndefined()
  })

  it('has no answer for an empty response, a list or text that is not JSON', () => {
    expect(agyOutput(JSON.stringify({ response: ' \n' })).answer).toBeUndefined()
    expect(agyOutput('[]')).toEqual({ answer: undefined, charged: undefined, tokens: NO_TOKENS })
    expect(agyOutput('oops')).toEqual({ answer: undefined, charged: undefined, tokens: NO_TOKENS })
  })
})

describe('claudeOutput', () => {
  const result = {
    is_error: false,
    result: 'fine',
    total_cost_usd: 0.042,
    type: 'result',
    usage: { cache_creation_input_tokens: 20, cache_read_input_tokens: 300, input_tokens: 10, output_tokens: 40 },
  }

  it('counts cached prompt in the input, and takes the dollars Claude Code charged', () => {
    expect(claudeOutput(JSON.stringify(result))).toEqual({
      answer: 'fine',
      charged: 0.042,
      failure: undefined,
      tokens: { cached: 300, input: 330, output: 40 },
    })
  })

  it('finds the result among a list of events', () => {
    expect(claudeOutput(JSON.stringify([{ type: 'system' }, result, { type: 'other' }])).answer).toBe('fine')
    expect(claudeOutput(JSON.stringify([{ type: 'system' }]))).toEqual({ answer: undefined, charged: undefined, tokens: NO_TOKENS })
  })

  it('reports an error result, and prices nothing when no dollars came back', () => {
    expect(claudeOutput(JSON.stringify({ ...result, is_error: true })).failure).toBe('client-error')
    expect(claudeOutput(JSON.stringify({ ...result, total_cost_usd: 0 })).charged).toBeUndefined()
    expect(claudeOutput(JSON.stringify({ result: 'x' }))).toMatchObject({ answer: 'x', charged: undefined, tokens: NO_TOKENS })
  })

  it('has no answer for text that is not JSON or a blank result', () => {
    expect(claudeOutput('oops').answer).toBeUndefined()
    expect(claudeOutput(JSON.stringify({ result: '' })).answer).toBeUndefined()
  })
})

describe('openCodeOutput', () => {
  const lines = (...events: unknown[]): string => events.map((event) => JSON.stringify(event)).join('\n')
  const finish = (cost: number, tokens: Record<string, unknown>): unknown => ({ part: { cost, tokens }, type: 'step_finish' })

  it('adds up every step’s dollars and tokens, and answers with the last step that wrote text', () => {
    const output = openCodeOutput(
      lines(
        { type: 'step_start' },
        { part: { text: 'first' }, type: 'text' },
        finish(0.01, { cache: { read: 5, write: 1 }, input: 10, output: 2, reasoning: 1 }),
        { type: 'step_start' },
        { part: { text: 'second ' }, type: 'text' },
        { part: { text: 'half' }, type: 'text' },
        finish(0.02, { input: 20, output: 4 }),
        { type: 'step_start' },
        finish(0, { input: 1, output: 1 }),
      ),
    )

    expect(output).toEqual({
      answer: 'second half',
      charged: 0.03,
      failure: undefined,
      tokens: { cached: 5, input: 37, output: 8 },
    })
  })

  it('fails on an error event, and skips lines and events it cannot read', () => {
    const output = openCodeOutput(`not json\n${lines('a string', null, { type: 'error' }, { part: { text: 7 }, type: 'text' }, { type: 'step_finish' })}`)

    expect(output).toEqual({ answer: undefined, charged: undefined, failure: 'client-error', tokens: NO_TOKENS })
  })

  it('answers with text that comes before any step starts', () => {
    expect(openCodeOutput(lines({ part: { text: 'a' }, type: 'text' }, { part: { text: 'b' }, type: 'text' })).answer).toBe('ab')
  })

  it('has no answer for an empty output', () => {
    expect(openCodeOutput('')).toEqual({ answer: undefined, charged: undefined, tokens: NO_TOKENS })
  })
})
