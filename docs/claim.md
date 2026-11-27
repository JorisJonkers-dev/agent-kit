# The `claim` command

`claim` lets agents on different machines agree on who works a ticket. A claim is the git ref
`refs/claims/<key>` in a repository. Creating it with a create-only push is atomic, so exactly one
agent wins; the claim commit's message records the device, the run, the issue and when the lease
expires.

```sh
claim acquire <repo> <key> --issue <n> [--lease-seconds S] [--device D] [--run R]
claim renew   <repo> <key> [--lease-seconds S] [--device D] [--run R]
claim release <repo> <key> --issue <n> [--comment TEXT]
claim list    <repo>
```

`<repo>` is a git URL or path, or an `owner/name` GitHub slug.

## Leases

A lease lasts 3600 seconds unless `--lease-seconds` says otherwise. `renew` extends it for the same
device and run. A claim whose lease has expired can be taken over by `acquire`; the takeover pushes
with a lease on the old value, so two agents taking over at once cannot both win.

## On GitHub

For an `owner/name` repository, a won `acquire` assigns the issue to `ExtraToast` (`--owner`), adds the
label `status: in-progress` (`--label`) and comments who holds it until when. `release` removes the label.

## Exit codes

0 won (or the other command succeeded), 1 lost or failed, 2 usage error.
