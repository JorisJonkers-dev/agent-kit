## The data policy

A repository's GitHub visibility decides first, and the family allowlist
critique writes decides for every repository that is not public. It lives at `$XDG_CONFIG_HOME/agent-kit/critique-allowlist.json`, else
`~/.config/agent-kit/critique-allowlist.json`, and keys each repository by
`<host>/<owner>/<repo>` with `families`, `code` and `history`.

- A repository with an entry lists only the client models whose family the
  entry names, and each reports the entry's `code` and `history` in `may_send`.
- A repository with a remote but no entry, or with no remote, lists nothing and
  `policy` says why.
- A repository owned by the `nedap` GitHub organisation lists nothing, whatever
  its visibility or its entry says.
- A repository GitHub says is public (`gh api repos/<owner>/<repo> --jq
  .visibility`, asked through `gh` and cached for fifteen minutes in
  `$XDG_CACHE_HOME/model-gateway/repository-visibility.json`) lists every client
  model, the `"data_scope": "public"` ones among them, each allowed to send code
  and history; its allowlist entry is not read.
- A repository whose visibility cannot be read (no gh, offline, an API error, a
  remote not on github.com) counts as private, never as public.
- A client with `"data_scope": "public"` (`pi-free`, `pi-tier-review`) is never listed for a
  repository that is not public, whatever its entry says of the family.
- No `repo`, or a path outside any repository, lists only subsidised and
  metered-personal client models, each `inline_only` and allowed to send
  neither code nor history.

`anthropic` and `openai` are families like any other: a repository's entry must
list them for a delegation from a private repository to reach Claude Code or
Codex. A `nedap` repository may send nothing to any client.

A path git cannot read — one that does not exist, or a repository git refuses
for its ownership or permissions — is an error (`code:
"repository-unreadable"`) naming the path and git's message. It never falls
back to inline content.

## The machine override

The owner's own settings live outside this repository, in
`$XDG_CONFIG_HOME/agent-kit/model-gateway.json`, else
`~/.config/agent-kit/model-gateway.json`. A missing file means no overrides.

The file takes the same top-level keys as `defaults.json`, every one optional,
and merges over the shipped defaults:

- `clients.<id>.<field>` overrides that field, including
  `provider_cost_classes`: the cost class of each provider a client's models
  run on, by provider name (OpenCode's, whose models are listed as
  `<provider>/<model>`). A provider it does not name takes the client's own
  cost class. `providers` is the list of providers a run may reach through the
  client, by the name its own command line takes: the ones an OpenCode model
  may name (`provider-not-allowed` otherwise; `["opencode-go", "litellm-opencode-go"]` by default) and
  the ones a Hermes run may name (`hermes --provider`; empty by default, so no
  Hermes run is allowed until the owner lists one). `openrouter` stays refused
  whatever it says, on every client; free models run only as `pi-free`'s
  `free/<model>`.
- `clients.<id>.plan_limits` is what that client's plan allows, window by
  window: a list of `{"window": "5h"|"day"|"week"|"month", "limit_usd": n}`,
  or `{"window": ..., "limit_runs": n}` for a plan that is not billed in
  dollars. Each window reaches back from now, and its spend comes from the
  ledger. The list replaces the client's whole plan, so an override that names
  one window names them all. The shipped defaults carry none: the owner's
  plans are personal and live here. The owner's OpenCode Go plan, for example:

  ```json
  {
    "clients": {
      "opencode": {
        "plan_limits": [
          {"window": "5h", "limit_usd": 12},
          {"window": "week", "limit_usd": 30},
          {"window": "month", "limit_usd": 60}
        ]
      }
    }
  }
  ```

  At or above `plan_warning_ratio` of any window, `usage` and every `run`
  result for that client carry a warning; at 100% of any window the client is
  `unable_to_start` in `usage`.
- A client field the gateway does not know is an error naming it, rather than a
  setting quietly ignored: a client has `binary`, `cost_class`,
  `model_source`, `families`, `provider_cost_classes`, `providers` and
  `plan_limits`, and nothing else.
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
