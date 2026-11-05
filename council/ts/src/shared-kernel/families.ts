/**
 * Every model family a client on this estate serves. One list, shared by the
 * model gateway and the critique runner, so both accept the same families: a
 * catalogue entry names one of these, and a family allowlist entry may name
 * one of these. A new family — a new vendor's models joined to the catalogue —
 * is added here once and both keep reading the file.
 *
 * The client the family arrives through, not the family alone, decides what a
 * `nedap` repository may reach; the family list never widens that rule.
 */
export const KNOWN_FAMILIES = [
  'mistral',
  'zhipu',
  'deepseek',
  'google',
  'anthropic',
  'openai',
  'alibaba',
  'moonshot',
  'xiaomi',
  'minimax',
  'xai',
  'tencent',
  'meituan',
  'stepfun',
] as const
export type KnownFamily = (typeof KNOWN_FAMILIES)[number]

/** Whether `family` is one the list names. */
export function isKnownFamily(family: string): boolean {
  return KNOWN_FAMILIES.includes(family as KnownFamily)
}
