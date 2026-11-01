import { join } from 'node:path'

import type { TriageIndexRecord } from '../domain/gate.js'
import type { CritiqueFilesPort } from './ports.js'

/**
 * Every triage of a staged change in the repository, one JSON line each in
 * the critique directory of the git common directory, oldest first. The
 * commit gate reads the ticket's labels from the last line staged on the
 * current HEAD, so `risk:high` gates a commit without calling the tracker.
 */
export function triageIndexPath(critiqueDir: string): string {
  return join(critiqueDir, 'triages.jsonl')
}

export async function appendTriageIndex(files: CritiqueFilesPort, commonDir: string, record: TriageIndexRecord): Promise<void> {
  await files.appendText(triageIndexPath(join(commonDir, 'critique')), `${JSON.stringify(record)}\n`)
}
