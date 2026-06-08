---
name: fleet-change
description: Edit personal-stack platform/inventory/fleet.yaml safely by rendering generated Traefik/catalog YAML, running platform validation, and committing generated output with the source change.
---

# Fleet Change

`platform/inventory/fleet.yaml` is the source of truth. Traefik IngressRoutes
and catalog ConfigMaps under `platform/cluster/flux/apps/` are generated from
it. CI re-renders and diffs those files, so a fleet change without committed
render output fails validation.

After editing `fleet.yaml`, run:

```bash
for s in platform/scripts/render/*.sh; do bash "$s"; done
./gradlew :platform:tooling:test
kubectl kustomize platform/cluster/flux/clusters/production > /dev/null
```

Stage the fleet edit and every regenerated file in the same commit. Verify a
fresh render is stable before pushing:

```bash
for s in platform/scripts/render/*.sh; do bash "$s"; done && git diff --stat
```

An empty diff means the committed tree matches a clean render.

Notes:

- A new WAN-origin override needs a matching `purpose` keyed site IP in
  `sites.*.networking.wan_public_ip` and validator coverage in
  `PlatformFleetLoader`.
- Adding a public service touches auth and app catalog files too; use
  `$add-public-service`.
