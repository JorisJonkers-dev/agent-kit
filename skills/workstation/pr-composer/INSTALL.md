# Installing pr-composer

This skill drives `git` and `gh` against a real checkout, so it belongs in Claude Code, not in a claude.ai chat.

## Claude Code (recommended)

Skills are filesystem-based — nothing to upload, no packaging step.

```bash
# Personal: available in every project
mkdir -p ~/.claude/skills
cp -r pr-composer ~/.claude/skills/

# or Project: committed, available to everyone on the repo
mkdir -p .claude/skills
cp -r pr-composer .claude/skills/
```

The **directory name is the command name**, so `~/.claude/skills/pr-composer/SKILL.md` gives you `/pr-composer`. The `name:` field in the frontmatter is only the display label at these levels.

From a `.skill` file (it's a zip):

```bash
mkdir -p ~/.claude/skills && unzip pr-composer.skill -d ~/.claude/skills/
```

Verify: run `/skills`, or ask "what skills are available?". Claude Code watches these directories, so edits to `SKILL.md` are picked up mid-session — but a *newly created* top-level skills directory needs a restart before it's watched.

## Scope, in one line each

| Location | Path | Applies to |
|---|---|---|
| Personal | `~/.claude/skills/pr-composer/` | all your projects |
| Project | `.claude/skills/pr-composer/` | that repo, committed |
| Plugin | `<plugin>/skills/pr-composer/` | wherever the plugin is enabled, as `/plugin:pr-composer` |
| Enterprise | managed settings | everyone in the org |

Conflicts resolve enterprise → personal → project, and any of them overrides a bundled skill of the same name.

## Frontmatter: two dialects

The frontmatter here uses only the six fields the Agent Skills spec allows — `name`, `description`, `license`, `compatibility`, `metadata`, `allowed-tools` — so it both runs in Claude Code and survives packaging or upload to claude.ai.

Claude Code accepts more fields, but adding any of them makes `package_skill.py` and claude.ai upload fail with a hard error rather than ignoring them:

```
Unexpected key(s) in SKILL.md frontmatter: argument-hint. Allowed properties are:
allowed-tools, compatibility, description, license, metadata, name
```

Two worth considering if you're installing locally and never packaging:

```yaml
disable-model-invocation: true   # only /pr-composer triggers it, never Claude on its own
argument-hint: "[issue numbers or slack links]"
```

`disable-model-invocation` is the documented pattern for skills with side effects. It's left off here because the confirmation gate lives inside the skill, so auto-triggering can't create anything — but if you'd rather the skill never load unless you type the command, add it and keep a separate un-packaged copy.

## Permissions

`allowed-tools` pre-approves the read-only half: the two bundled scripts, and the `git`/`gh` read commands. The grant lasts for the turn that invoked the skill and clears on your next message.

Mutating commands are deliberately absent, so `git push`, `gh pr create`, `gh pr edit`, and `gh project item-add` still hit Claude Code's permission prompt. Don't add them to `allowed-tools` — that prompt is the backstop if the skill's own confirmation gate is ever bypassed by a prompt injection in an issue body or Slack thread.

## Prerequisites

- `gh` authenticated (`gh auth status`)
- `gh auth refresh -s project,read:project` for project-board support — the default login doesn't include it
- Python 3 (stdlib only, no packages)

## Where it won't work

Cowork and cloud sessions don't read `~/.claude/skills/` from your machine. For those, either enable the skill for your claude.ai account or commit it to the repo's `.claude/skills/`. Note that skills synced from claude.ai have body features disabled — `!` command injection doesn't run, and some `${CLAUDE_*}` placeholders arrive as literal text — so a local or committed install is the reliable path for this skill.
