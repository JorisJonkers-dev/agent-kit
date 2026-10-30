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
