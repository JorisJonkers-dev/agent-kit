import type {
  EngineAuthorization,
  EngineAuthorizeRequest,
  EngineReauthorizeRequest,
  EngineRecordInput,
  EngineReservation,
} from '../../src/contexts/model-gateway/index.js'

/**
 * The gateway's engine guard, faked for a council test: it allows every run
 * unless the test gives it a refusal, and writes nothing anywhere but its own
 * lists. A test asserts what council asked it and what council recorded.
 */
export class FakeEngineGuard {
  readonly authorized: EngineAuthorizeRequest[] = []
  readonly recorded: EngineRecordInput[] = []
  readonly reauthorized: EngineReauthorizeRequest[] = []
  /** What `authorize` rejects with, as the gateway's refusal does; none by default. */
  refusal: Error | undefined
  /** What `reauthorize` rejects with, as a grant that lapsed does; none by default. */
  reauthorizeRefusal: Error | undefined
  /** What `record` rejects with, as a ledger that cannot be written does; none by default. */
  recordError: Error | undefined
  private runs = 0

  authorize(request: EngineAuthorizeRequest): Promise<EngineAuthorization> {
    this.authorized.push(request)
    if (this.refusal !== undefined) return Promise.reject(this.refusal)
    this.runs += 1
    const reservation: EngineReservation = {
      client: request.client,
      cost_class: 'metered-employer',
      estimate_usd: 0.5,
      family: 'openai',
      hold: undefined,
      model: request.model,
      repository: null,
      run_id: `fake-run-${String(this.runs)}`,
      workflow: request.workflow,
    }
    return Promise.resolve({ reservation })
  }

  reauthorize(request: EngineReauthorizeRequest): Promise<void> {
    this.reauthorized.push(request)
    return this.reauthorizeRefusal === undefined ? Promise.resolve() : Promise.reject(this.reauthorizeRefusal)
  }

  record(input: EngineRecordInput): Promise<void> {
    this.recorded.push(input)
    return this.recordError === undefined ? Promise.resolve() : Promise.reject(this.recordError)
  }
}
