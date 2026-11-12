# The Gold Band headless proof

Every [Gold Band workflow](../CONTEXT.md) checked into
`installer/gold-band/workflows/` is run headless against a stub agent before
any real run, and a real run is refused when that proof is missing, skipped or
failed (spec #392 user stories 72 and 79; issue #447). The harness is the
`gold-band-proof` crate in the cargo workspace (std plus `serde_json`).

## What a proof does

`gold-band-proof prove --workflow <FILE>` builds a throw-away Gold Band home in
a temporary directory:

* a `settings.json` that declares **every agent id the workflow names** (node
  `provider`s and `modelBindings` agent ids) with the harness itself, run as
  `gold-band-proof stub-acp`, as its command. The stub is a small ACP agent
  that answers every prompt with one fenced json block, so no model and no
  network is spent. What the block holds is the agent's **role**, which the
  harness reads off the workflow and passes as `STUB_ACP_ROLE`:

  | Role | Agent id | The stub answers |
  | --- | --- | --- |
  | `findings` | anything else | the findings document for the verdict |
  | `questions` | a node whose `output.schema` has an `open_questions` key | `{"open_questions": []}`, or one open question under the verdict `open-questions` |
  | `answer` | a node whose `output.schema` is an object without a `findings` key | `{"ok": true}` |
  | `dynamic:<agent ids>` | an agent id an `ai-dynamic` node's `agentStrategy` names | for an AI-DYNAMIC internal prompt, the node's `dynamic-node-completion`: the bootstrap fans out to one worker per listed agent id and every other internal node ends; any other prompt gets `{"ok": true}` |
  | `verdict:<n>` | `verdict-<n>` | `{"actionable": 1}` under the verdict `actionable=K` when `n <= K`, else `{"actionable": 0}` |
  | `gate:<name>` | `gate-<name>` | `{"exit_code": N}`, the exit code `gold-band-gate run` earned |
  | `copy:<name>` | `copy-<name>` | the gate report `gold-band-gate verify` printed, or a report-shaped refusal carrying its exit code |

  A gate shim runs `true`, or `false` when the scenario's verdict is
  `gate-fail=<name>`, under the fixed run id `proof-run`, attempt `1` and a
  30-second timeout, with its report directory at
  `<root>/reports/<name>` (`STUB_ACP_REPORT_DIR` names `<root>/reports`). The
  copier verifies the same command, so a failed gate leaves a report the
  copier refuses with `failed-command`;
* a minimal profile file for each profile the workflow's nodes name (Gold Band
  refuses a node whose profile is missing);
* a workspace that is a git repository with one empty commit and a
  `requirement.md`.

It then runs `gold-band run --workflow <FILE> --requirement-file requirement.md`
once per scenario and compares the exit code with what the scenario expects. It
also checks that the printed run state agrees with the exit code (Gold Band's
mapping: completed and success is 0, completed otherwise 1, paused 2).

| Scenario | Stub verdict | Expected exit | Added when |
| --- | --- | --- | --- |
| `clean` | empty findings, no open questions | from the walk (0, or 2 when a manual check is reached) | always |
| `needs-human` | one major finding | from the walk (2 when a failure edge leads to a manual check) | a findings node is reached |
| `open-questions` | one open question | from the walk | a questions node is reached |
| `actionable-<k>` | the critics' findings are actionable in rounds `1` to `k` | from the walk (0 when a repair clears them, 2 when the escalation's manual check is reached) | a verdict node `verdict-<n>` is reached, one scenario per round |
| `gate-fail-<name>` | the gate's command fails | from the walk (1 without a failure edge) | the workflow has a `gate-<name>` agent that the segment reaches, one scenario per gate, sorted by name |

The expected exit code comes from walking the workflow graph under the
scenario's verdict (`harness::walk`): a node fails when its stub does (a gate or
copier named by `gate-fail=`, a findings node under `needs-human`, a questions
node under `open-questions`), the walk follows the edge for the outcome, a
manual check pauses (2), a missing edge ends the run (0 after success, 1 after
failure) and `$end` ends it. A scenario is added only when it fails a node, so
a gate that sits behind a pause is not claimed by the first segment, and a failure of the same nodes that an earlier segment already proved is not run again in a later one (the key is every node the walk fails, in order, so `actionable-1` and `actionable-3` are both kept).

### Segments

A paused run cannot be continued headlessly (`gold-band continue` refuses a run
that waits for user input), so what lies after a manual check would never run.
For each manual-check node with a `success` edge to another node, the harness
also proves the stretch after it: it writes a pruned copy of the workflow under
the scenario's directory as `segment.json` (the nodes reachable from that edge's
target, their edges and bindings, `entry` set to the target; Gold Band refuses a
node that is unreachable from the entry) and runs the same scenarios against it,
named `after-<manual check id>.<scenario>`. The first segment keeps the plain
names. Each segment starts from nothing, so a gate and its copier must lie in the
same segment, which a workflow gets by never putting a manual check between them.

More scenarios come from `--scenario NAME:VERDICT:EXIT` (verdicts: `clean`,
`needs-human`, `fail`, `gate-fail=NAME`; `fail` makes the stub answer a prompt
with a protocol error, which Gold Band pauses as `runtime-abnormal`).

## The proof record

`prove` writes `<record-dir>/<workflow id>.proof.json` (default record dir
`$XDG_STATE_HOME/agent-kit/gold-band/proofs`, else
`~/.local/state/agent-kit/gold-band/proofs`): the workflow id and the SHA-256
of its file, the Gold Band commit (from `--gold-band-commit`, else the build
record setup writes at `~/.local/state/agent-kit/gold-band/build.json`), each
scenario's expected and actual exit code and run state, and an outcome of
`passed`, `failed` or `skipped`.

Without a Gold Band binary (`--gold-band` or `GOLD_BAND_BIN`) `prove` does not
fail: it prints `gold-band-proof: skipped: no Gold Band binary supplied ...`,
records the proof as `skipped` with that reason, and exits 0. A skipped proof
is never a passing one.

Exit status of `prove`: `0` passed or skipped, `1` a scenario did not behave as
expected, `2` bad usage or an unreadable workflow.

## Refusing an unproven run

`gold-band-proof run --workflow <FILE> -- <gold-band run arguments>` is how a
real run of a checked-in workflow starts. It first checks the record and
refuses, with exit **5** and the reason on stderr, when:

1. no record exists for the workflow;
2. the proof was skipped (the recorded reason is printed);
3. the proof failed (the first scenario that did not pass is named);
4. the workflow file changed since the proof (hash differs);
5. the installed Gold Band build (the build record's commit) is not the one the
   proof ran on;
6. the record claims a pass but holds no scenarios or a failing one.

Otherwise it runs `gold-band run --workflow <FILE> <arguments>` and returns
Gold Band's own exit code. `gold-band-proof check --workflow <FILE>` is the same
check without starting a run (exit 0 or 5).

## CI

The `Gold Band Headless Proof` job runs on Linux and macOS. It builds the
harness, fetches the Gold Band binary the repository supplies (repository
variables `GOLD_BAND_BIN_URL_LINUX` / `GOLD_BAND_BIN_URL_MACOS` with the
matching `..._SHA256_...`), and runs `prove` over every workflow in
`installer/gold-band/workflows/`. When no binary is supplied for an OS the job
prints the skip reason and passes without proving anything.

### ai-dynamic nodes

An `ai-dynamic` node needs a planner that proposes its internal nodes, which no
fixed answer covers. The harness declares every agent id of its `agentStrategy`
(`provider`, `bootstrapProvider` and each `availableAgents` provider) with the
`dynamic` role. The stub reads the node it is asked to run from Gold Band's runtime
context in the prompt; the bootstrap writes the handoff files Gold Band requires
(`handoff/summary.md` and one task file per worker, the merge and the acceptance
under the node's attachments directory) and answers a fan-out with one worker per
listed agent id, and every other internal node ends. An ai-dynamic node never fails
in the walk. What a fanned-out critic writes, and what the finding check reads, is
not proved: the gate shim runs `true`, so the proof checks the node's limits,
sequencing and routing only.

## Not covered

The stub answers with a findings document, `{"ok": true}`, a gate exit code or
a gate report, so the proof checks structure and routing for the spec-stage
template and for Workflow B's gates. A workflow whose nodes need another output
shape adds its own role and its own stub reply in the ticket that ports it.
