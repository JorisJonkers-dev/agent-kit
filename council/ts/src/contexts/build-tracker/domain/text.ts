/** The first line of `text` that is not blank, trimmed; undefined when every line is. */
export function firstLine(text: string): string | undefined {
  return text
    .split('\n')
    .map((line) => line.trim())
    .find((line) => line.length > 0)
}
