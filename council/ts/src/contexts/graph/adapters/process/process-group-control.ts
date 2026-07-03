import type {
  ActiveProcess,
  MaybePromise,
  WorkerSupervisorRuntime,
} from './types.js'

export type ProcessKiller = (pid: number, signal: NodeJS.Signals) => void
export type ProcessGroupKiller = (pid: number, signal: NodeJS.Signals) => void

export function defaultProcessKiller(pid: number, signal: NodeJS.Signals): void {
  process.kill(pid, signal)
}

export function killDetachedProcessGroup(
  pid: number,
  signal: NodeJS.Signals,
  killPid: ProcessKiller,
): void {
  killPid(-pid, signal)
}

export function tryKillProcessGroup(
  pid: number,
  signal: NodeJS.Signals,
  killGroup: ProcessGroupKiller,
): boolean {
  try {
    killGroup(pid, signal)
    return true
  } catch (error) {
    if (!isNoSuchProcessError(error)) {
      throw error
    }
    return false
  }
}

export function terminateActiveProcess(
  active: ActiveProcess,
  signal: NodeJS.Signals,
  supervisor: WorkerSupervisorRuntime,
  killGraceMs: number,
): MaybePromise<void> {
  if (active.exited || active.child.pid === undefined) {
    return
  }

  active.terminating = true
  try {
    if (!tryKillProcessGroup(active.child.pid, signal, supervisor.killGroup.bind(supervisor))) {
      active.exited = true
      return
    }

    supervisor.emit({ pid: active.child.pid, signal, type: 'terminated' })
    if (hasExited(active)) {
      return
    }

    const waited = waitForExitOrKill(active, supervisor, killGraceMs)
    if (isPromiseLike(waited)) {
      return waited.catch((error: unknown) => {
        active.terminating = false
        throw error
      })
    }
  } catch (error) {
    active.terminating = false
    throw error
  }
}

function waitForExitOrKill(
  active: ActiveProcess,
  supervisor: WorkerSupervisorRuntime,
  killGraceMs: number,
): MaybePromise<void> {
  return Promise.race([active.exitedPromise, supervisor.sleep(killGraceMs)]).then(() => {
    if (!active.exited && active.child.pid !== undefined) {
      if (!tryKillProcessGroup(active.child.pid, 'SIGKILL', supervisor.killGroup.bind(supervisor))) {
        active.exited = true
        return
      }

      supervisor.emit({ pid: active.child.pid, signal: 'SIGKILL', type: 'terminated' })
      return hasExited(active) ? undefined : active.exitedPromise
    }
  })
}

function hasExited(active: ActiveProcess): boolean {
  return active.exited
}

function isNoSuchProcessError(error: unknown): boolean {
  return (
    typeof error === 'object' &&
    error !== null &&
    'code' in error &&
    (error as { readonly code?: unknown }).code === 'ESRCH'
  )
}

function isPromiseLike<T>(value: MaybePromise<T>): value is Promise<T> {
  return (
    typeof value === 'object' &&
    value !== null &&
    'then' in value &&
    typeof (value as { readonly then?: unknown }).then === 'function'
  )
}
