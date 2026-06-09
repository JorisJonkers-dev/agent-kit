# Platform Blueprints Handoff

`platform-blueprints` should consume the agent runtime package contract as an
input and decide how to render a cluster deployment. The blueprint must not
assume values from any one consumer.

## Required Blueprint Inputs

| Input | Owner | Notes |
| --- | --- | --- |
| Runtime image reference | Consumer or image build pipeline | Repository, tag, digest, and pull policy are deployment values. |
| Namespace and labels | Platform blueprint | Must be configurable per cluster. |
| Runner service account | Platform blueprint | Runner service-account token should be disabled unless explicitly needed. |
| Orchestrator service account/RBAC | Platform blueprint | Scoped to the namespace and only to resources the orchestrator creates. |
| Credential persistence | Platform blueprint | PVC, volume, or external store; access mode and storage class are consumer inputs. |
| Secret projection | Platform blueprint | Source paths, secret names, and key names are consumer inputs. |
| MCP profiles | Runtime plus platform | Agent-kit defines file/profile shape; blueprint supplies profile data and endpoint values. |
| Bootstrap workflow | Platform blueprint | Interactive auth bootstrap is cluster-specific and optional. |
| Refresh workflow | Platform blueprint | Probe commands and schedule are deployment policy. |
| Kit install workflow | Platform blueprint | Installer URL, bearer source, cadence, and target homes are deployment policy. |
| Runner Pod factory | Platform blueprint and orchestrator | Resources, security context, volumes, env, and mounts are rendered by the platform side. |
| Scheduling | Platform blueprint | Placement constraints, topology, tolerations, and node labels are consumer inputs. |
| Network policy | Platform blueprint | Egress categories and concrete endpoints are deployment inputs. |
| Observability | Platform blueprint | Metrics, logs, alerts, and dashboards belong to the platform pack. |

## Non-Goals For Agent Kit

- No Kubernetes manifests.
- No Flux or Kustomize base.
- No namespace, RBAC, PVC, Secret, Service, CronJob, Deployment, or NetworkPolicy
  resources.
- No credential source paths or literal endpoint URLs.
- No default cluster placement, storage class, or hostPath policy.

## Future Renderer Boundary

A future platform renderer may read an `AgentRuntimePackage` and a deployment
intent file, then emit Kubernetes resources. That renderer belongs outside this
repository. The renderer should treat every concrete value in the deployment
intent as consumer-supplied and should fail closed when required secrets,
endpoints, or placement values are absent.
