import { markedSupersededBy, type AdrSource } from './adr-log.js'

/** One rewritten file as check-rewrite reads it: its text and the decision sentences the orchestrator labelled in it. */
export interface RewrittenFile {
  readonly path: string
  readonly text: string
  /** The decision sentences the orchestrator found in the rewritten text, quoted as they read there. */
  readonly decision_sentences: readonly string[]
}

/** What check-rewrite confirms about one rewrite of a decision record. */
export interface RewriteCheck {
  /** The decision sentences quoted from the frozen packet before the rewrite began. */
  readonly original_sentences: readonly string[]
  readonly rewritten: readonly RewrittenFile[]
  /** A Mega-ADR split: the record that was split and the records created from its sentences, which the original must mark superseded by. */
  readonly split:
    | {
        readonly original: string
        readonly text: string
        readonly files: readonly string[]
      }
    | undefined
}

/**
 * Why one rewrite check failed. `sentence-preserved` names an original
 * sentence that is not byte-identical in exactly one rewritten file;
 * `no-new-decision-sentence` names a labelled sentence that is not an
 * original one; `split-superseded` names the split files the original record
 * does not mark superseded by.
 */
export type RewriteFailure =
  | {
      readonly check: 'sentence-preserved'
      readonly problem: 'missing' | 'duplicated'
      readonly sentence: string
      /** The rewritten files whose text holds the sentence, byte-identical. */
      readonly found_in: readonly string[]
    }
  | {
      readonly check: 'no-new-decision-sentence'
      readonly file: string
      readonly sentence: string
    }
  | {
      readonly check: 'split-superseded'
      readonly original: string
      /** The split files the original record does not name on a Superseded by or Replaced by line. */
      readonly unmarked: readonly string[]
    }

/** `failed` while any rewrite check failed. */
export type RewriteCheckStatus = 'passed' | 'failed'

/**
 * The rewrite checks: every decision sentence the orchestrator quoted from
 * the frozen packet must appear byte-identical in exactly one rewritten file,
 * no rewritten file may label a decision sentence that is not an original
 * one, and a split must leave the original record marked superseded by the
 * files it was split into. The failures say which check failed and how.
 */
export function rewriteFailures(check: RewriteCheck): readonly RewriteFailure[] {
  const failures: RewriteFailure[] = []
  for (const sentence of check.original_sentences) {
    const foundIn = check.rewritten.filter((file) => file.text.includes(sentence)).map((file) => file.path)
    if (foundIn.length !== 1) {
      failures.push({ check: 'sentence-preserved', found_in: foundIn, problem: foundIn.length === 0 ? 'missing' : 'duplicated', sentence })
    }
  }
  const originals = new Set(check.original_sentences)
  for (const file of check.rewritten) {
    for (const sentence of file.decision_sentences) {
      if (!originals.has(sentence)) failures.push({ check: 'no-new-decision-sentence', file: file.path, sentence })
    }
  }
  const split = check.split
  if (split !== undefined) {
    const record: AdrSource = { path: split.original, text: split.text }
    const unmarked = split.files.filter((target) => !markedSupersededBy(record, target))
    if (unmarked.length > 0) failures.push({ check: 'split-superseded', original: split.original, unmarked })
  }
  return failures
}

/** Whether every rewrite check passed. */
export function rewriteCheckStatus(failures: readonly RewriteFailure[]): RewriteCheckStatus {
  return failures.length === 0 ? 'passed' : 'failed'
}
