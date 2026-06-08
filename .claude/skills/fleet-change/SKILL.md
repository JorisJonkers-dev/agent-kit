---
name: fleet-change
description: Edit platform/inventory/fleet.yaml safely — render the generated Traefik/catalog YAML, run the tooling tests, and commit the rendered output so CI's Platform Validate passes.
---

# Changing fleet.yaml

`fleet.yaml` is the source of truth; the committed Traefik IngressRoutes
and catalog ConfigMaps under `platform/cluster/flux/apps/` are
**generated** from it. CI's "Platform Validate" job re-renders and diffs
against the committed tree, so a fleet edit that isn't re-rendered and
committed fails CI.

After editing `platform/inventory/fleet.yaml`, run every render script
(there are five — the catalog/route/ingress/lan/gatus renderers),
the tooling tests, and the kustomize build:

```bash
for s in platform/scripts/render/*.sh; do bash "$s"; done
./gradlew :platform:tooling:test
kubectl kustomize platform/cluster/flux/clusters/production > /dev/null
```

Then stage **both** the fleet edit and every regenerated file in the
same commit. Verify a fresh re-render produces no diff before pushing:

```bash
for s in platform/scripts/render/*.sh; do bash "$s"; done && git diff --stat
```

An empty diff means the committed tree matches a clean render. The
pre-commit hook runs `prettier --write` on YAML; the renderer output is
prettier-stable, so this should not introduce drift — but the empty-diff
check above is what proves it.

## Notes

- A new WAN-origin override (e.g. `edge_direct`) needs a matching
  `purpose`-keyed site IP in `sites.*.networking.wan_public_ip` and a
  validator branch in `PlatformFleetLoader`; the renderer test asserts
  the rendered `external-dns` target/proxied annotations.
- Adding a public service touches several aligned files — use the
  `add-public-service` skill.
