import { performance } from 'node:perf_hooks';
export class SystemClockAdapter {
    now() {
        return new Date();
    }
    monotonicMs() {
        return performance.now();
    }
    async sleep(ms) {
        await new Promise((resolve) => {
            setTimeout(resolve, ms);
        });
    }
}
export class ProcessEnvAdapter {
    source;
    constructor(source = process.env) {
        this.source = source;
    }
    get(name) {
        return this.source[name];
    }
    require(name) {
        const value = this.get(name);
        if (value === undefined) {
            throw new Error(`required environment variable is missing: ${name}`);
        }
        return value;
    }
    all() {
        const entries = Object.entries(this.source).filter((entry) => entry[1] !== undefined);
        return Object.fromEntries(entries);
    }
}
