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

`models` lists every client model on this machine. It prints exactly one JSON
object on stdout and exits 0; on an error it prints
`{"status":"error","code":"<slug>","message":"<one line>"}` and exits 1.

```sh
node model-gateway.mjs models --json '{}'
node model-gateway.mjs models --input <file.json>
```

The input is `{}` today; later tickets add `repo`. The result carries one entry
per client model with its client, model id, family, cost class, difficulty tier,
capability notes and source, plus a `missing` list naming each client that is
not installed and why. A client counts as installed when its binary is on
`PATH`. Models are sorted by client, then model id.

`source` is `live`, `config` or `declared`. Today only `declared` is produced,
for Codex and Claude Code; Vibe and Antigravity listing comes later, and until
then those clients list no models and are not reported missing.

Any other subcommand answers `{"status":"error","code":"unknown-command",...}`.

## The machine override

The owner's own settings live outside this repository, in
`$XDG_CONFIG_HOME/agent-kit/model-gateway.json`, else
`~/.config/agent-kit/model-gateway.json`. A missing file means no overrides.

The file takes the same top-level keys as `defaults.json`, every one optional,
and merges over the shipped defaults:

- `clients.<id>.<field>` overrides that field.
- `models`: an entry matching `client` and `model` overrides its `tier`,
  `notes` or `family`; a new entry adds a declared model.
- `settings.<key>` overrides that setting, deep-merging
  `metered_personal_caps`.
- `disabled`: a list of `"<client>:<model>"` keys left out of `models`.

An unreadable or invalid override is an error (`code: "invalid-config"`) that
names the file. Never edit the shipped `defaults.json` to change this machine:
the next setup run replaces it.
