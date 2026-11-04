---
name: model-gateway
description: Holds the model gateway's runner and its shipped defaults; it is not a workflow of its own, since the delegate skill, coming later, drives it.
---

# model-gateway

This folder is what the model gateway runs on. It does nothing on its own: an
agent calls the runner, and the `delegate` skill (a later ticket) is what
teaches an orchestrator to plan with it. Until then the runner answers one
question, `models`: which client models this machine can use right now.

## What is in here

- `model-gateway.mjs` — the runner. It is built from `council/ts` by
  `npm run build` and pinned in `manifest.yaml`'s `model_gateway.files`, the
  way the critique runner is pinned in `critique.files`. Run it with `node`.
- `defaults.json` — the shipped defaults: the clients on this machine, the
  client models each offers with their family, difficulty tier and capability
  notes, and the caps and thresholds a delegation is held to.
- `defaults.schema.json` — the JSON Schema `defaults.json` is validated
  against. The Python asset validators check the shipped file against it, so a
  broken default fails CI rather than a delegation.

The runner reads `defaults.json` from the folder it sits in, so the two always
travel together.

## The `models` command

`models` lists the client models on this machine that the data policy lets a
delegation from a repository reach. It prints exactly one JSON object on stdout
and exits 0; on an error it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
node model-gateway.mjs models --json '{"repo": "/path/to/repo"}'
node model-gateway.mjs models --input <file.json>
```

`repo` is the path the caller works in; the gateway asks git for that path's
repository root and its remote. The result carries one entry per permitted
client model with its client, model id, family, cost class, difficulty tier,
capability notes, source, `may_send` (`code` and `history`) and `inline_only`,
plus a `missing` list naming each client the gateway cannot use and why, a
`skipped` list naming each model it cannot offer and why, and a `policy` line
saying why the data policy listed what it did. A client counts as installed
when its binary is on `PATH`; one that is not is named in `missing` whatever
its model source. Models are sorted by client, then model id.

`source` says where the list came from:

- `live` — the client's own listing: Antigravity's `agy models` (a
  `Fetching available models...` line, then one `<id>\t<display name>` per
  model) and OpenCode's `opencode models` (one `<provider>/<model>` per line).
  Only the families a client may reach are offered, so Antigravity's Claude and
  GPT-OSS models are left out. An OpenCode model takes the cost class of its
  provider: the shipped defaults count OpenCode Go, the owner's subscription,
  as subsidised, and every other provider as metered-personal.
- `config` — the models Vibe's own configuration declares, listed by the alias
  `vibe --model` accepts. The gateway reads `$VIBE_HOME/config.toml`, else
  `~/.vibe/config.toml`.
- `declared` — the shipped catalogue, for Codex and Claude Code.

A listed model takes its family, tier and notes from the catalogue entry that
matches its id. A model the catalogue does not know is still listed, with its
family worked out from its id, `"tier": null` and no notes: the `delegate`
skill must not pick a null-tier model, because nobody has judged what it
handles well. A model whose family the gateway cannot work out is not offered
at all: it is named in `skipped` with the reason, so it never vanishes
silently, and adding it to the catalogue in `defaults.json` or the machine
override brings it back.

A listing that fails, or a Vibe configuration that cannot be read, leaves that
client out and names it in `missing` with the reason; the other clients are
still listed. Hermes lists nothing until its own ticket lands, and is not
reported missing for that.

Any other subcommand answers `{"status":"error","code":"unknown-command",...}`.

## The data policy

The family allowlist critique writes decides what a repository's content may
go to. It lives at `$XDG_CONFIG_HOME/agent-kit/critique-allowlist.json`, else
`~/.config/agent-kit/critique-allowlist.json`, and keys each repository by
`<host>/<owner>/<repo>` with `families`, `code` and `history`.

- A repository with an entry lists only the client models whose family the
  entry names, and each reports the entry's `code` and `history` in `may_send`.
- A repository with a remote but no entry, or with no remote, lists nothing and
  `policy` says why.
- A repository owned by the `nedap` GitHub organisation lists only Codex and
  Claude Code, whatever its entry says, with code and history allowed.
- No `repo`, or a path outside any repository, lists only subsidised and
  metered-personal client models, each `inline_only` and allowed to send
  neither code nor history.

`anthropic` and `openai` are families like any other: a repository's entry must
list them for a delegation to reach Claude Code or Codex. In a `nedap`
repository it is the client that counts, not the family: only the Codex and
Claude Code clients may receive anything.

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
  cost class.
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
