module.exports = {
  forbidden: [
    {
      name: 'no-circular',
      severity: 'error',
      from: {},
      to: { circular: true },
    },
    {
      // The shared kernel is pure value/wire types: it may not depend on any
      // context, adapter, shell layer, ports, or Node builtins.
      name: 'shared-kernel-is-pure',
      severity: 'error',
      from: { path: '^src/shared-kernel/' },
      to: { path: '^src/(contexts|adapters|app|cli|composition|ports|workflows)/|^node:' },
    },
    {
      // A context's domain is the pure core: no adapters (its own or others'),
      // no Node IO builtins.
      name: 'context-domain-no-adapters-or-node',
      severity: 'error',
      from: { path: '^src/contexts/[^/]+/domain/' },
      to: { path: '^src/contexts/[^/]+/adapters/|^src/adapters/|^node:(child_process|fs)(/|$)' },
    },
    {
      // Cross-context references go through the sibling context's index barrel,
      // never into its internals. (Same-context deep imports are allowed.)
      name: 'contexts-cross-import-via-barrel',
      severity: 'error',
      from: { path: '^src/contexts/([^/]+)/' },
      to: {
        path: '^src/contexts/([^/]+)/(domain|adapters)/.+',
        pathNot: '^src/contexts/$1/',
      },
    },
    {
      // Critics never debate. The critique context and its runner must not
      // reach council's cross-critique, design-vote or consolidation code
      // (design, review, reviewpack's checkpoint packs, context's role
      // matrix, prompts, triage's lens catalog), nor the workflow,
      // composition and CLI shells that wire it.
      name: 'critique-never-imports-council-debate',
      severity: 'error',
      from: { path: '^src/(contexts/critique|critique-cli)/' },
      to: {
        path: '^src/(contexts/(design|review|reviewpack|context|prompts|triage)|workflows|composition|app|cli)/',
      },
    },
    {
      // The critique domain is pure: no ports, no application layer and no
      // adapters. How a critic client is launched lives in its adapter.
      name: 'critique-domain-is-pure',
      severity: 'error',
      from: { path: '^src/contexts/critique/domain/' },
      to: { path: '^src/(ports|adapters)/|^src/contexts/critique/(application|adapters)/' },
    },
    {
      // The critique runner's CLI reaches contexts only through their barrels.
      name: 'critique-cli-uses-context-barrels',
      severity: 'error',
      from: { path: '^src/critique-cli/' },
      to: { path: '^src/contexts/[^/]+/(domain|adapters|application)/.+' },
    },
    {
      // The build tracker is its own small runner: its context and CLI reach
      // no other context and none of council's shells, so its bundle stays
      // self-contained.
      name: 'build-tracker-stands-alone',
      severity: 'error',
      from: { path: '^src/(contexts/build-tracker|build-tracker-cli)/' },
      to: {
        path: '^src/(contexts/[^/]+|workflows|composition|app|cli|critique-cli|adapters|ports)/',
        pathNot: '^src/contexts/build-tracker/',
      },
    },
    {
      // The build tracker is a view for the owner, never a source for the
      // flow state: the critique runner may not read it. /flow runs the two
      // runners side by side instead.
      name: 'critique-never-reads-the-build-tracker',
      severity: 'error',
      from: { path: '^src/(contexts/critique|critique-cli)/' },
      to: { path: '^src/(contexts/build-tracker|build-tracker-cli)/' },
    },
    {
      // The build tracker's CLI reaches its context only through the barrel.
      name: 'build-tracker-cli-uses-context-barrel',
      severity: 'error',
      from: { path: '^src/build-tracker-cli/' },
      to: { path: '^src/contexts/build-tracker/(domain|adapters|application)/.+' },
    },
    {
      // The model gateway is its own small runner: its context and CLI reach
      // no other context and none of council's shells, so its bundle stays
      // self-contained. The shared ports and the shared process and
      // environment adapters are all it may use from outside.
      name: 'model-gateway-stands-alone',
      severity: 'error',
      from: { path: '^src/(contexts/model-gateway|model-gateway-cli)/' },
      to: {
        path: '^src/(contexts/[^/]+|workflows|composition|app|cli|critique-cli|build-tracker-cli)/',
        pathNot: '^src/contexts/model-gateway/',
      },
    },
    {
      // The model gateway's domain is pure: no ports, no application layer and
      // no adapters. Reading a file and asking PATH live in an adapter.
      name: 'model-gateway-domain-is-pure',
      severity: 'error',
      from: { path: '^src/contexts/model-gateway/domain/' },
      to: { path: '^src/(ports|adapters)/|^src/contexts/model-gateway/(application|adapters)/' },
    },
    {
      // The critique context reaches the model gateway only through its public
      // barrel: a critic's launch goes through the gateway's launch path, never
      // its internals. The gateway reaches nothing in critique.
      name: 'critique-uses-the-gateway-barrel',
      severity: 'error',
      from: { path: '^src/contexts/critique/' },
      to: { path: '^src/contexts/model-gateway/(domain|application|adapters)/.+' },
    },
    {
      // Council's engines reach the model gateway only through its public
      // barrel, and reach no other context: an engine process is authorised
      // and recorded by the gateway's guard, never by its internals. The
      // gateway reaches nothing in council or critique (model-gateway-stands-alone).
      name: 'engines-reach-only-the-gateway-barrel',
      severity: 'error',
      from: { path: '^src/contexts/engines/' },
      to: {
        path: '^src/contexts/(?!engines/)[^/]+/',
        pathNot: '^src/contexts/model-gateway/index\\.ts$',
      },
    },
    {
      // The model gateway's CLI reaches its context only through the barrel.
      name: 'model-gateway-cli-uses-context-barrel',
      severity: 'error',
      from: { path: '^src/model-gateway-cli/' },
      to: { path: '^src/contexts/model-gateway/(domain|adapters|application)/.+' },
    },
    {
      // The CLI is a thin inbound adapter and reaches contexts only through
      // their public barrels, never their internals.
      name: 'cli-uses-context-barrels',
      severity: 'error',
      from: { path: '^src/cli/' },
      to: { path: '^src/contexts/[^/]+/(domain|adapters)/.+' },
    },
    {
      // Composition (app) may wire concrete adapters, but never reaches into a
      // context's pure domain internals.
      name: 'app-does-not-reach-into-domain-internals',
      severity: 'error',
      from: { path: '^src/app/' },
      to: { path: '^src/contexts/[^/]+/domain/.+' },
    },
    {
      // Workflow orchestration reaches contexts only through their public
      // barrels. Concrete adapters are injected by composition.
      name: 'workflows-use-context-barrels',
      severity: 'error',
      from: { path: '^src/workflows/' },
      to: { path: '^src/contexts/[^/]+/(domain|adapters)/.+' },
    },
    {
      // Composition stays out of context internals; adapter wiring belongs in
      // the top-level adapters layer or public context barrels.
      name: 'composition-uses-context-barrels',
      severity: 'error',
      from: { path: '^src/composition/' },
      to: { path: '^src/contexts/[^/]+/(domain|adapters)/.+' },
    },
  ],
  options: {
    doNotFollow: { path: '(^|/)(node_modules|dist|ts-dist)(/|$)' },
    tsPreCompilationDeps: true,
    tsConfig: { fileName: 'tsconfig.json' },
  },
}
