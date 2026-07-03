export interface TailCursor {
  readonly offset: number
  readonly stream: string
}

export interface TailLogEventRange {
  readonly byteCount: number
  readonly occurredAt: string
  readonly offset: number
  readonly stream: string
}

export interface TailLogSource {
  readonly buffer: Uint8Array
  readonly bufferStartOffset: number
  readonly events?: readonly TailLogEventRange[]
  readonly sizeBytes: number
  readonly stream: string
}

export interface TailRequest {
  readonly cursor?: TailCursor
  readonly lines?: number
  readonly maxBytes: number
  readonly offset?: number
  readonly since?: string
  readonly stream: string
}

export interface TailSelectionInput {
  readonly request: TailRequest
  readonly sources: readonly TailLogSource[]
}

export interface TailFormattedChunk {
  readonly byteCount: number
  readonly offset: number
  readonly stream: string
  readonly text: string
}

export interface TailSelection {
  readonly chunks: readonly TailFormattedChunk[]
  readonly missing: boolean
  readonly nextCursor: TailCursor
  readonly rotated: boolean
  readonly truncated: boolean
}

interface ByteRange {
  readonly end: number
  readonly start: number
}

interface RangePlan {
  readonly ranges: readonly ByteRange[]
  readonly truncated: boolean
}

interface DecodedRange {
  readonly consumedEnd: number
  readonly chunk?: TailFormattedChunk
  readonly truncated: boolean
}

const textDecoder = new TextDecoder()
const textEncoder = new TextEncoder()

export function selectTaskLogTail(input: TailSelectionInput): TailSelection {
  const requestedOffset = requestedTailOffset(input.request)
  const source = input.sources.find((candidate) => candidate.stream === input.request.stream)
  if (source === undefined) {
    return tailSelection([], input.request.stream, requestedOffset, false, false, true)
  }

  const rotated = requestedOffset > source.sizeBytes
  const effectiveOffset = rotated ? 0 : requestedOffset
  const eventRanges = candidateRanges(source, input.request, effectiveOffset)
  const bounded = takeLastBytes(eventRanges, input.request.maxBytes)
  const available = availableRange(source)
  const windowed = clampToAvailableBuffer(bounded.ranges, available)
  const decoded = windowed.ranges.map((range) => decodeRange(source, range))
  const decodedChunks = decoded.flatMap((range) => (range.chunk === undefined ? [] : [range.chunk]))
  const nextOffset = nextCursorOffset(source, decoded)
  const lines = applyLineSelection(decodedChunks, input.request.lines, input.request.stream, nextOffset)

  return tailSelection(
    lines.chunks,
    input.request.stream,
    nextOffset,
    rotated,
    bounded.truncated || windowed.truncated || decoded.some((range) => range.truncated) || lines.truncated,
    false,
  )
}

function requestedTailOffset(request: TailRequest): number {
  return request.offset ?? (request.cursor?.stream === request.stream ? request.cursor.offset : undefined) ?? 0
}

function candidateRanges(source: TailLogSource, request: TailRequest, offset: number): readonly ByteRange[] {
  const since = request.since
  if (since === undefined) {
    return offset >= source.sizeBytes ? [] : [{ start: offset, end: source.sizeBytes }]
  }

  return (source.events ?? [])
    .filter((event) => event.stream === request.stream && event.occurredAt >= since)
    .map((event) => ({
      start: Math.max(event.offset, offset),
      end: Math.min(event.offset + event.byteCount, source.sizeBytes),
    }))
    .filter((range) => range.end > range.start)
}

function takeLastBytes(ranges: readonly ByteRange[], maxBytes: number): RangePlan {
  const budget = Math.max(0, maxBytes)
  const totalBytes = ranges.reduce((total, range) => total + range.end - range.start, 0)
  if (totalBytes <= budget) return { ranges, truncated: false }

  let remaining = budget
  const kept: ByteRange[] = []
  for (const range of [...ranges].reverse()) {
    const take = Math.max(0, Math.min(range.end - range.start, remaining))
    if (take > 0) kept.unshift({ start: range.end - take, end: range.end })
    remaining -= take
  }
  return { ranges: kept, truncated: true }
}

function availableRange(source: TailLogSource): ByteRange {
  return {
    start: source.bufferStartOffset,
    end: Math.min(source.sizeBytes, source.bufferStartOffset + source.buffer.byteLength),
  }
}

function clampToAvailableBuffer(ranges: readonly ByteRange[], available: ByteRange): RangePlan {
  const kept: ByteRange[] = []
  let truncated = false
  for (const range of ranges) {
    const start = Math.max(range.start, available.start)
    const end = Math.min(range.end, available.end)
    if (start !== range.start || end !== range.end) truncated = true
    if (end > start) kept.push({ start, end })
  }
  return { ranges: kept, truncated }
}

function decodeRange(source: TailLogSource, range: ByteRange): DecodedRange {
  const relativeStart = range.start - source.bufferStartOffset
  const relativeEnd = range.end - source.bufferStartOffset
  const safeStart = utf8SafeStart(source.buffer, relativeStart, relativeEnd)
  const safeEnd = utf8SafeEnd(source.buffer, safeStart, relativeEnd)
  const absoluteStart = source.bufferStartOffset + safeStart
  const absoluteEnd = source.bufferStartOffset + safeEnd
  const truncated = safeStart !== relativeStart || safeEnd !== relativeEnd
  if (absoluteEnd <= absoluteStart) return { consumedEnd: absoluteEnd, truncated }

  return {
    chunk: {
      byteCount: absoluteEnd - absoluteStart,
      offset: absoluteStart,
      stream: source.stream,
      text: textDecoder.decode(source.buffer.subarray(safeStart, safeEnd)),
    },
    consumedEnd: absoluteEnd,
    truncated,
  }
}

function utf8SafeStart(bytes: Uint8Array, start: number, end: number): number {
  let safeStart = start
  while (safeStart < end && isUtf8Continuation(bytes[safeStart] ?? 0)) safeStart += 1
  return safeStart
}

function utf8SafeEnd(bytes: Uint8Array, start: number, end: number): number {
  let sequenceStart = end - 1
  while (sequenceStart >= start && isUtf8Continuation(bytes[sequenceStart] ?? 0)) sequenceStart -= 1
  if (sequenceStart < start) return start

  const expectedLength = utf8SequenceLength(bytes[sequenceStart] ?? 0)
  return sequenceStart + expectedLength <= end ? end : sequenceStart
}

function isUtf8Continuation(byte: number): boolean {
  return (byte & 0xc0) === 0x80
}

function utf8SequenceLength(byte: number): number {
  if ((byte & 0x80) === 0) return 1
  if ((byte & 0xe0) === 0xc0) return 2
  if ((byte & 0xf0) === 0xe0) return 3
  return (byte & 0xf8) === 0xf0 ? 4 : 1
}

function nextCursorOffset(source: TailLogSource, decoded: readonly DecodedRange[]): number {
  return decoded.length === 0 ? source.sizeBytes : Math.max(...decoded.map((range) => range.consumedEnd))
}

function applyLineSelection(
  chunks: readonly TailFormattedChunk[],
  lines: number | undefined,
  stream: string,
  nextOffset: number,
): { readonly chunks: readonly TailFormattedChunk[]; readonly truncated: boolean } {
  if (lines === undefined) return { chunks, truncated: false }

  const text = chunks.map((chunk) => chunk.text).join('')
  const selected = lastLines(text, lines)
  if (selected.length === 0) return { chunks: [], truncated: text.length > 0 }

  const byteCount = textEncoder.encode(selected).byteLength
  return {
    chunks: [{ byteCount, offset: nextOffset - byteCount, stream, text: selected }],
    truncated: selected !== text,
  }
}

function lastLines(text: string, lineCount: number): string {
  if (lineCount <= 0 || text.length === 0) return ''

  const trailingNewline = text.endsWith('\n')
  const records = (trailingNewline ? text.slice(0, -1) : text).split('\n')
  const selected = records.slice(-lineCount).join('\n')
  return trailingNewline ? `${selected}\n` : selected
}

function tailSelection(
  chunks: readonly TailFormattedChunk[],
  stream: string,
  offset: number,
  rotated: boolean,
  truncated: boolean,
  missing: boolean,
): TailSelection {
  return {
    chunks,
    missing,
    nextCursor: { offset, stream },
    rotated,
    truncated,
  }
}
