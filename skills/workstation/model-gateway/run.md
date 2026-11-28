## The `run` command

`run` hands one task to one client model, read-only or writing, and reports
what came of it. It prints one JSON object on stdout and exits 0, whether the delegation
succeeded or failed; only a refusal is an error, and it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
model-gateway run --json '{"workflow":"review","client":"vibe","model":"mistral-large-latest","mode":"read-only","prompt":"Review src/app.ts","repo":"/path/to/repo"}'
model-gateway run --input <file.json>
```

The input is `workflow`, `client`, `model` (an alias `models` listed), `mode`,
`prompt`, and optionally `repo`, `carries` (`{"code":bool,"history":bool}`,
only for an inline run), `time_limit_s` (default 900), `max_price_usd` and
`provider` (which a Hermes run must name, and which this machine must allow),
`escalate` (default `true`; `false` turns off the retry, the climb and the
fall-through below), `family_binding` (default `true`; see below), `free_fallback` (default `true`:
a `free/<model>` run that finds no free copy falls back once to OpenCode's
`opencode/<model>-free`, reported in `fallback`, `available_again_in_s` /
`available_at` and `recommendation`; never for `client_pinned: true`, never
for a repository that is not public, independent of `escalate`),
`settings.retry_wait` (see "Waiting for a free model" below),
`max_attempts` (default 3), `headless` (a boolean that
changes nothing else: a run marked headless is refused and launched exactly
like any other), `tier` (see below) and `free_only` (a boolean; see below).
`free_only: true` refuses the run, before anything starts, with code
`free-only-refused` unless the model is `free` (the one definition above); it also turns off the retry,
the climb and the fall-through, so a free-only run never moves to another
model. A non-boolean is `invalid-input`.

A run may pin its harness with a model address in place of `client` and
`model`: `"model": "<harness>:<pricing>/<model>"` and no `client`, for example
`pi:free/glm-4.7-flash`, `pi-free:free/hy4-flash` or
`opencode:free/deepseek-v4.1-flash`. The harness is `pi` (the Pi route of the
pricing prefix, so `pi:free/x` runs on `pi-free`), a Pi route itself
(`pi-<route>`, which must serve that pricing prefix) or `opencode` (which
reaches the proxy as `litellm-<pricing>/<pricing>/<model>`); the proxy's own
address, `<pricing>/<model>`, reaches it unchanged and the proxy picks the
provider. The pricing prefix is `free`, `paid` or `auto`; an unknown one is
refused with `unknown-pricing`, and an unknown harness, or a Pi route that does
not serve the prefix, with `unknown-harness`, each naming the ones there are.
A `<harness>:` address pins the run (`client_pinned: true`): the gateway never
swaps its harness, the free fallback included. Naming `client` and `model`
alone does not pin; set `client_pinned: true` to pin that way. The data policy,
the free-only rules and the repository's scope (public, `nedap`, private)
apply to a pinned run as to any other. No profile, tier or critique route may
name a provider's own model id on a provider's route; the ones that still do are
listed in `tests/address_audit.rs`, and the list can only shrink.

### Waiting for a free model

When every free copy of a model is briefly limited the proxy answers a 429
that names when the first is back (`... the first is back in 5s`). A run that
ends on such a refusal waits that time plus `margin_s` and runs the same model
again, at most `max_retries` times and `max_total_wait_s` seconds in all
(`settings.retry_wait`: 3, 60 and 1 by default; `0` for either turns the wait
off, and a machine file may set any of them). Each retry is a run of its own:
its `events.jsonl` starts with a `retry` event and its result carries a `retry`
object (`retry`, `of_run_id`, `reason`, `back_in_s`, `waited_s`,
`total_waited_s`, `max_retries`, `max_total_wait_s`). Other 429s, a refusal that
names no time and every other failure behave as before.

The order with the free fallback: a wait that fits is waited out and the same
model retried first (this holds for a pinned run too, since the model and the
harness do not change); a wait longer than what is left of the cap is not
taken, and nor is one after the retries are used up: the run then falls back
once to OpenCode's free copy as above (never for a pinned run). So a short
limit costs seconds on the same model, and a long one moves on at once.

A run may name a LiteLLM tier instead of a client and a model: `tier` (or
`--tier T`), where `T` is one of the tiers the defaults state (`tier-free`,
`tier-vibe`, `tier-go-cheap`, `tier-strong-open`, `tier-review`, rendered from
`registry/litellm-tiers.json`). The tier picks the profile, the client and the
model, so it is not combined with `profile`, `harness`, `client` or `model`
(`invalid-input`). The run goes through the tier's Pi route and sends the
tier's name as the model, and LiteLLM walks the tier's chain. An unknown tier
is `unknown-tier`, never a default; a tier this machine holds no key for is
`tier-key-missing`. The tier is chosen by what is left of its quota: at 80% of
a window of the plan it draws on, the run drops to the next cheaper tier on the
ladder and never to a dearer one, and a tier takes only
`settings.quota.parallel` runs at once (`tier-busy` when none frees up within
`slot_wait_s`). `docs/model-gateway/quota-tiers.md` has the ladder, the signals
and the settings.
`workflow` is required (`code: "invalid-input"`), and is a name: a letter or
digit, then letters, digits and `. _ : / @ -` only, at most 80 characters. Every
command that takes a workflow name holds it to that rule, because the name
reaches the shell command the gateway suggests. A run can go to `client:
"vibe"`, `"agy"` (Antigravity), `"claude"` (Claude Code), `"opencode"`,
`"hermes"`, `"codex"` or a Pi client named `pi-<route>` (`pi-vibe`,
`pi-opencode-go`, ...), in `mode: "read-only"` or `"write"` (below); any other
client or mode is `code: "not-supported-yet"`. A Pi client the configuration
does not declare is `unknown-model`, and one that names a `provider` of its own
is `provider-not-allowed`: the provider is the route's.

Everything that can refuse the run happens before anything is launched:

- `nested-delegation` — the gateway sets `AGENT_KIT_DELEGATION=1` in every
  delegate's environment and refuses any call made under it, so a delegate
  cannot delegate on.
- `policy-refused` — the data policy (above) for `repo`, naming the rule. A run
  with a `repo` reads that repository, so it counts as carrying code and needs
  `may_send.code`; `carries.history` needs `may_send.history`. A run without a
  `repo` is inline-only: it works in a fresh empty temporary directory, never
  inside a repository, and `carries` may not claim code or history the policy
  does not allow.
- `unknown-profile` and `profile-refused` — the run names a delegation profile
  (below) the defaults do not state, or the profile refuses it, with the reason.
- `write-needs-repository` — a writing run names no `repo`.
- `provider-not-allowed` — a Hermes run must name a `provider`, and it must be
  one this machine lists in `clients.hermes.providers` (empty by default, so
  every Hermes run is refused until the owner names one). `openrouter` is
  refused whatever the configuration says, and so is a model id that starts
  with `openrouter/`, on every client: free models run only as `free/<model>`
  on `pi-free`.
  OpenCode's own `opencode-go/<id>` provider names no LiteLLM proxy address,
  which the sandbox requires, so a run that names it is run as
  `litellm-opencode-go/<id>` when this machine lists that model (the result's
  `rewritten` and a `route.rewritten` event say so, and `models` lists only the
  LiteLLM name), and is refused naming it when it does not. An OpenCode model is named
  `<provider>/<model>`, and only the providers `clients.opencode.providers`
  names (default `["opencode-go", "litellm-opencode-go"]`, the second being the
  provider setup writes for the LiteLLM OpenCode Go route) are allowed; every
  other provider is refused, and so is `opencode/<model>`, OpenCode's
  pay-per-token Zen, even when the machine lists it: a run never reaches a
  pay-per-token model. The one exception is OpenCode's own free models, named
  `opencode/<id>-free` (an id ending in `-free`; a zero-cost one without the
  suffix, such as `opencode/big-pickle`, stays refused). They are cost class
  `free`, so they carry **public data only**: a public repository or inline
  public content, never a private repository. Only `opencode/deepseek-v4.1-flash-free`
  and `opencode/hy4-flash-free` keep nothing and train on nothing (tier
  `routine`); the others (`mimo-v2.6-flash-free`, `nemotron-3.5-lightning-free`,
  `nemotron-3.5-super-vl-free`, `beam-free`, `flint-40b-free`, `iceman-free`,
  `seaglass-alpha-free`, `tidepool-alpha-free`) may keep or train on prompts,
  are tier `trivial`, and no profile picks them, though a run may name them for
  public data. They work only through the OpenCode CLI, and OpenCode's quota
  is per IP address. Their sandbox reaches `opencode.ai:443` and nothing else,
  needs no sign-in (the run's home is bare, and the owner's `opencode.db` is not
  given to it). When
  every model OpenCode lists is on a provider the list does not name, `models`
  names `opencode` in `missing` with the reason instead of offering nothing.
- `workflow-ended` — the run names a workflow that ended, so its caps and
  grants lapsed with it (below).
- `no-grant` — a Codex run whose workflow no live grant covers. The message
  names the workflow and the `grant write` call that would add one. It applies
  to every mode, to every attempt of a delegation, and to a `headless` run.
- `cap-exceeded` — the run's estimate does not fit what its cap has left: the
  metered-personal caps for a metered-personal run, the grant for a Codex run
  (below).
- `worktree-failed` — git could not add the throwaway worktree, or create a
  writing run's worktree off its base (below). A
  worktree git cannot remove after the run is not an error: the result is
  returned with a `cleanup_warning`.
- `unknown-model` — a model the catalogue cannot place, or one this machine
  disabled.
- `model-not-usable` — the live LiteLLM catalogue says the model cannot be used
  on its client's route: blocked (with the provider's reason) or not probed
  usable. A machine with no readable catalogue never refuses on it.

