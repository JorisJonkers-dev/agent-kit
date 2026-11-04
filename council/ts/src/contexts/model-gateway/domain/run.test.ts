import { describe, expect, it } from 'vitest'

import { ModelGatewayError } from './config.js'
import {
  compactTimestamp,
  DEFAULT_TIME_LIMIT_S,
  finalAnswerOf,
  NO_TOKENS,
  readOnlyProof,
  type ProofFiles,
  readRunInput,
  runCost,
  runFailureReason,
  runId,
  runTokens,
  summaryOf,
  wrappedPrompt,
} from './run.js'

/** The error a call threw, so a test can name its code. */
function thrown(call: () => unknown): ModelGatewayError {
  try {
    call()
  } catch (error) {
    if (error instanceof ModelGatewayError) return error
    throw error
  }
  throw new Error('the call did not throw')
}

const MINIMAL = { client: 'vibe', mode: 'read-only', model: 'zai-glm-5-3', prompt: 'read this', workflow: 'spec-293' }

describe('readRunInput', () => {
  it('reads the input a caller sends, with the default time limit', () => {
    expect(readRunInput(MINIMAL)).toEqual({
      carries: undefined,
      client: 'vibe',
      max_price_usd: undefined,
      mode: 'read-only',
      model: 'zai-glm-5-3',
      prompt: 'read this',
      repo: undefined,
      time_limit_s: DEFAULT_TIME_LIMIT_S,
      workflow: 'spec-293',
    })
  })

  it('reads every optional field', () => {
    const input = readRunInput({
      ...MINIMAL,
      carries: { code: true, history: false },
      max_price_usd: 0.5,
      repo: '/work/agent-kit',
      time_limit_s: 60,
    })

    expect(input).toMatchObject({
      carries: { code: true, history: false },
      max_price_usd: 0.5,
      repo: '/work/agent-kit',
      time_limit_s: 60,
    })
  })

  it('refuses a call that names no workflow', () => {
    expect(thrown(() => readRunInput({ ...MINIMAL, workflow: '' })).code).toBe('invalid-input')
    expect(thrown(() => readRunInput({ ...MINIMAL, workflow: undefined })).message).toBe('a run must name its workflow')
  })

  it('refuses a client or a mode a later ticket builds', () => {
    expect(thrown(() => readRunInput({ ...MINIMAL, client: 'codex' })).code).toBe('not-supported-yet')
    expect(thrown(() => readRunInput({ ...MINIMAL, client: 'codex' })).message).toBe('only vibe can run a delegation, not "codex"')
    expect(thrown(() => readRunInput({ ...MINIMAL, mode: 'writing' })).code).toBe('not-supported-yet')
    expect(thrown(() => readRunInput({ ...MINIMAL, mode: undefined })).message).toBe('only read-only runs are supported, not missing')
  })

  it('refuses an input that is not an object, or that names no model or prompt', () => {
    expect(thrown(() => readRunInput([])).message).toBe('the input must be a JSON object')
    expect(thrown(() => readRunInput({ ...MINIMAL, model: '' })).message).toBe('model must be a non-empty string')
    expect(thrown(() => readRunInput({ ...MINIMAL, prompt: '' })).message).toBe('prompt must be a non-empty string')
  })

  it('refuses a repo, a carries mark, a time limit or a price that is not what it says', () => {
    expect(thrown(() => readRunInput({ ...MINIMAL, repo: '' })).message).toBe('repo must be a non-empty string')
    expect(thrown(() => readRunInput({ ...MINIMAL, carries: { code: true } })).message).toBe(
      'carries must name code and history, each true or false',
    )
    expect(thrown(() => readRunInput({ ...MINIMAL, time_limit_s: 1.5 })).message).toBe('time_limit_s must be a whole number')
    expect(thrown(() => readRunInput({ ...MINIMAL, time_limit_s: 0 })).message).toBe(
      'time_limit_s must be a whole number of seconds above 0',
    )
    expect(thrown(() => readRunInput({ ...MINIMAL, max_price_usd: 0 })).message).toBe('max_price_usd must be a number above 0')
  })
})

describe('the prompt and the summary', () => {
  it('wraps the caller’s prompt with the instruction that ends the answer with a SUMMARY line', () => {
    expect(wrappedPrompt('read this', 1500)).toBe(
      'read this\n\nEnd your answer with a line "SUMMARY:" followed by at most 1500 characters that summarise the result.',
    )
  })

  it('takes the text after the last SUMMARY line, cut at the limit', () => {
    expect(summaryOf('work\nSUMMARY: it is fine', 1500)).toBe('it is fine')
    expect(summaryOf('SUMMARY: first\nSUMMARY: second', 1500)).toBe('second')
    expect(summaryOf('SUMMARY: 0123456789', 4)).toBe('0123')
  })

  it('falls back to the last characters of the answer when the delegate wrote no SUMMARY line', () => {
    expect(summaryOf('0123456789', 4)).toBe('6789')
  })
})

describe('finalAnswerOf', () => {
  it('joins the last assistant message of a Vibe journal', () => {
    const journal = JSON.stringify([
      { content: [{ text: 'first' }], role: 'assistant', type: 'message' },
      { content: [{ text: 'the ' }, { text: 'answer' }], role: 'assistant', type: 'message' },
      { content: [{ text: 'a tool result' }], role: 'tool', type: 'message' },
    ])

    expect(finalAnswerOf(journal)).toBe('the answer')
  })

  it('says nothing when stdout is not a journal, holds no assistant message, or holds an empty one', () => {
    expect(finalAnswerOf('not json')).toBeUndefined()
    expect(finalAnswerOf('{"type":"message"}')).toBeUndefined()
    expect(finalAnswerOf(JSON.stringify([{ content: [{ text: 'x' }], role: 'user', type: 'message' }]))).toBeUndefined()
    expect(finalAnswerOf(JSON.stringify([{ content: [], role: 'assistant', type: 'message' }]))).toBeUndefined()
    expect(finalAnswerOf(JSON.stringify([{ role: 'assistant', type: 'message' }]))).toBeUndefined()
  })
})

describe('the tokens and the dollars a run used', () => {
  it('reads the export’s token counts', () => {
    expect(runTokens({ usage: { cached_input_tokens: 10, input_tokens: 100, output_tokens: 20 } })).toEqual({
      cached: 10,
      input: 100,
      output: 20,
    })
  })

  it('counts no tokens when the export holds none, or holds nonsense', () => {
    expect(runTokens(undefined)).toEqual(NO_TOKENS)
    expect(runTokens({})).toEqual(NO_TOKENS)
    expect(runTokens({ usage: { input_tokens: -1, output_tokens: 'many' } })).toEqual(NO_TOKENS)
  })

  it('takes Vibe’s own figure when it priced the run', () => {
    expect(runCost({ cost_usd: 0.42 }, { cached: 0, input: 1_000_000, output: 0 }, { cached_input: 0, input: 1, output: 1 })).toBe(0.42)
  })

  it('counts the tokens at the catalogue price when Vibe priced nothing', () => {
    const prices = { cached_input: 0.1, input: 1, output: 2 }

    expect(runCost({}, { cached: 500_000, input: 1_000_000, output: 1_000_000 }, prices)).toBeCloseTo(0.5 * 0.1 + 0.5 * 1 + 2, 10)
  })

  it('counts nothing when nothing prices the run', () => {
    expect(runCost({}, { cached: 0, input: 10, output: 10 }, undefined)).toBe(0)
  })
})

describe('runFailureReason', () => {
  it('names a timeout, an exit code, or an answer it could not read', () => {
    expect(runFailureReason({ exitCode: 124, timedOut: true }, 'text')).toBe('timeout')
    expect(runFailureReason({ exitCode: 3 }, 'text')).toBe('exit-3')
    expect(runFailureReason({ exitCode: 0 }, undefined)).toBe('parse-error')
    expect(runFailureReason({ exitCode: 0 }, 'text')).toBeUndefined()
  })
})

describe('the run id', () => {
  it('is the UTC moment the run started, and a short suffix', () => {
    expect(compactTimestamp(new Date('2026-11-04T10:52:15.123Z'))).toBe('20261104T105215Z')
    expect(runId(new Date('2026-11-04T10:52:15.123Z'), 'a1b2c3')).toBe('20261104T105215Z-a1b2c3')
  })
})

/** Critique's routing, as the shipped file holds it: one Vibe client per model. */
const ROUTING = JSON.stringify({
  clients: {
    'agy-gemini-flash-high': { kind: 'agy', model: 'gemini-3.8-flash-high' },
    'vibe-glm': { kind: 'vibe', model: 'zai-glm-5-3' },
    'vibe-mistral-small': { kind: 'vibe', model: 'mistral-small-latest' },
  },
})

/** The proof files a test names, with the ones it leaves out missing. */
function proofFiles(files: Partial<ProofFiles>): ProofFiles {
  return { routing: undefined, routingOverride: undefined, setupResults: undefined, ...files }
}

/** Setup's results, with the read-only proof of one critic client recorded as `result`. */
function setupResults(result: string, client = 'vibe-glm'): string {
  return JSON.stringify({ checks: { [`read-only:${client}`]: { result } }, version: 1 })
}

describe('readOnlyProof', () => {
  it('is the proof of the Vibe critic client whose model this is', () => {
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('pass') }), 'zai-glm-5-3')).toEqual({
      critic: 'vibe-glm',
      status: 'proven',
    })
  })

  it('finds the proof of an alias by the model id the routing names among its names', () => {
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('pass') }), 'glm-5-3', ['zai-glm-5-3'])).toEqual({
      critic: 'vibe-glm',
      status: 'proven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('pass') }), 'glm-5-3').status).toBe('unproven')
  })

  it('is unproven when critique’s routing is not installed, or is not JSON', () => {
    expect(readOnlyProof(proofFiles({}), 'zai-glm-5-3')).toEqual({
      reason: "critique's routing is not installed, so no Vibe critic client proves a model read-only",
      status: 'unproven',
    })
    expect(readOnlyProof(proofFiles({ routing: 'not json' }), 'zai-glm-5-3').status).toBe('unproven')
  })

  it('is unproven when no Vibe critic client runs the model', () => {
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('pass') }), 'mistral-large-4-0')).toEqual({
      reason: "no Vibe critic client in critique's routing runs mistral-large-4-0",
      status: 'unproven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING }), 'gemini-3.8-flash-high').status).toBe('unproven')
  })

  it('is unproven when setup recorded no proof, a failed one, or one it could not show', () => {
    expect(readOnlyProof(proofFiles({ routing: ROUTING }), 'zai-glm-5-3')).toEqual({
      reason: "setup's read-only proof for vibe-glm is not recorded",
      status: 'unproven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('fail') }), 'zai-glm-5-3').status).toBe('unproven')
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: setupResults('unproven') }), 'zai-glm-5-3')).toEqual({
      reason: "setup's read-only proof for vibe-glm is unproven",
      status: 'unproven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING, setupResults: 'not json' }), 'zai-glm-5-3').status).toBe('unproven')
  })

  it('follows this machine’s routing override, which may move a client to another model', () => {
    const override = JSON.stringify({ clients: { 'vibe-glm': { model: 'zai-glm-6-0' } } })

    expect(readOnlyProof(proofFiles({ routing: ROUTING, routingOverride: override, setupResults: setupResults('pass') }), 'zai-glm-6-0')).toEqual({
      critic: 'vibe-glm',
      status: 'proven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING, routingOverride: override, setupResults: setupResults('pass') }), 'zai-glm-5-3').status).toBe(
      'unproven',
    )
  })

  it('reads an override that adds a client, and ignores one that is not an object', () => {
    const added = JSON.stringify({ clients: { 'vibe-new': { kind: 'vibe', model: 'mistral-medium-4-1' } } })

    expect(readOnlyProof(proofFiles({ routing: ROUTING, routingOverride: added, setupResults: setupResults('pass', 'vibe-new') }), 'mistral-medium-4-1')).toEqual({
      critic: 'vibe-new',
      status: 'proven',
    })
    expect(readOnlyProof(proofFiles({ routing: ROUTING, routingOverride: '[]' }), 'zai-glm-5-3').status).toBe('unproven')
  })
})
