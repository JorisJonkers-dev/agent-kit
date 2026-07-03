export interface EnvPort {
  get(name: string): string | undefined
  require(name: string): string
  all(): Readonly<Record<string, string>>
}
