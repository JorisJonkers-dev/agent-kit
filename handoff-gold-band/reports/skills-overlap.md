---
**SUMMARY** (100 words)
Gold Band discovers skills from **three sources**: global (`~/.gold-band/skills/`), project (`<repo>/.gold-band/skills/`), and per-agent directories (primary and compatible). It **reads** `.claude/skills` as a *compatible* directory (never writes to it). Gold Band **symlinks** skills into an agent’s `primary_agent_dir` (e.g., `.claude/skills`) only if they are missing or conflict-free, but **never duplicates** existing skills. The owner’s Claude skills appear because Gold Band reads `.claude/skills` as a compatible dir. **Recommendation**: Move all skills to `~/.gold-band/skills/` as the single source, set `compatibleAgentDirs: []` for all agents, and let Gold Band symlink them. This avoids conflicts and duplication.

---

### **RECOMMENDATION**
#### **1. Move all skills to `~/.gold-band/skills/`**
Create the directory and move:
- Workstation skills: `agent-kit/skills/workstation/*` → `~/.gold-band/skills/`
- Repo skills: `agent-kit/skills/repo/*` → `~/.gold-band/skills/`
- Third-party skills (e.g., `mattpocock-skills`, `superpowers`) → `~/.gold-band/skills/`

#### **2. Update `settings.json` for each agent**
Set `compatibleAgentDirs: []` and ensure `primaryAgentDir` points to the agent’s skills directory (e.g., `.claude/skills`). Example for `claude`:
```json
{
  "adapter": { "command": "claude-acp", "args": ["--agent-dir", ".claude"] },
  "icon": "claude",
  "primaryAgentDir": ".claude/skills",
  "projectPrimaryAgentDir": null,
  "compatibleAgentDirs": [],
  "systemPromptDelivery": "metaAppend"
}
```
**Path**: `~/.gold-band/settings.json`

#### **3. Test**
Run Gold Band and verify:
1. No duplicate skills appear.
2. All skills are accessible in DIRECT/WORKFLOW/AUTO modes.
3. No symlinks are created in `.claude/skills` (or other agents’ dirs) unless a skill is missing.

---

### **FINDINGS**
1. **Claim**: Gold Band discovers skills from global (`~/.gold-band/skills/`), project (`<repo>/.gold-band/skills/`), and per-agent directories (primary and compatible).
   **Evidence**: `skill/mod.rs` lines 120-180 (`configured_agent_skill_read_dirs`, `scan_skills_dir`).
   **Consequence**: Skills can be sourced from multiple locations; conflicts are possible.
   **Confidence**: **H**

2. **Claim**: Gold Band reads `.claude/skills` as a *compatible* directory (read-only, never writes).
   **Evidence**: `agent-config.rs` lines 100-120 (`compatible_agent_dirs` in `ManagedAgentConfig`).
   **Consequence**: The owner’s Claude skills appear in Gold Band because `.claude/skills` is listed in `compatibleAgentDirs`.
   **Confidence**: **H**

3. **Claim**: Gold Band **symlinks** skills into an agent’s `primaryAgentDir` only if they are missing or conflict-free.
   **Evidence**: `skill/mod.rs` lines 60-90 (`SyncConflict` error, `skill_directory_policy`).
   **Consequence**: No duplication occurs unless a skill is missing in `primaryAgentDir`.
   **Confidence**: **H**

4. **Claim**: Gold Band **never writes** to compatible directories (e.g., `.claude/skills`).
   **Evidence**: `agent-config.rs` lines 100-120 (`compatible_agent_dirs` is read-only).
   **Consequence**: Moving skills to `~/.gold-band/skills/` is safe and avoids conflicts.
   **Confidence**: **H**

5. **Claim**: Workstation skills are in `agent-kit/skills/workstation/`, repo skills in `agent-kit/skills/repo/`.
   **Evidence**: Excerpt `scripts/render_agent_kit.py` (unverified, but referenced in CONTEXT.md).
   **Consequence**: These must be moved to `~/.gold-band/skills/` to avoid duplication.
   **Confidence**: **M**

6. **Claim**: Third-party skills (e.g., `mattpocock-skills`) are installed in agent-specific dirs (e.g., `.claude/skills`).
   **Evidence**: CONTEXT.md (unverified).
   **Consequence**: Move them to `~/.gold-band/skills/` to centralize management.
   **Confidence**: **M**

7. **Claim**: Name conflicts across sources trigger a `SyncConflict` error.
   **Evidence**: `skill/mod.rs` lines 60-70 (`SyncConflict` error).
   **Consequence**: Avoid duplicate skill names across sources.
   **Confidence**: **H**

---

### **VERIFY LIST**
1. **Fact**: Does Gold Band **symlink** skills into `primaryAgentDir` if they are missing?
   **Test**: Delete a skill from `.claude/skills/`, restart Gold Band, and check if it reappears as a symlink.

2. **Fact**: Does Gold Band **ignore** skills in `compatibleAgentDirs` if they already exist in `primaryAgentDir`?
   **Test**: Add a skill to both `.claude/skills/` and `~/.gold-band/skills/`, restart Gold Band, and check for conflicts.

3. **Fact**: Are third-party skills (e.g., `mattpocock-skills`) installed in agent-specific dirs?
   **Test**: Check `.claude/skills/`, `.vibe/skills/`, etc., for third-party skill folders.

4. **Fact**: Does Gold Band **rewrite** `workflows.json` on load?
   **Test**: Edit `~/.gold-band/context/workflows.json`, restart Gold Band, and check if changes persist.

5. **Fact**: Are workstation/repo skills in `agent-kit/skills/workstation/` and `agent-kit/skills/repo/`?
   **Test**: Run `ls agent-kit/skills/workstation/` and `ls agent-kit/skills/repo/` (or equivalent).

---

### **DECISIONS FOR THE OWNER**
1. **Should workstation skills (e.g., `vibe:worktree`) be moved to `~/.gold-band/skills/`?**
   **Recommendation**: **Yes**. Centralize all skills to avoid duplication and conflicts.

2. **Should repo skills (e.g., `vibe:skill-creator`) be moved to `~/.gold-band/skills/`?**
   **Recommendation**: **Yes**. Repo skills are project-agnostic and should be globally available.

3. **Should third-party skills (e.g., `mattpocock-skills`) be moved to `~/.gold-band/skills/`?**
   **Recommendation**: **Yes**. Avoid scattering them across agent dirs.

4. **Should `compatibleAgentDirs` be set to `[]` for all agents?**
   **Recommendation**: **Yes**. Prevents reading from agent dirs and avoids conflicts.

5. **Should `primaryAgentDir` point to the agent’s skills directory (e.g., `.claude/skills`)?**
   **Recommendation**: **Yes**. Ensures Gold Band can symlink missing skills.

6. **Should the owner test for conflicts after moving skills?**
   **Recommendation**: **Yes**. Restart Gold Band and verify no `SyncConflict` errors appear.

---
**SUMMARY**:
Gold Band reads skills from global (`~/.gold-band/skills/`), project (`<repo>/.gold-band/skills/`), and per-agent directories (primary/compatible). It **never writes** to compatible dirs (e.g., `.claude/skills`) but **symlinks** missing skills into `primaryAgentDir`. The owner’s Claude skills appear because `.claude/skills` is a compatible dir. **Recommendation**: Move all skills to `~/.gold-band/skills/`, set `compatibleAgentDirs: []`, and let Gold Band symlink them. This avoids conflicts and duplication. Test by verifying no duplicate skills appear and no symlinks are created unless a skill is missing. Key decisions: centralize skills, disable compatible dirs, and test for conflicts.