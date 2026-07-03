export type EngineCli = 'claude' | 'codex'

export type ModelTier = string

export interface EngineDef {
  readonly cli: EngineCli
  readonly model: string
  readonly label?: string
}

export interface EngineTagged {
  readonly engine?: EngineDef
  readonly model_tier?: string
}
