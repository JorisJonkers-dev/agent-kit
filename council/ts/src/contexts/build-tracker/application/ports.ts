/** The files the build tracker reads and writes. */
export interface BuildTrackerFilesPort {
  /** Rejects with the platform's error (its `code` is `ENOENT` for a missing file). */
  readText(path: string): Promise<string>
  /** Replaces the file whole: a temporary file beside it, then a rename. */
  writeAtomic(path: string, content: string): Promise<void>
}

export interface BuildTrackerClockPort {
  now(): Date
}
