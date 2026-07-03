import { runClaudeEngine } from './claude-driver.js';
import { runCodexEngine } from './codex-driver.js';
import { runGenericCommandEngine } from './generic-driver.js';
class DelegatingEngineDriver {
    runEngine;
    constructor(runEngine) {
        this.runEngine = runEngine;
    }
    run(request, ports) {
        return this.runEngine(request, ports);
    }
}
const ENGINE_DRIVERS = Object.freeze({
    claude: new DelegatingEngineDriver(runClaudeEngine),
    codex: new DelegatingEngineDriver(runCodexEngine),
});
const GENERIC_ENGINE_DRIVER = new DelegatingEngineDriver(runGenericCommandEngine);
export function runEngine(request, ports) {
    return selectEngineDriver(request.engine.name).run(request, ports);
}
function selectEngineDriver(engineName) {
    return ENGINE_DRIVERS[engineName] ?? GENERIC_ENGINE_DRIVER;
}
