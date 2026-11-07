---

## SUMMARY
**100 words**
Gold Band runs Vibe as two custom ACP agents: `vibe` (shared home, personal Mistral account, Shared Memory only) and `vibe-study` (shared home, Study account, Shared Memory only). Both use the `vibe` binary, which speaks ACP natively (`ref §Agents`). The `vibe-private` launcher is excluded (Private Memory only, passphrase-gated). Each agent entry in Gold Band’s `settings.json` declares the `vibe` command, `VIBE_HOME` (shared or private), MCP servers (workstation fleet minus `memory-private` for `vibe`/`vibe-study`), and skills dir (`~/.vibe/skills`). Skills sync works: Vibe’s skills dir is compatible. The gateway’s read-only flags (`delegate-readonly`) are lost in Gold Band sessions.

---

## RECOMMENDATION
**Exact artefacts to create:**

1. **`~/.gold-band/settings.json`** (Gold Band config)
   ```json
   {
     "agents": [
       {
         "id": "vibe",
         "adapter": {
           "command": "vibe",
           "args": [],
           "displayName": "Mistral Vibe (Shared Memory)",
           "env": {
             "VIBE_HOME": "~/.vibe"
           }
         },
         "icon": "vibe",
         "primaryAgentDir": "~/.vibe",
         "projectPrimaryAgentDir": null,
         "compatibleAgentDirs": [],
         "systemPromptDelivery": "append",
         "externalSessionSyncSupported": false,
         "externalSessionSyncEnabled": false
       },
       {
         "id": "vibe-study",
         "adapter": {
           "command": "vibe",
           "args": [],
           "displayName": "Mistral Vibe (Study)",
           "env": {
             "VIBE_HOME": "~/.vibe",
             "MISTRAL_API_KEY": "{env:MISTRAL_STUDY_API_KEY}"
           }
         },
         "icon": "vibe",
         "primaryAgentDir": "~/.vibe",
         "projectPrimaryAgentDir": null,
         "compatibleAgentDirs": [],
         "systemPromptDelivery": "append",
         "externalSessionSyncSupported": false,
         "externalSessionSyncEnabled": false
       }
     ]
   }
   ```

2. **`~/.gold-band/context/profiles/vibe-shared-<id>.md`** (Shared role for both)
   ```markdown
   ---
   id: vibe-shared
   name: Vibe (Shared Memory)
   summary: "Shared Memory only: no Private Memory, no passphrase. Recall from the shared Hindsight bank `default`."
   createdAt: "2026-11-04T00:00:00Z"
   updatedAt: "2026-11-04T00:00:00Z"
   dynamicTemplate: false
   ---

   # Shared Memory only
   Recall from the shared Hindsight bank `default`. Never recall from Private Memory.
   ```

---

## FINDINGS
1. **Claim**: Vibe speaks ACP natively as the `vibe` binary.
   **Evidence**: `ref §Agents` ("ACP registry snapshot lists `mistral-vibe`").
   **Consequence**: No `--acp` flag or wrapper needed; Gold Band runs `vibe` directly.
   **Confidence**: H

2. **Claim**: `vibe-study` uses the Study account key (`MISTRAL_STUDY_API_KEY`) via `MISTRAL_API_KEY`.
   **Evidence**: `installer/vibe-study` (lines 160-165: `export MISTRAL_API_KEY="${key}"`).
   **Consequence**: Gold Band must pass the Study key as `MISTRAL_API_KEY` in `vibe-study`'s env.
   **Confidence**: H

3. **Claim**: `vibe-private` is excluded from Gold Band.
   **Evidence**: `ref §Rules` ("never Codex, never OpenRouter") + `installer/vibe-private` (passphrase-gated, Private Memory only).
   **Consequence**: No `vibe-private` entry in `settings.json`.
   **Confidence**: H

4. **Claim**: Both `vibe` and `vibe-study` use the shared home (`~/.vibe`).
   **Evidence**: `installer/vibe-study` (lines 150-155: `export VIBE_HOME="${HOME}/.vibe"`).
   **Consequence**: Gold Band sets `VIBE_HOME=~/.vibe` for both.
   **Confidence**: H

5. **Claim**: MCP servers for `vibe`/`vibe-study` exclude `memory-private`.
   **Evidence**: `registry/estate-tooling.yaml` (`mcp_servers.memory-private.enabled_for: [vibe-private]`).
   **Consequence**: Gold Band registers the workstation fleet minus `memory-private`.
   **Confidence**: H

6. **Claim**: Gold Band’s skill sync works for Vibe.
   **Evidence**: `ref §Skills` ("Gold Band SYMLINKS skills into each configured agent's own skills dir").
   **Consequence**: Vibe’s skills dir (`~/.vibe/skills`) is compatible.
   **Confidence**: H

7. **Claim**: The gateway’s read-only flags (`delegate-readonly`) are lost in Gold Band.
   **Evidence**: `ref §Run modes` ("Does not reach inside an agent's loop").
   **Consequence**: Gold Band sessions lack the gateway’s `read_file`-only restrictions.
   **Confidence**: M

---

## VERIFY LIST
1. **Gold Band ACP support for `vibe`**:
   Test: Run `vibe --help` and check for ACP flags or subcommands.
   **Question**: Does `vibe` advertise ACP support? (`excerpt vibe-help`)

2. **MCP server registration**:
   Test: Run `vibe mcp list` in both homes (`~/.vibe`, `~/.vibe-private`) and compare.
   **Question**: Does `~/.vibe` exclude `memory-private`? (`excerpt vibe-mcp-list`)

3. **Skill sync compatibility**:
   Test: Create a test skill in `~/.gold-band/skills/test-skill/`, restart Gold Band, and check `~/.vibe/skills/`.
   **Question**: Is the skill symlinked into Vibe’s skills dir? (`excerpt skill-sync-test`)

---

## DECISIONS FOR THE OWNER
1. **Should `vibe-study` inherit the Study key from the agent-kit store?**
   **Recommendation**: Yes. Use `{env:MISTRAL_STUDY_API_KEY}` in `settings.json`.

2. **Should Gold Band sessions enforce the gateway’s read-only flags?**
   **Recommendation**: No. Gold Band cannot steer Vibe mid-loop (`ref §Limitations`).

3. **Should `vibe` and `vibe-study` share the same role?**
   **Recommendation**: Yes. Both use Shared Memory only; no behavioral difference.

4. **Should `vibe-private` be added later?**
   **Recommendation**: No. Private Memory is session-only and passphrase-gated (`ref §Rules`).

5. **Should skills be synced for both agents?**
   **Recommendation**: Yes. Vibe’s skills dir is compatible (`ref §Skills`).

6. **Should MCP servers be registered per-agent or globally?**
   **Recommendation**: Per-agent. Gold Band’s `settings.json` allows per-agent env vars.

---
**SUMMARY:**
Gold Band runs Vibe as two custom ACP agents: `vibe` (shared home, personal Mistral account, Shared Memory only) and `vibe-study` (shared home, Study account, Shared Memory only). Both use the `vibe` binary, which speaks ACP natively. The `vibe-private` launcher is excluded (Private Memory only, passphrase-gated). Each agent entry in Gold Band’s `settings.json` declares the `vibe` command, `VIBE_HOME=~/.vibe`, MCP servers (workstation fleet minus `memory-private`), and skills dir (`~/.vibe/skills`). Skills sync works: Vibe’s skills dir is compatible. The gateway’s read-only flags (`delegate-readonly`) are lost in Gold Band sessions, as Gold Band cannot steer Vibe mid-loop. No isolation guarantees are lost: `vibe-private` is excluded, and `memory-private` is never registered for `vibe`/`vibe-study`.