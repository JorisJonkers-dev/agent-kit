---
name: add-public-service
description: Add a new public SSO-protected service to personal-stack by keeping fleet.yaml, auth-api permissions, app-ui registry/icons, render output, and tests aligned.
---

# Add Public Service

A new public app must line up across routing, auth, and the app catalog.

1. Add the service to `platform/inventory/fleet.yaml`:
   `service_intent.kubernetes.public_apps`, `placement_intent.*`,
   `exposure_intent.*` (`public` or `public_and_lan`),
   `access_intent.sso_protected`, `access_intent.host_labels`, and
   `ingress_intent.kubernetes_backends`.
2. Add an enum entry in
   `services/auth-api/.../domain/model/ServicePermission.kt`. Its subdomain
   values must match the public hostnames because forward-auth resolves
   permission from host.
3. Add host mapping rows to
   `services/auth-api/.../ServicePermissionTest.kt`.
4. Add the MyApps registry entry in
   `services/app-ui/src/features/apps/data/serviceRegistry.ts` and a local
   `services/app-ui/public/icons/<name>.svg` icon.
5. Add an `infra/scripts/make-admin.sh` grant row when non-admin users need the
   permission.

After editing, use `$fleet-change` for rendering and `$run-tests` for the
affected auth/app suites. Commit source edits and regenerated manifests
together.

Before inventing a host, Vault path, secret name, or exception rule, grep the
repo and check live state. For CLI access to SSO hosts, prefer `kubectl
port-forward` over public ingress path exceptions.
