## When a delegation fails or cannot start

A delegation does not come straight back when it fails. It is retried, climbs
to a stronger model, and swaps a client that cannot start, all inside one `run`
call, and every step is a run of its own with its own run id, run directory and
ledger line. `"escalate": false` turns all of it off; `max_attempts` (default
3: the first attempt, the retry, one climb) is how many runs a delegation may
make in all.

- **A failed result** is a run whose status is `failed`: a timeout, output the
  adapter cannot parse, a non-zero exit, or a writing run whose check failed.
  Refusals (`policy-refused`, `provider-not-allowed`, `cap-exceeded`,
  `no-grant`, `no-estimate`, `workflow-ended` and the other error codes) are
  errors before anything is launched and are never retried. The raw or partial output stays in
  the run directory (`output.json`, `stderr.txt` and `events.jsonl`) beside the
  `reason` and `failure` in `result.json`.
- **Retry.** The first failure is retried once on the same client model. The
  retry's prompt is the caller's prompt plus `A previous attempt failed:
  <reason>; <the last 1,000 characters of the check's output for a failed
  check, else of the client's stderr, else of its output>`. A writing run's
  retry gets a fresh worktree; the failed one is kept like any other until the
  workflow ends.
- **Climb.** A second failure moves to the next stronger subsidised model: a
  catalogue `tier` higher than the failing model's, whatever the failing
  model's own cost class, in the repository's permitted list (the family
  allowlist and the visibility rules, through the data policy above), never a model
  the machine disabled, ordered by tier and then catalogue order. It never
  moves into a metered class (a metered-personal run climbs only to a
  subsidised model, and a metered-employer model is never a climb target at
  all), and never to a model the catalogue gives no tier. A model with no
  catalogue tier is never climbed to or swapped to, and is retried once and
  then returned, because there is no way to say which model is stronger. A
  climbed model that fails is retried once, then climbed from again. When no
  stronger model is left, or `max_attempts` is spent, the last result comes
  back with `"escalation": "exhausted"`. An attempt whose client could not
  start does not count toward `max_attempts`, though it is listed in
  `attempts`.
- **Fall-through.** A client that cannot start is swapped for another
  permitted client model of the same cost class whose tier is at least the
  requested model's. Cannot start means what `usage` reports as
  `unavailable` or `unable_to_start` for the client (not installed, signed out,
  a plan window at 100%, a recent rate-limit or auth error), or a run whose
  `reason` is `launch-failed`, `rate-limited` or `auth`; a swap is not a retry
  and does not use the retry. A metered-employer model is in that pool only
  while a live grant covers the workflow, so a Codex model is swapped in only
  when the owner allowed it; `no-grant` and `cap-exceeded` are refusals, which
  no attempt ever retries. The gateway never moves into a costlier cost
  class on its own: when no client model of the cost class can start it
  returns `{"status":"unavailable","workflow","client","model","message",
  "reasons":[{"client","model","reason"}],"attempts":[...]}`, exit 0, and the
  orchestrator decides. That `unavailable` is only for a delegation no attempt
  of which ever ran: when an earlier attempt really ran and failed and a later
  one cannot start with nothing to swap to, the last real result comes back
  instead, with `escalation: "stopped"` and `stopped_reason` naming what the
  next attempt could not start with. In a `nedap` repository nothing is
  permitted, so nothing is started there at all.

- **The family is binding by default.** The model a delegation names fixes its
  family, and the retry, the climb and the fall-through stay inside that family.
  A delegation that no model of its family can answer ends failed (or
  `unavailable`, whose message says the family is binding) with its cause; it is
  never answered by another family. This is the default for every delegation
  because a caller who names a model almost always wants that family's opinion
  (review, critique, second opinion), and an answer from the orchestrator's own
  family defeats that. `"family_binding": false` lets the gateway move to another
  family. When it then does, the summary of the answer opens with a line naming
  both families and the cause (the whole summary is still cut at
  `summary_max_chars`), for example `escalated from mistral (vibe:
  model-unavailable) to anthropic`; a batch report shows the same line in that
  delegation's summary.

The result of a delegation is the last run's result plus `attempts`, a short
list of `{run_id, client, model, status, reason?}` in order (one entry when
nothing went wrong, or with `escalate: false`); `skipped`, the client models
passed over because they could not start; `escalation: "exhausted"` as above;
and, on a retry or a climb, `attempt` (its place, 2 and up) and `attempt_of`
(the first run's id), which each such run's ledger line carries too. A later
attempt the gateway refuses (the workflow ended meanwhile) ends the delegation
with the last result, `escalation: "stopped"` and `stopped_reason` naming the
refusal. The first attempt's refusals are errors as always.

## Web tools in a Vibe run

Vibe does not expose `web_search` and `web_fetch` directly: a headless run
reaches them through `search_tool_functions`, which loads them on demand.
`--enabled-tools` removes `search_tool_functions`, so a run that names a web
tool in `--enabled-tools` has no way to the web and used to stall without
saying why. The gateway now refuses such a Vibe run before its first model
call: a non-zero exit, `code: "extras-not-supported"`, and a message naming the
tool (`vibe cannot enable the web_fetch tool: ...`). Nothing is launched.

Through the gateway, leave the `tools` extra unset: the launcher then passes no
`--enabled-tools`, so Vibe keeps its default tool set including the on-demand
web tools. The gateway has no extra that passes `--disabled-tools`.

Workaround when running Vibe directly (outside the gateway): keep Vibe's
default tool set and remove the tools the run must not have with
`--disabled-tools`, for example
`vibe -p "..." --model <alias> --disabled-tools bash --disabled-tools write_file --disabled-tools edit --disabled-tools run_typescript --auto-approve`.
With that, `web_search` and `web_fetch` work. A delegation that needs the web
through the gateway can instead go to a client with its own web access.

