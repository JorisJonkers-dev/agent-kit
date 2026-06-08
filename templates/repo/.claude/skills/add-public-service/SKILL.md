---
name: add-public-service
description: Add a new public (SSO-protected) service to the stack — the set of files across fleet.yaml, auth-api, and app-ui that must stay aligned, plus render + test.
---

# Adding a public service

A new public app touches several files that must agree, or
forward-auth, the MyApps grid, or Platform Validate break.

1. **`platform/inventory/fleet.yaml`** — add the service to
   `service_intent.kubernetes.public_apps`, `placement_intent.*`,
   `exposure_intent.*` (`public` or `public_and_lan`),
   `access_intent.sso_protected`, `access_intent.host_labels`, and
   `ingress_intent.kubernetes_backends` (namespace/service/port/health).
2. **`services/auth-api/.../domain/model/ServicePermission.kt`** — add an
   enum entry whose subdomain(s) match the host(s). Forward-auth resolves
   the permission from the host via `fromHost`; an unknown host is denied
   (ADMIN bypasses). Multiple hostnames for one service = `vararg`
   subdomains on a single entry.
3. **`services/auth-api/.../ServicePermissionTest.kt`** — add the
   `@CsvSource` row(s) mapping each host to the enum.
4. **`services/app-ui/src/features/apps/data/serviceRegistry.ts`** +
   `services/app-ui/public/icons/<name>.svg` — the MyApps card (a
   hand-crafted placeholder icon is fine; don't auto-fetch assets).
5. **`infra/scripts/make-admin.sh`** — grant row (optional; ADMIN
   bypasses permission checks anyway).

Then render + validate (see the `fleet-change` skill) and run
`:services:auth-api:test` (see `run-tests`). Commit the fleet edit and
the regenerated IngressRoutes/catalogs together.

## Reuse, don't invent

Before adding a Vault path, secret name, or host, grep the repo and
check live state — existing secrets/paths are often already there under
a different name. Prefer the simplest shape that matches the mental
model (`sso_protected` = whole host behind forward-auth, not per-path
allowlists). For CLI access to an SSO host, `kubectl port-forward`
rather than carving path exceptions into the public ingress.
