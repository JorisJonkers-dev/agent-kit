/**
 * How many attempts may run on one client at once. A batch shares one of these
 * between its delegations, and an attempt takes a slot of the client it is
 * about to run on, which after a fall-through is not the client its delegation
 * was planned for. An attempt that finds the client full waits, in the order
 * it asked, and gives its slot back when it ends.
 */
export class ClientSlots {
  private readonly running = new Map<string, number>()
  private readonly waiting = new Map<string, (() => void)[]>()

  constructor(private readonly limit: number) {}

  /** Resolves with the function that gives the slot back, once the client has one free. */
  acquire(client: string): Promise<() => void> {
    return new Promise((resolve) => {
      const grant = (): void => {
        this.running.set(client, (this.running.get(client) ?? 0) + 1)
        resolve(() => {
          this.release(client)
        })
      }
      if ((this.running.get(client) ?? 0) < this.limit) grant()
      else this.waiting.set(client, [...(this.waiting.get(client) ?? []), grant])
    })
  }

  private release(client: string): void {
    this.running.set(client, (this.running.get(client) ?? 1) - 1)
    const [next, ...rest] = this.waiting.get(client) ?? []
    this.waiting.set(client, rest)
    next?.()
  }
}
