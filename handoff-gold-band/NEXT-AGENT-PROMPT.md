# Prompt for the next agent: full Gold Band adoption

Paste everything below the line into a fresh agent session started in this repository on branch `feat/gold-band-epic` (pushed to origin). Last updated 2026-11-09.

---

You continue the Gold Band adoption for agent-kit (JorisJonkers-dev/agent-kit). The epic is #368. Read it, then its specs #327, #350 and #367, then `handoff-gold-band/HANDOFF.md`.

The aim: agents, roles, skills and MCP servers are declared in one registry and converged by setup; the delegation workflows run headless on Gold Band; and every existing skill is ported to a Gold Band target or recorded as staying outside.

## Owner rules (hard)

- **Work profile:** critique in the work Claude profile (`~/.claude`) may use ONLY Codex or Claude (work). It never uses claude-personal or any other agent.
  - `~/.claude-personal/settings.json` and `~/.claude-personal/skills` are symlinks into `~/.claude`, so whatever is installed for personal is live in work sessions.
  - Commit `a627cc8` makes the three critique hooks no-op when the payload's `transcript_path` is under `~/.claude/`. It is installed only once the owner copies the hooks: `cp -p skills/workstation/critique/hooks/critique-*.sh ~/.claude/skills/critique/hooks/`. The classifier blocked the agent from doing it.
  - Open follow-ups, each needing the owner's answer:
    - a runner-level guard: a work-profile `/review-*` run may route only to Codex or Claude;
    - unsharing the two profiles' `settings.json`;
    - whether the rule covers Gold Band workflows that mix `claude-acp` with other agents.
- **No Codex** in any Gold Band workflow or provider list.
- **Gold Band** may use every OpenRouter and OpenCode model, free ones included; budgets guard the paid keys. The model gateway's rules are unchanged.
- **Opus** only in judgment nodes, with the `-planning` suffix rule (see #371).
- **New code is Rust** under the 7-day crate cooldown. crate-gate runs at 127.0.0.1:17342; the only allowed dependencies are serde =1.0.219 and serde_json =1.0.140.
- **Delegation:** non-Claude first.
  - Vibe `glm-5-3` for writing; DeepSeek, Kimi or Qwen for reviews.
  - Never above Sonnet at medium effort, and `"escalate": false` on every gateway call.
  - Antigravity only rarely, because of its low weekly quota.
- **Two reviewers per change**, from families other than the implementer's: for example the critique runner plus one OpenCode-Go model.
- **Never bypass hooks.** Run `critique.mjs record` in a separate command from `git commit`.

## State (feat/gold-band-epic)

Landed and reviewed: #331, #332, #333, #334, #335, #336, #337, #338, #339, #345 (crate and setup wiring), #347, #348, #351, #352 (crate), #355, #356, #369, #370, #371, #374. Also landed:
- `a627cc8`: the work-profile hook guard;
- `9fc2af0`: setup moves a stray cask binary aside, so the `agy` cask install works;
- `a1f4340`: the Claude adapter is verified to pass `CLAUDE_CONFIG_DIR` on and to load its hooks.

Build trackers are one artifact per spec; the URLs are in memory under `gold-band-build-trackers`. `handoff-gold-band/tools/build-report.sh` refreshes them.

Notes on the last commits:
- **#339** was built before #335. Resolving it on the epic added `agent: claude` to both Claude entries, and `skillSet: personal` to `claude-personal`, so the two identities stay distinct under the one-id rule.
- **#374 and #345** were reviewed by the critique runner (Mistral). Their Kimi reviews were cut off by a network reset. Re-run a second-family review if you want two.

Real setup ran from this worktree, run with `env -u CLAUDE_CONFIG_DIR`. Gold Band holds 7 agents (`settings.json` 0600). Outstanding from that run:
- Pi installs once its pinned 1.5.0 passes the cooldown (2026-11-09 11:05 UTC).
- `GRIMOIRE_API_TOKEN` is blocked by Grimoire's forward auth; JorisJonkers-dev/grimoire #401 covers it.
- The n8n sign-in does not stick. It is probably the same forward auth; ask the owner.
- The kubernetes-skill plugin had to be reinstalled by hand because `claude plugin update` would not move it to its pin. Setup should fall back to uninstall and install.

## Frontier

- **Cut Waves 3–9 into tickets.**
  - `docs/gold-band-port-inventory.md` "## Waves" holds the ordered rows, and "## Owner decisions" holds the eight decisions.
  - Show the owner the breakdown first; roughly 40–60 tickets.
  - A `(loop)` row's ticket must carry its unrolled node sequence.
- **#329:** MCP overlap. It can be run headlessly; the owner asked the agent to do it, not them.
- **#330:** settle the unverified reach cells by probing the binaries. Update `gold_band.reach_support` in a reviewed commit that cites the evidence.
- **#340–#344:** per-agent reach levels and the OS write confinement. #344 also needs #328's remaining UI details, which are minor.
- **#349, #353, #357–#365, #372, #373** in dependency order.
- **#354** needs the five API keys. The owner runs `handoff-gold-band/tools/owner/api-keys.sh`, which stores them in the macOS keychain as `agent-kit.<VAR>`: `MISTRAL_API_KEY`, `MISTRAL_STUDY_API_KEY`, `OPENCODE_GO_API_KEY`, `OPENROUTER_API_KEY`, `GEMINI_API_KEY`. The owner answered #354's free-models question: Pi may use free OpenRouter and OpenCode models.
- **#379:** setup under an ambient `CLAUDE_CONFIG_DIR`. The owner's default: always install the work profile into `~/.claude`.
- **Open PRs.** `gh pr create` is gated on a passing `/review-pr` for the head. Run the feature review first.

## Facts learned (keep)

- **Gold Band headless** (v0.19.1, 987fc3e). The binary is at `~/IDEAProjects/gold-band-src/target/release/gold-band`; see `docs/gold-band-headless-build.md`.
  - `settings.json` needs `settingsSchemaVersion` (12), or custom agents are silently dropped.
  - Output schemas use Gold Band's simplified shape.
  - JSON artifacts get `.json` appended.
  - Profiles live in `~/.gold-band/context/profiles`.
  - A model binding needs the agent to offer that model in `session/new` `configOptions`.
  - A worker sees every earlier node of its round as predecessor context.
- **Vibe sessions** can crash with "session history too large". The answer is then still in `<output-dir>/session/chunks`.
- **Vibe GLM** sometimes cannot run commands and leaks drafting text into files. Always re-run the checks yourself and grep for `<arg_value>` or `</think>`.
- **Monitors run under zsh:** plain `for x in $list` does not split, so use arrays or files.
