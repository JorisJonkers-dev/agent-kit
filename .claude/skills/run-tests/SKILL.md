---
name: run-tests
description: Run the right tests before pushing a change in this monorepo — Kotlin services (unit + integration, not just compile), the platform tooling, and the Vue/TS UIs (typecheck + lint + vitest).
---

# Validating a change before pushing

Compile-only is not enough — run the actual suites for what was touched.

## Kotlin services (`services/<svc>`)

```bash
./gradlew :services:<svc>:test            # unit + integration (Testcontainers)
./gradlew :services:<svc>:integrationTest # if a separate source set exists
./gradlew :services:<svc>:ktlintCheck :services:<svc>:detekt
```

Every custom service except `system-tests` enforces ≥80% Jacoco
coverage and ships both unit and integration tests. A DTO change must
keep the OpenAPI contract in sync — regenerate `openapi.json`
(`exportOpenApiSpec`) **and** the UI's `generated.ts`
(`contract:generate`), or the Validate OpenAPI Contract job fails.

## Platform tooling

```bash
./gradlew :platform:tooling:test
```

Gates `fleet.yaml` changes. (Local runs inside a nested git worktree
show ~13 pre-existing `repositoryRoot`-resolution failures that are
clean in CI — confirm the test you touched passes and the failures are
the unrelated node/IP-fixture set.)

## Vue / TS UIs (`services/*-ui`)

```bash
cd services/<ui> && npm run typecheck && npm run lint && npm run test
```

`lint` runs with `--max-warnings 0`. Detekt and ktlint cap lines at 120
chars; test titles are lowercase-first (`describe('sessionTabs', …)`).

## Discipline

Test boundary shapes (empty body, non-JSON, alternate render branches),
describe the bug in the test, and add a test file when introducing a new
module. Don't claim UI work succeeds without loading the page.
