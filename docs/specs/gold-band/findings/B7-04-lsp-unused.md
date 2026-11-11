# B7-04: why the language-server tool goes unused

Ticket #519, spec #392 story 153. The record lists each candidate cause with
the evidence for it and the change it predicts would raise the tool's use.
It holds counts, session ids and configuration keys only; no transcript
content.

## Method

- `transcript-mix` (B7-01) over this workstation's Claude Code transcripts,
  window `--since 2026-01-01`: 287 task transcripts (top-level sessions and
  their sub-agent runs), 48 of which reached an edit, 7,238,440 exploration
  tokens.
- Direct counts over the 14 top-level session files under
  `~/.claude/projects` (tool-use records by tool name, and whether the
  language-server tool's name appears in the deferred-tool listing).
- The installed configuration: `enabledPlugins` in `~/.claude/settings.json`,
  the `lspServers` entries of the `agent-kit-official` marketplace, and
  `command -v` for every server binary.

Limit: only the `claude` agent has transcripts on this machine. No Codex,
Vibe or OpenCode transcript was available, so those agents are covered by
configuration only (cause 5).

## What the tool mix shows

Across the 287 tasks: language-server tool calls 0; `Read` 136; `Bash`
calls classed as search 108; `Grep` 1; knowledge-page tools 5. Across the 14
top-level sessions the language-server tool was called 0 times, `ToolSearch`
was called in 6 of them, and no `ToolSearch` query named the language-server
tool, language servers, definitions or references.

## Causes

Wiring is not a cause. All 12 `*-lsp` plugins that carry `lspServers` are
`true` under `enabledPlugins` (`liquid-lsp`, the thirteenth, carries none, see cause 6), and
`command -v` finds the binary for each of those 12 servers. The tool is
listed to the agent in every session; it is just never chosen.

### 1. The tool is deferred: agents see its name, not its schema

Evidence: in all 14 top-level sessions the tool name `LSP` appears only in the
deferred-tool listing; its parameters are not loaded until `ToolSearch`
selects it. No session selected it (0 of 14). Session ids:
23a4abad-0a8e-4903-b01a-59fd677ea36e, 4567a865-05e2-4ae7-9f54-aed8b306c64c,
5ffa4915-ed23-4da7-bc4d-dc2c5b96e07f, f6d964c3-b5dc-41b6-a42c-7bf5338304f8,
10b0a545-0882-49a9-bfee-645e89fb9be5, 5d394416-4853-438d-b632-8b9ff13197e3,
d4484d7f-07d9-4ae6-ad51-5a0be5a8abe1, 45ff56cf-9b4f-40b0-9144-6ba46486e365,
b7aa7607-6705-4d60-aec4-1a83050ca35c, ea2e41ad-0411-4ff1-8eb8-182c23311c17,
11d434fa-0043-4c40-8497-7409c6b93744, a3bb0ae9-0c21-4af0-9a72-b5eea4e8d37f,
7ffcf7a5-9d2e-4a33-8e2c-1e43944e70ae, dd3cf630-9812-43ca-8c53-585f6fb0aa13.
Configuration key: none; the deferral is the harness default and
`~/.claude/settings.json` has no `env` block that changes it.

Predicted change: put the tool's one-line purpose and its call shape in
always-loaded instructions, or make the tool non-deferred where the harness
allows it. Prediction: the first `LSP` call appears in sessions that
navigate code, where today it is 0 of 14.

### 2. Nothing in the instructions says when to use it

Evidence: `~/.claude/CLAUDE.md`, the repository `CLAUDE.md` and the estate
`CLAUDE.md` do not mention the tool. The only guidance in the repository is
install documentation (`docs/SETUP.md`, `registry/estate-tooling.yaml`
`language_servers`), which tells a person how to install a server, not an
agent when to call it. Every session in the sample navigated with shell
search and reads instead.

Predicted change: a short rule ("for definitions, references and callers of a
symbol use the language-server tool before searching text") in the estate
`CLAUDE.md`. Prediction: share of code-finding calls that are the
language-server tool rises from 0 of 245 to a measurable share on the next
`transcript-mix` run.

### 3. Text search is cheaper to start and already habitual

Evidence: code finding runs through `Bash` search (108 calls) and `Read`
(136) in the 287 tasks; `Grep` is almost unused because `Bash` search is
chosen first. Sessions with the highest counts: 5d394416 (1019 `Bash`
calls), 45ff56cf (406), 4567a865 (392), 23a4abad (328). A text search needs
no file path, line or column; the language-server tool needs a position
inside a file.

Predicted change: the code-navigation server of B7-05 onward, which takes
a symbol name, not a position. Prediction: exploration tokens per edited
task (7,238,440 over 48 edited tasks in this sample) fall once symbol-name
navigation exists. A tool that needs a position will keep losing to
`grep`.

### 4. Many tasks never touch code the servers cover

Evidence: 239 of the 287 tasks ended without an edit, so many tasks may not
have needed symbol navigation at all. The unused-tool rate is therefore
partly a denominator problem; the sample cannot say how much.

Predicted change: measure the tool's use only over tasks that edit source
files in a language with a configured server. Prediction: the 0 of 245
figure stays 0 under that filter if causes 1 and 2 are the real blockers;
if it rises, this cause explains part of the gap.

### 5. Other agents have no such tool

Evidence: the wiring is `*-lsp` Claude Code plugins (`enabledPlugins`
keys `typescript-lsp@agent-kit-official` and the twelve others). Codex, Vibe
and OpenCode have no equivalent in this configuration, so for them the tool
cannot be used at all. `registry/estate-tooling.yaml` lists `serena` (an
LSP-backed MCP server) for the container surface only, install null on the
workstation.

Predicted change: expose navigation to every agent through the MCP server
of B7-05, which is listed for every reach level, instead of per-harness
plugins. Prediction: non-Claude agents show non-zero navigation calls once
it is listed for them. Not verified: no transcripts from those agents were
available.

### 6. Minor wiring gaps

Evidence: `liquid-lsp` has no `lspServers` entry in the marketplace, so it
registers nothing; the registry marks its language enabled. This does not
explain the 0 of 14 (no Liquid code in the sample) but it is a second
instance of "plugin enabled, tool absent", which the registry comment
already warns looks the same from inside a session.

Predicted change: drop or fix `liquid-lsp` in `language_servers`.
Prediction: none for use; it removes a false positive from future audits.

## Ranking

Causes 1 and 2 are the ones the data supports directly (0 of 14 selections,
no instruction). Cause 3 explains why fixing them alone may not be enough.
Cause 4 should be controlled for when the next measurement is taken. Cause 5
needs transcripts from the other agents before it is confirmed.

## Re-measuring

```bash
cargo run --manifest-path scripts/Cargo.toml -p transcript-mix -- \
    --root <DIR> --since <date>
```

with `<DIR>/<agent>` pointing at each agent's transcripts. A cause is
confirmed when its predicted change moves the language-server line of that
report.
