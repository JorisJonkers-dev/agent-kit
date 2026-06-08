---
name: run-tests
description: Select and run the right personal-stack validation commands for Kotlin services, platform tooling, and Vue/TypeScript UIs before pushing.
---

# Run Tests

Compile-only is not enough. Run the actual suites for the touched area.

Kotlin services:

```bash
./gradlew :services:<svc>:test
./gradlew :services:<svc>:integrationTest
./gradlew :services:<svc>:ktlintCheck :services:<svc>:detekt
```

Keep DTO/OpenAPI changes synchronized with `exportOpenApiSpec` and the UI
contract generation when applicable.

Platform tooling:

```bash
./gradlew :platform:tooling:test
```

Use with `$fleet-change` for inventory/render changes.

Vue/TypeScript UIs:

```bash
cd services/<ui>
npm run typecheck
npm run lint
npm run test
```

Report blocked checks explicitly, including the missing local dependency such
as Docker/Testcontainers or `kubectl`.
