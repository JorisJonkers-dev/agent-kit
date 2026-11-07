# Prompt for the next agent: full Gold Band adoption

Paste everything below the line into a fresh agent session started in this repository on branch `feat/gold-band-epic`.

---

You continue the Gold Band adoption for agent-kit (JorisJonkers-dev/agent-kit). The single epic is https://github.com/JorisJonkers-dev/agent-kit/issues/368 ("Epic: adopt Gold Band for agent setup, delegation workflows and the skills port"). Read it first, then its three specs, then `handoff-gold-band/HANDOFF.md`. The aim is full Gold Band adoption: agents, roles, skills and MCP servers declared in one registry and converged by setup; the three delegation workflows (deep research, implementation, grill to tickets) running headless on Gold Band; the existing skills ported to Gold Band targets or recorded as staying outside.

## Tracker map

- Epic #368 has sub-issues: spec #327 (tickets #328–#349), spec #350 (tickets #351–#365), spec #367 (skills port; tickets not cut yet, they wait on the #351 inventory). Dependency order: #327, then #350, then #367. #326 is the older surface spec, amended by #327 and folded into it; it has no parent and its fate (close or attach to the epic) is an open owner decision. #285 (research delegation) may be superseded by #361; also an open owner decision.
- Tickets carry native blocked-by links. `node ~/.claude-personal/skills/critique/critique.mjs check-tickets --json '{"repo":"<root>","spec":"350"}'` passes; the 12 "dangling" blockers on #350 tickets are #327 tickets and are expected.
- Epic rules: new code is Rust under the 7-day crate cooldown (standards ADR-0014); no Codex, no OpenRouter, no `opencode/` Zen models in any Gold Band workflow or provider list (ADR-0007's nedap Codex grant applies only to gateway delegations); Opus only in judgment nodes (escalation after the repair cap, final acceptance); Gold Band facts come from commit 987fc3e and are re-read by the implementer; Gold Band workflows bypass the model gateway (ADR written by #352); the allowlist check runs at setup AND when each run starts (owner decision, now in #352).
- Budgets: 255 EUR Vibe key, 128 EUR Vibe-study key, 60 EUR OpenCode Go key (LiteLLM per-key `max_budget`, authoritative), Claude Max x5.

## State of the work (this branch)

Branch `feat/gold-band-epic` holds three commits on top of `feat/gateway-known-families-personal-default` (commit 7a52003, which was unpushed when this was written):

1. #331 DONE and reviewed (`/review-implementation`, 2 findings fixed): a std-only cargo workspace in `scripts/` (member `agent-kit-setup`), called by `scripts/converge.py` with a 30 s timeout and a fail-closed gate; `.githooks/pre-commit` and `.github/workflows/crate-cooldown.yml` run `crate-age --root` (crate-age is built from the standards repo, never copied); waiver route in `docs/RUST.md` and `docs/waivers/crate-cooldown.yaml`. 9 canary tests and the full suite (1131) pass. NOT verified: the 100 percent line coverage of the crate (`cargo llvm-cov` is not installed; decide the tool and measure), the workflow on a real runner (it checks out the standards repo, which may be private), and that `core.hooksPath` is set (`git config core.hooksPath .githooks`).
2. #351 PARTIAL: `docs/gold-band-port-inventory.md` (68 rows). It was written by a Vibe worker with no access to `gh` or the Gold Band facts, from repository files only, and it has NOT been spot-checked or reviewed. Ticket criteria still open: the Rust check script that fails when a skill directory has no entry (new Rust code, now unblocked by #331), and the proposed wave order must be verified against the specs. Read it critically; open questions listed in it go to the owner.
3. Handoff documents under `handoff-gold-band/` (HANDOFF.md, spec drafts, reports, config artifacts, this file). `reports/gb-src` (a 57 MB shallow clone of Gold Band at 987fc3e) and a network log are not committed; re-clone Gold Band at 987fc3e if you need source.

Next on the frontier (all unblocked once #331 is merged): #328 and #329 and #330 need the owner's real Gold Band app/binaries; #332 registry and validation (Rust); #355 export wrapper and #356 research repo scaffold (both need only #331); then #347 headless Gold Band binary (run `crate-age` over Gold Band's `Cargo.lock` first), #353 gate wrapper, and the rest in dependency order. Cut #367's tickets with `/to-tickets 367` once #351 is final, then `/review-tickets 367`.

## How to work

- Delegate through the model gateway (`/delegate`): cheap first. Vibe `glm-5-3` and `deepseek-v4-1-flash-1017`, Vibe-study, opencode-go models for most work; Sonnet 5.5 or Haiku only for the hardest tasks; Opus orchestrates. Codex is banned, OpenRouter and `opencode/` Zen models never.
- LESSONS from wave 1: the `claude` client in the gateway's non-interactive worktree had every write and shell command refused (it delivered nothing); Vibe GLM writing delegations work, but their sandbox may deny `gh` and paths outside the worktree, so embed issue text and facts in the prompt. Read every staged diff and run the tests yourself: the Vibe worker invoked `crate-age` with the wrong arguments (the real interface is `--root DIR`) and shipped two off-by-one test assertions. A leftover `.write-probe` file shows up in worktrees; remove it.
- The repo has a PreToolUse commit gate: risky changes (e.g. `.github/workflows/**`) cannot be committed until the owner runs `/review-implementation`; stage in a command of its own. The review skills (`/review-spec`, `/review-tickets`, `/review-implementation`, `/to-tickets`) cannot be invoked by the agent; ask the owner to run them.
- Chat style: terse caveman; anything persisted (issue bodies, comments, commits, docs) is normal English. The owner wants multiple-choice questions (AskUserQuestion) for decisions.
- Unverified items to settle by real runs (record each outcome on its ticket): GLM and DeepSeek reachable via LiteLLM at the keys; LiteLLM `max_budget` window, cap behaviour and unit; how one gate agent id is pinned to one wrapper; what `ai-dynamic` does with self-proposed fan-out; whether the ticket body counts as history; the unrolled-loop routing; and the Gold Band file:line facts (spot-check before relying).
- Owner-dependent: real API keys (#354), real runs (#360, #365), the real Gold Band app (#328, #329; quit it before applying config), OpenCode and Antigravity reach cells (#330), installing `handoff-gold-band/config/bin/pi-read`.
