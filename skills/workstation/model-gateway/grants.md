## Grants: spending the owner's employer money

Codex is the one `metered-employer` client: its runs are billed to the owner's
employer, not to the owner. A Codex run therefore needs a **grant** — the
owner's permission for one named workflow, with a dollar cap — and the gateway
never writes one itself and never renews one.

**Ask the owner first.** A grant is written by the agent, after it asked the
owner and got an answer, and it records both the question and the answer so a
grant written without asking stands out. Never write one on your own
initiative, and never widen one: a bigger cap or a longer life is a new
question to the owner.

```sh
model-gateway grant write --json '{"workflow":"spec-293","client":"codex","cap_usd":5,"question":"may I spend up to $5 of nedap money on spec-293?","answer":"yes, up to five dollars"}'
model-gateway grant list  --json '{}'
```

`grant write` takes `workflow`, `client` (a metered-employer client, `codex`
today), `cap_usd`, `question` and `answer`, and optionally `max_starts` (a whole
number above 0: the Codex node starts a Gold Band run may take, also given as
`--max-starts N`; `grant list` shows it), and prints
`{"status":"ok","grant":{...}}` with what the grant has left. It refuses an
empty `question` or `answer`, a `cap_usd` that is not above 0, a client that is
not metered-employer (`invalid-input`, naming its cost class), a client the
gateway does not know (`unknown-model`), a workflow that ended
(`workflow-ended`) and a workflow and client a live grant already covers
(`grant-exists`, naming that grant, its cap and what it has left): the owner
re-approves only after it lapsed, so a second grant can never quietly double
the cap. The duplicate check runs inside the grants file's lock, so of two
concurrent writes one is refused `grant-exists`. A ledger the gateway cannot
read refuses the write too (`ledger-unreadable`, naming the file): a standing
grant cannot be ruled out, so it fails closed. A grant lapses 24 hours after it
was written, or when its workflow's own 24 hours run out first, whichever comes
first.

**A delegate may not write a grant.** A gateway run carries the
`AGENT_KIT_DELEGATION` mark in its environment, and `grant write` refuses when
it is set (`code: "nested-delegation"`): only the agent the owner works in may
write one, after asking the owner. `grant list` stays allowed. A writing run's
`check` carries the mark too. The mark guards against accidents, not against a
determined delegate: a delegate that runs shell commands can scrub its own
environment, so it stops a careless nested call but not a deliberate one. The
real protection is that the owner reads the staged diff and the ledger, and
that grants are small and short-lived.

The write is the last thing that can fail: when the ledger cannot be read
afterwards, the result is still `ok`, with the grant it wrote, `"room": null`
and a `warning` saying what it could not read. Never ask the owner again
because of that warning — the grant stands.

`grant reserve` is the Gold Band work instance's per-node grant check. A Gold
Band run counts as one workflow, named `<gold band workflow>@<repository>@<run
id>`, and its grant's `max_starts` counts Codex node starts (one per start),
because the gateway never sees Gold Band spend. `grant write` takes it as
`"max_starts"` or `--max-starts N`; `cap_usd` stays dollars and is never read as
a start count. Before every Codex node,
`grant reserve --json '{"workflow":"<name>","client":"codex","node":"<node id>","attempt":1}'`
holds one start (a node and attempt that ask again hold the one start,
`"repeated": true`) against the grant and prints `{"status":"ok","reservation":{...}}`
with `max_starts`, `started` (this one included) and `remaining`. The decision and the
reservation line are one locked ledger update, so two parallel nodes cannot both
take the last start. A reservation holds for about a minute and then settles; a settled start still
counts, so a start is never given back. It refuses `cap-exceeded` when no start
is left, `no-max-starts` when the grant has no `max_starts`, and `no-grant` when no grant covers the workflow or the newest
one lapsed (its message names the grant and why: `workflow-ended` or
`expired`). The workflow need not have a record. The shim that calls it is
`gold-band-run-start grant-check`.

`grant list` prints `{"status":"ok","grants":[...],"lapsed":[...]}`. Each live
grant carries its `cap_usd`, `spent_usd` (what its workflow has spent on that
client's metered-employer runs since the grant was written), `reserved_usd`
(what live reservations of that client hold) and `remaining_usd`. A lapsed
grant is in `lapsed` with a `reason`: `workflow-ended`, `expired` or
`cap-spent`. A grant lapses as `cap-spent` only once settled spend reaches its
cap: one whose cap is merely reserved by a run in flight is still live, and the
next run that does not fit it is refused `cap-exceeded`. The gateway never
renews a lapsed grant; ask the owner again.

The grants live in the machine configuration,
`$XDG_CONFIG_HOME/agent-kit/model-gateway-grants.json`, else
`~/.config/agent-kit/model-gateway-grants.json`, under the same lock and
temporary-file write as the workflows. **Never inside a repository**: a grants
path that lies inside any git repository is refused, for reading and for
writing alike (`code: "grants-in-repository"`, naming the repository), because
a grant is the owner's and not the project's.

A grant covers one workflow **and one client**: a grant for `codex` never
covers another metered-employer client on the same workflow, and its room
counts only that client's runs and reservations.

A Codex run is refused `no-grant` when no live grant covers its workflow and
client, and `cap-exceeded` when its estimate does not fit what the grant has
left. Before it launches, the run's estimate — the same estimate the
metered-personal caps use — is reserved against the grant's remaining cap in
one locked critical section, and the run's actual spend replaces the
reservation when it ends. The gateway settles the reservations whose runs died
before it reads a grant, so a dead run's reservation never makes a grant look
spent for the run that asks next. A run under a grant is also clamped to the
time left: its `time_limit_s` never exceeds what remains until the grant's
`expires_at` and its workflow's 24 hours (or end), so a run launched just
before either keeps no spend running past it, and one launched with under 60
seconds left is refused `grant-expiring`, naming the time left. The check of a writing run gets only the time still left once the delegate is done (a failed run with reason `grant-expired`, and no check, when none is left). A refused run
reserves nothing and starts no workflow. The order is always `requireWorkflowOpen` → reserve →
`startWorkflow`.

## The check runs the delegate's code

The gateway runs the caller's `check` with `sh -c` inside the worktree the
delegate has just written, so the check is only as trustworthy as the delegate:
a delegate that edits the project's test entry point (the `test` script in
`package.json`, a `Makefile` target, a test runner's config) can make the check
run code of its own choosing, with your privileges and outside the worktree.
Send a writing delegation only to a client model you trust with that, and read
the staged diff before you commit it.

