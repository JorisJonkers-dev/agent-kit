I'll investigate the repository to find how the second Claude profile is defined.The repo is at `/Users/j.w.jonkers/IDEAProjects/agent-kit`. Let me find grep/search capability since bash is denied.The bash denial may have been due to searching outside the repo. Let me retry with a grep restricted to the repository, as the task permits.Bash is fully denied by policy, but `read_file` can list directories. I'll navigate manually.Bash is unavailable, so I'll read the registry file directly in chunks to find `claude_profiles`.I found the `claude_profiles` block. Now let me pin exact line numbers for citations in both files.TWO IDENTITIES: the repo answers are all in hand — the second profile is `~/.claude-personal`, selected solely by `CLAUDE_CONFIG_DIR`.

## 1) SUMMARY

The estate defines two Claude profiles as two config roots: `~/.claude` (work, primary) and `~/.claude-personal` (personal), the second created with `CLAUDE_CONFIG_DIR=$HOME/.claude-personal` and everything shared (skills, hooks, plugins, `settings.json`) symlinked back to the primary; only login, history, `sessions/`, `projects/` and `.claude.json` differ. Gold Band neither knows nor needs a profile concept: its spawned adapter passes every `adapter.env` key to the child, so the personal identity is one new custom agent `claude-personal` sharing the claude-acp command with `env.CLAUDE_CONFIG_DIR` set. `useLocalClaude` picks the *binary*, not the profile.

## 2) RECOMMENDATION

**Target path: `~/.gold-band/settings.json`** — the `agents` section, two entries:

```jsonc
"agents": {
  // Identity 1 — work. Keep the BUILT-IN id `claude-acp`: its command/args are
  // catalog-owned (stripped on save, re-injected on load), so omit them here.
  "claude-acp": {
    "adapter": {
      "displayName": "Claude (work)",
      // Set explicitly so an ambient CLAUDE_CONFIG_DIR (e.g. Gold Band launched
      // from a claude-personal shell) can never hijack the work identity.
      "env": { "CLAUDE_CONFIG_DIR": "/Users/j.w.jonkers/.claude" }
    },
    "primaryAgentDir": "/Users/j.w.jonkers/.claude",
    "compatibleAgentDirs": [],
    "systemPromptDelivery": "metaAppend",
    "externalSessionSyncSupported": true,
    "externalSessionSyncEnabled": false
  },

  // Identity 2 — personal. NEW custom id (lowercase + dashes, valid), so it
  // is user-owned: copy command/args verbatim from the built-in claude-acp
  // entry (shown in Gold Band's agent settings / resources/agent-catalog.json,
  // pinned 0.87.0 per excerpt catalog-policy.json).
  "claude-personal": {
    "adapter": {
      "command": "<claude-acp command, exactly as the built-in claude-acp entry shows>",
      "args": ["<same args as the built-in claude-acp entry>"],
      "displayName": "Claude (personal)",
      "env": { "CLAUDE_CONFIG_DIR": "/Users/j.w.jonkers/.claude-personal" }
    },
    "primaryAgentDir": "/Users/j.w.jonkers/.claude-personal",
    "compatibleAgentDirs": [],
    "systemPromptDelivery": "metaAppend",
    "externalSessionSyncSupported": true,
    "externalSessionSyncEnabled": false
  }
}
```

Leave `useLocalClaude` on/default: it resolves `claude` from PATH (→ `~/.local/bin/claude`) and sets `CLAUDE_CODE_EXECUTABLE` for each adapter — the *same* binary for both; do **not** hand-set `CLAUDE_CODE_EXECUTABLE` in `env` (an env value suppresses Gold Band's own resolution). Target any Gold Band skills at `claude-acp` only: `~/.claude-personal/skills` is a symlink into `~/.claude/skills`, so both entries reconcile to the same physical tree.

**Identity test** (DIRECT session per agent, in an empty scratch dir, e.g. `/tmp/claude-identity-test`): prompt:
> Run `echo "CONFIG=$CLAUDE_CONFIG_DIR"` and report the output verbatim. Then report the newest entry under each of `~/.claude/projects` and `~/.claude-personal/projects`.

Expected: `claude-acp` answers `CONFIG=/Users/j.w.jonkers/.claude`; `claude-personal` answers `CONFIG=/Users/j.w.jonkers/.claude-personal`. Afterwards, a new encoded `<scratch-dir>` entry exists under the matching `projects/` root **only** — the other root is unchanged (that is exactly the separation SETP.md says `CLAUDE_CONFIG_DIR` buys: docs/SETUP.md:443-446).

## 3) FINDINGS

1. `CLAUDE_CONFIG_DIR` moves the whole Claude root (credentials, `projects/`, `history.jsonl`, `sessions/`, `.claude.json`) | registry/estate-tooling.yaml:126-128; docs/SETUP.md:443-445 | one binary + two config dirs = two independent identities; identity is decided by this one variable. H
2. Gold Band sets every `adapter.env` key on the spawned child (`for (key, value) in &resolved_env { command.env(key, value); }`) and has no CLAUDE_CONFIG_DIR handling of its own | excerpt adapter-spawn-env.rs (src/acp/adapter.rs ~120-345) | `adapter.env.CLAUDE_CONFIG_DIR` is the profile selector for ACP sessions. H
3. Executable selection = `CLAUDE_CODE_EXECUTABLE` + `use_local_claude`: Gold Band resolves `claude` from the adapter PATH and injects it; an existing `CLAUDE_CODE_EXECUTABLE` in env wins; `REQUIRE_LOCAL_CLAUDE_ENV`/`require_local_claude_executable` make unresolvable claude a hard error | excerpt adapter-spawn-env.rs | `useLocalClaude` is orthogonal to profiles — leave default, same `~/.local/bin/claude` for both. H
4. Each agent entry owns its `adapter` (command, args, displayName, env) and is spawned separately | excerpt agent-config.rs (src/config/mod.rs:495-640) | two agents may share the claude-acp command with different `env`. H
5. Built-in ids' command/args are catalog-owned (stripped on save, re-injected on load); ids are lowercase ASCII+digits+`-`; any ACP command can be a custom agent with command/args/env | ref §Agents, §Storage | `claude-personal` must be a **new** id — you cannot get a second profile by re-editing `claude-acp`; `claude-personal` is a valid id. H (per the reference)
6. `~/.claude-personal` exists with `skills, agents, commands, hooks, plugins, settings.json` symlinked to the primary; work root is `~/.claude` (`claude-personal` launcher only sets `CLAUDE_CONFIG_DIR` and execs `claude`) | registry/estate-tooling.yaml:152-165; docs/SETUP.md:454-456, 469-477 | hooks (critique + plugin hooks, incl. primary-script absolute paths) and skills load identically in both identities; `primaryAgentDir: ~/.claude-personal` is safe — Gold Band's skill symlinks land in the shared physical tree. H
7. Claude's MCP servers live per profile in each root's own `.claude.json`, registered once per profile by setup | registry/estate-tooling.yaml:144-146; docs/SETUP.md:461-464 | each identity already carries the full fleet; do not re-register token-bearing servers in Gold Band `contextServers` (settings.json is plaintext — per task rules). H
8. The spawned child may inherit Gold Band's ambient environment beyond the explicit env map | **unverified** (excerpt shows only explicit `command.env` inserts) | mitigated by setting `CLAUDE_CONFIG_DIR` explicitly on **both** entries. M
9. Project `CLAUDE.md` (cwd) loads for both; user-scope memory lives under the config root (not in `shared_paths`), so it is per-identity | docs/SETUP.md:454-456 for what is shared; Claude Code memory semantics **unverified** | expect identical project context, divergent user memory. L
10. Two simultaneous logins depend on keychain entries being namespaced per config root | docs/SETUP.md:491-498 | run the keychain check before using both at once. H

## 4) VERIFY LIST

1. Exact claude-acp `command`/`args` and the settings.json record shape (`agents` key nesting, `systemPromptDelivery` enum value, `primaryAgentDir` absolute vs relative) — **verify in diodeme/Gold-Band (src/config/mod.rs, resources/agent-catalog.json)**. Test: both agents pass Gold Band's per-agent "Doctor" and report claude-acp pin 0.87.0.
2. Ambient-env inheritance — launch Gold Band from a shell with `CLAUDE_CONFIG_DIR=$HOME/.claude-personal claude-personal` style env set, then run the identity prompt above in `claude-acp`. Expected: `/Users/j.w.jonkers/.claude` (explicit entry wins).
3. `useLocalClaude` setting name/default and that the adapter honors `CLAUDE_CODE_EXECUTABLE` — check the built-in prompts/`claude --version` vs `Doctor` output for both agents; expected: identical Claude Code version.
4. macOS keychain namespacing: `security dump-keychain | grep -c '"svce"<blob>="Claude Code-credentials"'` (docs/SETUP.md:495) — expect **2** entries.
5. The identity test in §2 (prompt + new-`projects/`-entry-only-under-the-matching-root).

## 5) DECISIONS FOR THE OWNER

1. **Reuse built-in `claude-acp` for work instead of duplicating it as a custom `claude` id?** Recommended answer: yes — the catalog keeps the version pin and command correct; you only add `env`, `primaryAgentDir`, `displayName`. A custom clone would freeze command/args under your ownership.
2. **`compatibleAgentDirs`: empty for both?** Recommended: yes — never let the work entry read `~/.claude-personal`'s root (history/credentials) or vice versa.
3. **Gold Band skills targeted at which Claude agents?** Recommended: `claude-acp` only; `claude-personal` resolves to the same physical skills dir through the symlink, so targeting both is redundant but harmless.
4. **Make `claude-personal`'s MCP come from Gold Band `contextServers` too?** Recommended: no for token-bearing servers (plaintext rule); each profile's `.claude.json` already carries the fleet.
5. **Enable `externalSessionSync` on either identity?** Recommended: off for `claude-personal` (keeps personal history out of shared-client sync); off on work too unless you later want Gold Band to import Claude app sessions — confirm semantics in diodeme/Gold-Band (src/config/mod.rs): does external session sync for the two config roots collide on one keychain identity?

SUMMARY: agent-kit defines `claude-personal` entirely via `CLAUDE_CONFIG_DIR=$HOME/.claude-personal` (registry/estate-tooling.yaml:152-165, docs/SETUP.md:441-478): a second config root holding only its own login, projects/, sessions/ and .claude.json, with skills, agents, commands, hooks, plugins and settings.json symlinked back to the primary ~/.claude. Gold Band's spawn code passes every adapter.env key to the child ACP process and only manages the *executable* (useLocalClaude → CLAUDE_CODE_EXECUTABLE from PATH, overridable via env), so the profile is selected purely by env. Recommendation: keep the built-in claude-acp entry for work (add env.CLAUDE_CONFIG_DIR, primaryAgentDir ~/.claude, empty compatibleAgentDirs) and add one new custom agent id claude-personal with the same claude-acp command/args (copied from the catalog, pinned 0.87.0) and env.CLAUDE_CONFIG_DIR=/Users/j.w.jonkers/.claude-personal, primaryAgentDir ~/.claude-personal. Both share one binary (~/.local/bin/claude) and, via the symlink, identical skills and hooks; histories and credentials stay isolated, subject to the macOS keychain namespacing check (docs/SETUP.md:495, expect two entries). Identity proof: prompt each agent to echo $CLAUDE_CONFIG_DIR in a scratch dir and confirm a new projects/ entry appears only under ~/.claude-personal for the personal agent and ~/.claude for work.