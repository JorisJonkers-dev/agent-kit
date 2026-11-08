#!/usr/bin/env bash
# Guided probe for ticket #328: Gold Band real-app facts.
# Run it yourself: bash handoff-gold-band/tools/probe-328.sh
# It pauses at every step that needs you in the Gold Band window, and writes
# every answer, with the screen or command used, to the results file below.
set -euo pipefail

HERE="$(cd "$(dirname "$0")/.." && pwd)"
RESULTS="$HERE/reports/probe-328-results.md"
SETTINGS="$HOME/.gold-band/settings.json"
SCRATCH="$HOME/IDEAProjects/gb-probe-328"
STAMP="$(date -u +%Y%m%dT%H%M%SZ)"
BACKUP="$SETTINGS.bak-328-$STAMP"

say() { printf '\n== %s\n' "$*"; }
ask() { # ask <question>; records the answer line
  local q="$1" a
  printf '%s\n> ' "$q"; IFS= read -r a
  printf -- '- %s\n  - answer: %s\n' "$q" "$a" >> "$RESULTS"
}
note() { printf -- '%s\n' "$*" >> "$RESULTS"; }
wait_enter() { printf '%s [Enter] ' "$1"; IFS= read -r _; }
app_running() { pgrep -f '/Applications/Gold Band.app/' >/dev/null 2>&1; }

mkdir -p "$(dirname "$RESULTS")"
printf '# #328 probe results (%s)\n\nRun by the owner with handoff-gold-band/tools/probe-328.sh.\n\n' "$STAMP" > "$RESULTS"

say "Step 0: quit Gold Band"
while app_running; do wait_enter "Gold Band is running. Quit it (Cmd-Q), then press"; done
cp -p "$SETTINGS" "$BACKUP"
echo "Backup: $BACKUP"
note "Settings backup: \`$BACKUP\`"

say "Step 1: scratch personal repository"
if [ ! -d "$SCRATCH/.git" ]; then
  mkdir -p "$SCRATCH" && git -C "$SCRATCH" init -q
  printf 'probe\n' > "$SCRATCH/README.md"
  git -C "$SCRATCH" add README.md && git -C "$SCRATCH" commit -qm "probe: start"
fi
echo "Scratch repo: $SCRATCH (no remote, personal)"
note "Scratch repo: \`$SCRATCH\`"

say "Step 2: merge the claude-personal agent into settings.json"
python3 - "$SETTINGS" "$HERE/config/agents.json" <<'PY'
import json, sys
settings_path, agents_path = sys.argv[1], sys.argv[2]
settings = json.load(open(settings_path))
drafted = json.load(open(agents_path))
drafted = drafted.get("agents", drafted)
agents = settings.get("agents") or {}
agents["claude-personal"] = drafted["claude-personal"]
settings["agents"] = agents
tmp = settings_path + ".tmp"
json.dump(settings, open(tmp, "w"), indent=2)
import os; os.replace(tmp, settings_path)
print("agents now:", ", ".join(sorted(agents)))
PY
note "Command: merged \`claude-personal\` from \`config/agents.json\` into the \`agents\` key; ids before: claude-acp, opencode."

say "Step 3: merge semantics"
wait_enter "Start Gold Band, open the agent picker (new session), then press"
note ""; note "## Merge semantics (screen: agent picker of a new session)"
ask "Which agents does the picker list? (type the names)"
ask "Are the built-ins not in settings.json (codex-acp, gemini, cursor, ...) still listed? (yes/no)"
ask "Can codex-acp be selected and started? (yes/no/not listed)"
python3 -c "import json,sys;d=json.load(open(sys.argv[1]));print('agents after app load:', ', '.join(sorted((d.get('agents') or {}).keys())))" "$SETTINGS" | tee -a "$RESULTS"

say "Step 4: Claude identity (environment forwarding)"
echo "In Gold Band, in workspace $SCRATCH, start one session with 'Claude' (claude-acp)"
echo "and one with 'Claude (personal)'. In each, send exactly:"
echo '   Run this shell command and show the output: echo "$CLAUDE_CONFIG_DIR"; pwd'
wait_enter "When both answered, press"
note ""; note "## Claude environment forwarding (command: echo \$CLAUDE_CONFIG_DIR in each session)"
ask "claude-acp session printed CLAUDE_CONFIG_DIR as:"
ask "claude-personal session printed CLAUDE_CONFIG_DIR as:"
slug="$(printf '%s' "$SCRATCH" | sed 's#[/.]#-#g')"
for home in "$HOME/.claude" "$HOME/.claude-personal"; do
  d="$home/projects/$slug"
  if [ -d "$d" ]; then n=$(ls "$d" | wc -l | tr -d ' '); else n=0; fi
  echo "sessions under $d: $n" | tee -a "$RESULTS"
done

say "Step 5: do our critique hooks fire for Gold Band-launched Claude?"
seq 1 400 > "$SCRATCH/big.txt"; git -C "$SCRATCH" add big.txt
echo "A 400-line file is staged in $SCRATCH. In the claude-acp session send:"
echo '   Run: git commit -m "probe: big change"'
echo "Our commit gate should block it with 'Commit blocked: the staged change is risky'."
wait_enter "When it answered, press"
note ""; note "## Critique hooks (command: git commit of a 400-line staged change in the claude-acp session)"
ask "Was the commit blocked by the critique commit gate? (yes/no, paste the message)"
if git -C "$SCRATCH" log --oneline -1 | grep -q 'big change'; then
  echo "git log: the commit landed (hook did not block)" | tee -a "$RESULTS"
else
  echo "git log: no 'big change' commit" | tee -a "$RESULTS"
fi
git -C "$SCRATCH" reset -q HEAD big.txt 2>/dev/null || true

say "Step 6: skills and node settings"
echo "In Gold Band: open an OpenCode agent's settings, then create a test workflow with one worker node."
wait_enter "When you have both screens open, press"
note ""; note "## Skills and node settings (screens: agent settings, workflow node editor)"
ask "OpenCode agent: which skill-directory fields are shown (primary/compatible dirs), and do skills from ~/.agents/skills appear for it?"
ask "Workflow node editor: is there a working-directory field per node? (yes/no)"
ask "Workflow node editor: can skills be limited per node? (yes/no, what is shown)"

say "Step 7: restore"
printf 'Keep claude-personal in settings.json? (y = keep, n = restore backup) > '; IFS= read -r keep
if [ "$keep" != "y" ]; then
  while app_running; do wait_enter "Quit Gold Band first, then press"; done
  cp -p "$BACKUP" "$SETTINGS"; note "Restored settings from the backup."
else
  note "Kept claude-personal in settings.json."
fi
chmod -R go-rwx "$HOME/.gold-band"
echo; echo "Done. Results: $RESULTS"
