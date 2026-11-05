import { mkdir, readFile, writeFile } from 'node:fs/promises'
import { dirname, join } from 'node:path'

import type { ModelGatewayProcessPort } from '../../src/contexts/model-gateway/index.js'
import type { ProcessCommand, ProcessResult } from '../../src/ports/index.js'

/** What one fake launch does: the answer it gives and what it costs, how long it takes, and how it ends. */
export interface DelegateBehavior {
  /** What the client answers; a SUMMARY line is added when it has none. */
  readonly text?: string
  /** The dollars the client says the run cost. */
  readonly cost?: number
  /** How long the launch takes, so that launches of one test overlap. */
  readonly delayMs?: number
  /** A non-zero exit fails the run. */
  readonly exitCode?: number
  readonly stderr?: string
  /** Runs as the launch starts, as the time passing in the world would. */
  readonly effect?: () => void
}

/** One launch a fake saw: which task it was, on which client, with which prompt. */
export interface DelegateLaunch {
  readonly task: string
  readonly client: string
  readonly prompt: string
  /** The nth launch of this task, from 1. */
  readonly attempt: number
  /** The longest the launch may take, as the gateway told the process port. */
  readonly timeoutMs: number | undefined
}

/**
 * A client that answers each launch by the task its prompt names (`TASK-<name>`),
 * whatever order launches arrive in, and counts how many are running at once
 * per client. It plays Vibe, Claude Code, Codex and Hermes by the shape of
 * their command lines, answers every other command (a status check) as signed
 * in, and starts nothing and spends nothing.
 */
export class FakeDelegateProcess implements ModelGatewayProcessPort {
  readonly launches: DelegateLaunch[] = []
  /** The most launches of one client that ran at the same time. */
  readonly peak = new Map<string, number>()
  /** The most launches of any client that ran at the same time. */
  peakTotal = 0
  private readonly active = new Map<string, number>()
  private readonly attempts = new Map<string, number>()

  /**
   * `barrier` holds the first that many launches until that many are running
   * at once (or five seconds pass), so a test of how many run in parallel does
   * not depend on how fast the machine starts them.
   */
  constructor(
    private readonly behaviors: Record<string, DelegateBehavior | readonly DelegateBehavior[]> = {},
    private readonly barrier = 0,
  ) {}

  which(): Promise<boolean> {
    return Promise.resolve(true)
  }

  async exec(command: ProcessCommand): Promise<ProcessResult> {
    const launched = await this.identify(command)
    // A writing delegation's check: `false` fails it, anything else passes.
    if (command.command === 'sh') return { exitCode: command.args[1] === 'false' ? 1 : 0, stderr: '', stdout: '' }
    if (launched === undefined) return { exitCode: 0, stderr: '', stdout: 'Logged in\n' }
    const attempt = (this.attempts.get(launched.task) ?? 0) + 1
    this.attempts.set(launched.task, attempt)
    this.launches.push({ attempt, client: launched.client, prompt: launched.prompt, task: launched.task, timeoutMs: command.timeoutMs })
    const behavior = this.behaviorOf(launched.task, attempt)
    behavior.effect?.()
    this.enter(launched.client)
    try {
      if (this.launches.length <= this.barrier) await this.holdUntilRunning(this.barrier)
      await new Promise((resolve) => setTimeout(resolve, behavior.delayMs ?? 5))
    } finally {
      this.leave(launched.client)
    }
    return this.answer(command, launched.client, launched.task, behavior)
  }

  /** Gives a task a behavior after the fact, for a test that needs the workspace it is built in. */
  add(task: string, behavior: DelegateBehavior): void {
    this.behaviors[task] = behavior
  }

  /** The tasks launched, in the order the launches arrived. */
  tasks(): string[] {
    return this.launches.map((launch) => launch.task)
  }

  private behaviorOf(task: string, attempt: number): DelegateBehavior {
    const configured = this.behaviors[task]
    if (configured === undefined) return {}
    if (!Array.isArray(configured)) return configured as DelegateBehavior
    const list = configured as readonly DelegateBehavior[]
    return list[Math.min(attempt, list.length) - 1] ?? {}
  }

  private async holdUntilRunning(count: number): Promise<void> {
    const until = Date.now() + 5000
    while (this.running() < count && Date.now() < until) await new Promise((resolve) => setTimeout(resolve, 2))
  }

  private running(): number {
    return [...this.active.values()].reduce((total, running) => total + running, 0)
  }

  private enter(client: string): void {
    this.active.set(client, (this.active.get(client) ?? 0) + 1)
    this.peak.set(client, Math.max(this.peak.get(client) ?? 0, this.active.get(client) ?? 0))
    this.peakTotal = Math.max(this.peakTotal, this.running())
  }

  private leave(client: string): void {
    this.active.set(client, (this.active.get(client) ?? 1) - 1)
  }

  /** The client and the prompt of a launch, or undefined for a command that is not one. */
  private async identify(command: ProcessCommand): Promise<{ client: string; prompt: string; task: string } | undefined> {
    const prompt = await this.promptOf(command)
    const task = prompt === undefined ? undefined : /TASK-([A-Za-z0-9]+)/.exec(prompt)?.[1]
    if (prompt === undefined || task === undefined) return undefined
    const client = command.command === 'env' ? 'vibe' : command.command
    return { client, prompt, task }
  }

  private async promptOf(command: ProcessCommand): Promise<string | undefined> {
    if (command.command === 'claude' || command.command === 'codex') return command.stdin
    if (command.command === 'hermes') return command.args[command.args.indexOf('-z') + 1]
    const at = command.args.indexOf('--prompt-file')
    return at === -1 ? undefined : readFile(String(command.args[at + 1]), 'utf8')
  }

  private async answer(command: ProcessCommand, client: string, task: string, behavior: DelegateBehavior): Promise<ProcessResult> {
    const exitCode = behavior.exitCode ?? 0
    const stderr = behavior.stderr ?? ''
    if (exitCode !== 0) return { exitCode, stderr, stdout: '' }
    const text = behavior.text ?? `did ${task}`
    const answer = text.includes('SUMMARY:') ? text : `${text}\nSUMMARY: ${text}`
    const cost = behavior.cost ?? 0.01
    if (client === 'claude') {
      return { exitCode, stderr, stdout: JSON.stringify({ is_error: false, result: answer, subtype: 'success', total_cost_usd: cost, type: 'result', usage: { input_tokens: 10, output_tokens: 40 } }) }
    }
    if (client === 'hermes') {
      const usagePath = String(command.args[command.args.indexOf('--usage-file') + 1])
      await mkdir(dirname(usagePath), { recursive: true })
      await writeFile(usagePath, JSON.stringify({ api_calls: 1, estimated_cost_usd: cost, failed: false, input_tokens: 100, output_tokens: 50 }))
      return { exitCode, stderr, stdout: answer }
    }
    if (client === 'codex') {
      const events = [
        { thread_id: 't-1', type: 'thread.started' },
        { item: { id: 'item_0', text: answer, type: 'agent_message' }, type: 'item.completed' },
        { type: 'turn.completed', usage: { cached_input_tokens: 0, input_tokens: 1000, output_tokens: 200 } },
      ]
      return { exitCode, stderr, stdout: events.map((event) => JSON.stringify(event)).join('\n') }
    }
    const outputDir = String(command.args[command.args.indexOf('--output-dir') + 1])
    await mkdir(outputDir, { recursive: true })
    await writeFile(join(outputDir, 'export.json'), JSON.stringify({ cost_usd: cost, usage: { cached_input_tokens: 0, input_tokens: 100, output_tokens: 50 } }))
    return { exitCode, stderr, stdout: JSON.stringify([{ content: [{ text: answer }], role: 'assistant', type: 'message' }]) }
  }
}
