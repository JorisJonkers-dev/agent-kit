import { defineConfig } from 'vitest/config'

export default defineConfig({
  test: {
    coverage: {
      exclude: ['../ts-dist/**', 'dist/**', 'node_modules/**'],
      provider: 'v8',
      include: ['src/**/*.ts'],
      thresholds: {
        lines: 100,
        statements: 100,
        functions: 100,
      },
    },
  },
})
