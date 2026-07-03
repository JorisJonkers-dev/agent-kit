export interface ClockPort {
  now(): Date
  monotonicMs(): number
  sleep(ms: number): Promise<void>
}
