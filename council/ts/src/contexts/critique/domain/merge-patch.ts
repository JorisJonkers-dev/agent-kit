import { isJsonObject } from './json.js'

/**
 * Applies `patch` to `target` the way a JSON merge patch (RFC 7386) does:
 * an object merges into an object key by key, `null` removes the key, and
 * any other value (a list included) replaces what was there.
 */
export function mergePatch(target: unknown, patch: unknown): unknown {
  if (!isJsonObject(patch)) return patch
  const base = isJsonObject(target) ? target : {}
  const keys = new Set([...Object.keys(base), ...Object.keys(patch)])
  return Object.fromEntries(
    [...keys].flatMap((key) => {
      if (!(key in patch)) return [[key, base[key]]]
      const value = patch[key]
      return value === null ? [] : [[key, mergePatch(base[key], value)]]
    }),
  )
}
