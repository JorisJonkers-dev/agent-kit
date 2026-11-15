import { isObject, ModelGatewayError, type ToolKind } from './config.js'

/** What a request narrows a profile to; every field may only take something away. */
export interface Narrowing {
  readonly models?: readonly string[]
  readonly mcp?: readonly string[]
  readonly skills?: readonly string[]
  readonly plugins?: readonly string[]
  readonly max_spend_usd?: number
  /** The family of the writer whose work a critic reads; it is left out of the allowed models. */
  readonly writer_family?: string
}

const NARROWING_FIELDS = ['models', 'mcp', 'skills', 'plugins', 'max_spend_usd', 'writer_family'] as const

/** Reads a request's narrowing, or says what is wrong with it. */
export function readNarrowing(raw: unknown): Narrowing | undefined {
  if (raw === undefined) return undefined
  if (!isObject(raw)) throw new ModelGatewayError('invalid-input', 'narrow must be an object')
  const unknownField = Object.keys(raw).find((key) => !(NARROWING_FIELDS as readonly string[]).includes(key))
  if (unknownField !== undefined) {
    throw new ModelGatewayError('invalid-input', `narrow.${unknownField} is not a field; choose from ${NARROWING_FIELDS.join(', ')}`)
  }
  const list = (name: ToolKind | 'models'): readonly string[] | undefined => {
    if (raw[name] === undefined) return undefined
    const value: unknown = raw[name]
    if (!Array.isArray(value) || value.some((item) => typeof item !== 'string' || item === '') || (name === 'models' && value.length === 0)) {
      throw new ModelGatewayError('invalid-input', `narrow.${name} must be a ${name === 'models' ? 'non-empty ' : ''}list of non-empty strings`)
    }
    return value as readonly string[]
  }
  const models = list('models')
  const mcp = list('mcp')
  const skills = list('skills')
  const plugins = list('plugins')
  const spend: unknown = raw.max_spend_usd
  if (spend !== undefined && (typeof spend !== 'number' || !Number.isFinite(spend) || spend <= 0)) {
    throw new ModelGatewayError('invalid-input', 'narrow.max_spend_usd must be a number above 0')
  }
  const writer: unknown = raw.writer_family
  if (writer !== undefined && (typeof writer !== 'string' || writer === '')) {
    throw new ModelGatewayError('invalid-input', 'narrow.writer_family must be a non-empty string')
  }
  return {
    ...(models === undefined ? {} : { models }),
    ...(mcp === undefined ? {} : { mcp }),
    ...(skills === undefined ? {} : { skills }),
    ...(plugins === undefined ? {} : { plugins }),
    ...(spend === undefined ? {} : { max_spend_usd: spend }),
    ...(writer === undefined ? {} : { writer_family: writer }),
  }
}
