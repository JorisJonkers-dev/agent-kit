export function defaultProcessKiller(pid, signal) {
    process.kill(pid, signal);
}
export function killDetachedProcessGroup(pid, signal, killPid) {
    killPid(-pid, signal);
}
export function tryKillProcessGroup(pid, signal, killGroup) {
    try {
        killGroup(pid, signal);
        return true;
    }
    catch (error) {
        if (!isNoSuchProcessError(error)) {
            throw error;
        }
        return false;
    }
}
export function terminateActiveProcess(active, signal, supervisor, killGraceMs) {
    if (active.exited || active.child.pid === undefined) {
        return;
    }
    active.terminating = true;
    try {
        if (!tryKillProcessGroup(active.child.pid, signal, supervisor.killGroup.bind(supervisor))) {
            active.exited = true;
            return;
        }
        supervisor.emit({ pid: active.child.pid, signal, type: 'terminated' });
        if (hasExited(active)) {
            return;
        }
        const waited = waitForExitOrKill(active, supervisor, killGraceMs);
        if (isPromiseLike(waited)) {
            return waited.catch((error) => {
                active.terminating = false;
                throw error;
            });
        }
    }
    catch (error) {
        active.terminating = false;
        throw error;
    }
}
function waitForExitOrKill(active, supervisor, killGraceMs) {
    return Promise.race([active.exitedPromise, supervisor.sleep(killGraceMs)]).then(() => {
        if (!active.exited && active.child.pid !== undefined) {
            if (!tryKillProcessGroup(active.child.pid, 'SIGKILL', supervisor.killGroup.bind(supervisor))) {
                active.exited = true;
                return;
            }
            supervisor.emit({ pid: active.child.pid, signal: 'SIGKILL', type: 'terminated' });
            return hasExited(active) ? undefined : active.exitedPromise;
        }
    });
}
function hasExited(active) {
    return active.exited;
}
function isNoSuchProcessError(error) {
    return (typeof error === 'object' &&
        error !== null &&
        'code' in error &&
        error.code === 'ESRCH');
}
function isPromiseLike(value) {
    return (typeof value === 'object' &&
        value !== null &&
        'then' in value &&
        typeof value.then === 'function');
}
