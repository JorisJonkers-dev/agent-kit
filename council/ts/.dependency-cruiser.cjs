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
      to: { path: '^src/(contexts|adapters|app|cli|ports)/|^node:' },
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
  ],
  options: {
    doNotFollow: { path: 'node_modules' },
    tsPreCompilationDeps: true,
    tsConfig: { fileName: 'tsconfig.json' },
  },
}
