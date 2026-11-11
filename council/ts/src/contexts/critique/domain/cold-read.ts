import { isJsonObject } from './json.js'

/**
 * The three questions the cold reader answers about one decision record: what
 * was decided, what was rejected and why, and when to revisit. It answers
 * from the record alone, so an empty answer is one it could not give.
 */
export const COLD_READ_ANSWERS = ['decided', 'rejected', 'revisit'] as const
export type ColdReadAnswerName = (typeof COLD_READ_ANSWERS)[number]

/** What the cold reader made of one record: one answer per question. */
export type ColdReadAnswers = Readonly<Record<ColdReadAnswerName, string>>

/**
 * `answered`: the reader gave a readable answer, though it may have left one
 * empty. `unavailable`: it could not run, or gave no answer the runner can
 * read. Neither is a pass: the orchestrator compares the answers with the
 * record itself.
 */
export type ColdReadStatus = 'answered' | 'unavailable'

/** One rewritten record's cold read. */
export interface ColdReadFile {
  /** The record's repository path, as the caller named it. */
  readonly path: string
  readonly status: ColdReadStatus
  /** What the reader answered; absent when it could not run. */
  readonly answers?: ColdReadAnswers
  /** The answers it left empty, in the order the role asks them: an omission, never a pass. */
  readonly missing: readonly ColdReadAnswerName[]
  /** Why it could not run, or gave no readable answer. */
  readonly reason?: string
}

/** `cold-read.json`, which the command writes beside the run and returns. */
export interface ColdReadResult {
  readonly status: 'checked'
  readonly run_id: string
  /** Why the gateway's ledger would not take a launch's record, collected as the run goes. */
  readonly ledger_warnings?: readonly string[]
  readonly files: readonly ColdReadFile[]
}

const FENCED_JSON = /```json[ \t]*\r?\n([\s\S]*?)\r?\n?```/g

/**
 * Reads the last ```json block of the cold reader's reply. Undefined when the
 * reply holds no such block, the block is not JSON, or a value is not a
 * string: the reader gave no answer the runner can read. A missing key and an
 * empty string are both an empty answer, which is an omission.
 */
export function parseColdReadReply(text: string): ColdReadAnswers | undefined {
  const block = [...text.matchAll(FENCED_JSON)].at(-1)
  if (block === undefined) return undefined
  let value: unknown
  try {
    value = JSON.parse(block[1] ?? '')
  } catch {
    return undefined
  }
  if (!isJsonObject(value)) return undefined
  const answers: Record<ColdReadAnswerName, string> = { decided: '', rejected: '', revisit: '' }
  for (const name of COLD_READ_ANSWERS) {
    const answer = value[name]
    if (answer !== undefined && typeof answer !== 'string') return undefined
    answers[name] = (answer ?? '').trim()
  }
  return answers
}

/** The answers left empty, in the order the role asks them. */
export function missingAnswers(answers: ColdReadAnswers): readonly ColdReadAnswerName[] {
  return COLD_READ_ANSWERS.filter((name) => answers[name] === '')
}

/** One record the reader answered. */
export function answeredFile(path: string, answers: ColdReadAnswers): ColdReadFile {
  return { answers, missing: missingAnswers(answers), path, status: 'answered' }
}

/** One record the reader could not read: every answer is missing, and `reason` says why. */
export function unavailableFile(path: string, reason: string): ColdReadFile {
  return { missing: [...COLD_READ_ANSWERS], path, reason, status: 'unavailable' }
}
