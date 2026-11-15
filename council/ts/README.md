# Runner sources

TypeScript package holding the sources of the runners the skills ship as single
files:

- `src/critique-cli` (the critique context) bundles into
  `../../skills/workstation/critique/critique.mjs`, beside the prompts, lenses,
  schema and routing it reads.
- `src/model-gateway-cli` bundles into
  `../../skills/workstation/model-gateway/model-gateway.mjs`, beside the defaults
  it reads.
- `src/build-tracker-cli` bundles into
  `../../skills/workstation/build-tracker/build-tracker.mjs`.

`pnpm run build` rebuilds the three bundles. They are committed, so run the build and
commit its output with any change to the code they hold, then re-pin their
`sha256` values in `manifest.yaml` (`scripts/update-manifest-hashes.ts`).
`src/contexts/graph` and `src/contexts/watchdog` stay because the critique runner
imports the graph's git adapter and the watchdog's retry policy.
