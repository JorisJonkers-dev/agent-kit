module.exports = {
  forbidden: [
    {
      name: 'no-circular',
      severity: 'error',
      from: {},
      to: {
        circular: true,
      },
    },
    {
      name: 'domain-no-adapters-or-cli',
      severity: 'error',
      from: {
        path: '^src/domain/',
      },
      to: {
        path: '^src/(adapters|cli)/',
      },
    },
    {
      name: 'domain-no-node-process-or-fs',
      severity: 'error',
      from: {
        path: '^src/domain/',
      },
      to: {
        path: '^node:(child_process|fs)(/|$)',
      },
    },
    {
      name: 'ports-cross-module-imports-via-barrels',
      severity: 'error',
      from: {
        path: '^src/ports/',
      },
      to: {
        path: '^src/(domain|adapters|cli)/(?![^/]+(/index)?\\.ts$)',
      },
    },
    {
      name: 'domain-cross-module-imports-via-barrels',
      severity: 'error',
      from: {
        path: '^src/domain/',
      },
      to: {
        path: '^src/(ports|adapters|cli)/(?![^/]+(/index)?\\.ts$)',
      },
    },
    {
      name: 'adapters-cross-module-imports-via-barrels',
      severity: 'error',
      from: {
        path: '^src/adapters/',
      },
      to: {
        path: '^src/(domain|ports|cli)/(?![^/]+(/index)?\\.ts$)',
      },
    },
    {
      name: 'cli-cross-module-imports-via-barrels',
      severity: 'error',
      from: {
        path: '^src/cli/',
      },
      to: {
        path: '^src/(domain|ports|adapters)/(?![^/]+(/index)?\\.ts$)',
      },
    },
    {
      name: 'adapters-do-not-import-adapters',
      severity: 'error',
      from: {
        path: '^src/adapters/([^/]+)/',
        pathNot: '\\.test\\.ts$',
      },
      to: {
        path: '^src/adapters/([^/]+)/',
        pathNot: '^src/adapters/$1/',
      },
    },
  ],
  options: {
    doNotFollow: {
      path: 'node_modules',
    },
    tsPreCompilationDeps: true,
    tsConfig: {
      fileName: 'tsconfig.json',
    },
  },
}
