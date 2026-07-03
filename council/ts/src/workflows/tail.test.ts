import { describe, expect, it } from 'vitest'

import { selectTaskLogTail, type TailLogSource } from './tail.js'

function bytes(text: string): Uint8Array {
  return new TextEncoder().encode(text)
}

function source(overrides: Partial<TailLogSource> = {}): TailLogSource {
  const buffer = overrides.buffer ?? bytes('alpha\nbeta\ngamma\n')
  return {
    buffer,
    bufferStartOffset: overrides.bufferStartOffset ?? 0,
    sizeBytes: overrides.sizeBytes ?? buffer.byteLength,
    stream: overrides.stream ?? 'stdout',
    ...(overrides.events === undefined ? {} : { events: overrides.events }),
  }
}

describe('selectTaskLogTail', () => {
  it('slices a stream from a byte offset and advances the follow cursor', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, offset: 6, stream: 'stdout' },
      sources: [source({ buffer: bytes('hello\nworld\n') })],
    })

    expect(result).toEqual({
      chunks: [{ byteCount: 6, offset: 6, stream: 'stdout', text: 'world\n' }],
      missing: false,
      nextCursor: { offset: 12, stream: 'stdout' },
      rotated: false,
      truncated: false,
    })
  })

  it('keeps UTF-8 formatting boundary-safe when offsets split a character', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, offset: 1, stream: 'stdout' },
      sources: [source({ buffer: bytes('€uro\n') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 4, offset: 3, stream: 'stdout', text: 'uro\n' }])
    expect(result.nextCursor).toEqual({ offset: 7, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })

  it('keeps incomplete trailing UTF-8 bytes for the next cursor advance', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: new Uint8Array([0x6f, 0x6b, 0x0a, 0xe2, 0x82]), sizeBytes: 5 })],
    })

    expect(result.chunks).toEqual([{ byteCount: 3, offset: 0, stream: 'stdout', text: 'ok\n' }])
    expect(result.nextCursor).toEqual({ offset: 3, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })

  it('formats complete four-byte UTF-8 characters without shifting byte offsets', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('😀') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 4, offset: 0, stream: 'stdout', text: '😀' }])
    expect(result.nextCursor).toEqual({ offset: 4, stream: 'stdout' })
  })

  it('formats complete two-byte UTF-8 characters without shifting byte offsets', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('é') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 2, offset: 0, stream: 'stdout', text: 'é' }])
    expect(result.nextCursor).toEqual({ offset: 2, stream: 'stdout' })
  })

  it('drops a continuation-only fragment without fabricating replacement text', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: new Uint8Array([0x82]), sizeBytes: 1 })],
    })

    expect(result.chunks).toEqual([])
    expect(result.nextCursor).toEqual({ offset: 1, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })

  it('selects the last N lines after byte windowing', () => {
    const result = selectTaskLogTail({
      request: { lines: 2, maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('one\ntwo\nthree\nfour\n') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 11, offset: 8, stream: 'stdout', text: 'three\nfour\n' }])
    expect(result.nextCursor).toEqual({ offset: 19, stream: 'stdout' })
  })

  it('supports zero requested lines while still advancing the cursor', () => {
    const result = selectTaskLogTail({
      request: { lines: 0, maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('one\ntwo') })],
    })

    expect(result.chunks).toEqual([])
    expect(result.nextCursor).toEqual({ offset: 7, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })

  it('selects the last line without requiring a trailing newline', () => {
    const result = selectTaskLogTail({
      request: { lines: 1, maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('one\ntwo') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 3, offset: 4, stream: 'stdout', text: 'two' }])
    expect(result.nextCursor).toEqual({ offset: 7, stream: 'stdout' })
  })

  it('filters output from event timestamps before applying line selection', () => {
    const log = 'old\nnew\nlater\n'
    const result = selectTaskLogTail({
      request: { lines: 1, maxBytes: 100, since: '2026-07-03T10:05:00.000Z', stream: 'stdout' },
      sources: [
        source({
          buffer: bytes(log),
          events: [
            { byteCount: 4, occurredAt: '2026-07-03T10:00:00.000Z', offset: 0, stream: 'stdout' },
            { byteCount: 4, occurredAt: '2026-07-03T10:05:00.000Z', offset: 4, stream: 'stdout' },
            { byteCount: 6, occurredAt: '2026-07-03T10:06:00.000Z', offset: 8, stream: 'stdout' },
          ],
          sizeBytes: bytes(log).byteLength,
        }),
      ],
    })

    expect(result.chunks).toEqual([{ byteCount: 6, offset: 8, stream: 'stdout', text: 'later\n' }])
    expect(result.nextCursor).toEqual({ offset: 14, stream: 'stdout' })
  })

  it('bounds formatted bytes to the requested maximum and marks omitted data truncated', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 4, stream: 'stdout' },
      sources: [source({ buffer: bytes('abcdefghij') })],
    })

    expect(result.chunks).toEqual([{ byteCount: 4, offset: 6, stream: 'stdout', text: 'ghij' }])
    expect(result.nextCursor).toEqual({ offset: 10, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })

  it('reports empty logs without fabricating chunks', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, stream: 'stderr' },
      sources: [source({ buffer: bytes(''), sizeBytes: 0, stream: 'stderr' })],
    })

    expect(result).toEqual({
      chunks: [],
      missing: false,
      nextCursor: { offset: 0, stream: 'stderr' },
      rotated: false,
      truncated: false,
    })
  })

  it('reports missing stream data and preserves the requested follow offset', () => {
    const result = selectTaskLogTail({
      request: { cursor: { offset: 42, stream: 'stderr' }, maxBytes: 100, stream: 'stderr' },
      sources: [source({ stream: 'stdout' })],
    })

    expect(result).toEqual({
      chunks: [],
      missing: true,
      nextCursor: { offset: 42, stream: 'stderr' },
      rotated: false,
      truncated: false,
    })
  })

  it('resets a stale cursor when the file was truncated or rotated', () => {
    const result = selectTaskLogTail({
      request: { cursor: { offset: 99, stream: 'stdout' }, maxBytes: 100, stream: 'stdout' },
      sources: [source({ buffer: bytes('fresh\n') })],
    })

    expect(result).toEqual({
      chunks: [{ byteCount: 6, offset: 0, stream: 'stdout', text: 'fresh\n' }],
      missing: false,
      nextCursor: { offset: 6, stream: 'stdout' },
      rotated: true,
      truncated: false,
    })
  })

  it('uses the available bounded buffer window when the requested offset is no longer buffered', () => {
    const result = selectTaskLogTail({
      request: { maxBytes: 100, offset: 0, stream: 'stdout' },
      sources: [source({ buffer: bytes('klmnop'), bufferStartOffset: 10, sizeBytes: 16 })],
    })

    expect(result.chunks).toEqual([{ byteCount: 6, offset: 10, stream: 'stdout', text: 'klmnop' }])
    expect(result.nextCursor).toEqual({ offset: 16, stream: 'stdout' })
    expect(result.truncated).toBe(true)
  })
})
