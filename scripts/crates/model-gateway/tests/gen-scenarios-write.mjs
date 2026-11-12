// The scenarios for a writing `run`: the same fake client programs as gen-scenarios-run.mjs, but launched
// in a worktree the gateway makes off the caller's base, where the fake changes files through its `act`
// snippet (it runs with the worktree as its working directory) and the gateway then stages that work and
// runs the caller's check with `sh -c`. A step snapshots the run directory, the ledger, the kept-worktrees
// file and the worktree itself, so the goldens show what a writing run leaves behind.
//
// A writing step adds `sh` (the check runs `sh -c`) and `touch` (what the snippets use) to the shims; the
// fake programs, the canned behaviour, the repositories and the grants come from gen-scenarios-run.mjs.
// Imported by gen-scenarios.mjs, which writes them out; regenerate-fixtures.mjs makes the goldens with the
// node runner. Run ids and temporary directories are masked as there (@RAND1@, @TMP1@), and time is fixed.

import {
  REPO,
  ROOTS,
  SBX,
  allowlist,
  behaves,
  claudeOut,
  clients,
  codexOut,
  grant,
  grants,
  journal,
  merge,
  repo,
  runSteps,
  vibeExport,
} from './gen-scenarios-run.mjs'

/**
 * The identity and the instants a delegate's own `git commit` uses. The sandbox's HOME holds no git
 * configuration, so a commit without these would fail, and a commit with the host's clock would not be
 * the same twice.
 */
const COMMITTER = {
  GIT_AUTHOR_DATE: '2026-11-03T12:00:00Z',
  GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
  GIT_AUTHOR_NAME: 'Fixture',
  GIT_COMMITTER_DATE: '2026-11-03T12:00:00Z',
  GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
  GIT_COMMITTER_NAME: 'Fixture',
}

export function addWriteScenarios(scenarios, step) {
  const { run } = runSteps(step)
  /** A writing run: the mode, the base and the check, with the caller's own fields over them. */
  const write = (input, extra) => run({ mode: 'write', repo: REPO, base: 'main', check: 'test -f new.txt', ...input }, extra)
  /** A writing run whose input is given whole, for the shapes that are refused before anything is launched. */
  const refusal = (input, extra) => run({ mode: 'write', ...input }, extra)
  /** The fake client programs, with the shell the check runs in and the tool the snippets use. */
  const sandbox = (...names) => merge(clients(...names), { shims: ['sh', 'touch'] })

  // A writing run that works: the delegate edits a file and adds one, the gateway stages both and lists
  // them, the check passes, and the worktree is kept for the caller beside its run directory and ledger line.
  scenarios['run-write-happy'] = [
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Add the new file.' }, merge(
      sandbox('claude'),
      repo('anthropic'),
      behaves('claude', { act: 'echo changed > README.md\ntouch new.txt', out: claudeOut() }),
    )),
    // A delegate that commits its own work: the listing is taken against the base, so the commit is listed too.
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Commit the change.', check: 'test "$(git log --format=%s -1)" = "feat: change the readme"' }, merge(
      behaves('claude', { act: "echo committed > README.md\ngit add -A\ngit -c commit.gpgsign=false commit -q -m 'feat: change the readme'", out: claudeOut() }),
      { env: COMMITTER, now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // The check fails: its exit code and its output are kept in the run directory, and the run says why.
  scenarios['run-write-check-fails'] = [
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Break the build.', escalate: false, check: "echo 'the build broke'; exit 3" }, merge(
      sandbox('claude'),
      repo('anthropic'),
      behaves('claude', { act: 'echo broken > README.md', out: claudeOut() }),
    )),
  ]

  // A delegate that does not finish: nothing is staged and no check runs, whether it exits non-zero or
  // runs past its time limit.
  scenarios['run-write-delegate-fails'] = [
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Fail.', escalate: false }, merge(
      sandbox('claude'),
      repo('anthropic'),
      behaves('claude', { act: 'echo half done > README.md', err: 'it broke', exit: 3, out: 'partial' }),
    )),
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Too slow.', time_limit_s: 1, escalate: false }, merge(
      behaves('claude', { sleep: '1' }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // A Codex writing run: the git directories its sandbox may write are on the command line, and the brief
  // the repository keeps is put in front of the prompt, cut at 20000 characters.
  scenarios['run-write-codex'] = [
    write({ client: 'codex', model: 'gpt-5.5', prompt: 'Add the flag.' }, merge(
      sandbox('codex'),
      repo('openai'),
      { write: { 'repo/codex-brief.md': 'This repository keeps its brief here.\n' } },
      grants(grant({ cap_usd: 5 })),
      behaves('codex', { act: 'echo flag > README.md\ntouch new.txt', out: codexOut() }),
    )),
    write({ client: 'codex', model: 'gpt-5.5', prompt: 'A brief that is too long.' }, merge(
      { write: { 'repo/codex-brief.md': `${'b'.repeat(20005)}\n` }, now: '2026-11-03T12:05:00Z' },
      behaves('codex', { act: 'touch new.txt', out: codexOut('Done.') }),
    )),
  ]

  // What a writing run needs before anything is launched, and the one failure that comes after the workflow
  // is joined: a base git cannot find leaves the workflow started and no worktree behind.
  scenarios['run-write-refusals'] = [
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'No base.', repo: REPO, check: 'true' }, merge(
      sandbox('claude'),
      repo('anthropic'),
    )),
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'No check.', repo: REPO, base: 'main' }, merge(
      { now: '2026-11-03T12:01:00Z' },
    )),
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'A base git cannot find.', repo: REPO, base: 'nope', check: 'true' }, merge(
      { now: '2026-11-03T12:02:00Z' },
    )),
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Not a repository.', repo: `${SBX}/work`, base: 'main', check: 'true' }, merge(
      { now: '2026-11-03T12:03:00Z' },
    )),
    refusal({ client: 'codex', model: 'gpt-5.5', prompt: 'No grant.', repo: REPO, base: 'main', check: 'true' }, merge(
      sandbox('codex'),
      allowlist('anthropic', 'openai'),
      { now: '2026-11-03T12:04:00Z' },
    )),
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'A family the entry does not allow.', repo: REPO, base: 'main', check: 'true' }, merge(
      allowlist('google'),
      { now: '2026-11-03T12:05:00Z' },
    )),
    refusal({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Nested.', repo: REPO, base: 'main', check: 'true' }, merge(
      { env: { AGENT_KIT_DELEGATION: '1' }, now: '2026-11-03T12:06:00Z' },
    )),
  ]

  // A checkout of the repository beside the run's own worktree: one the delegate changes fails the run and
  // is named in the result, while one under the gateway's own worktrees is this gateway's own workplace and
  // is not watched at all.
  scenarios['run-write-foreign'] = [
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Change the other checkout.', escalate: false }, merge(
      sandbox('claude'),
      repo('anthropic'),
      { git: [['-C', REPO, 'worktree', 'add', '--detach', `${SBX}/other`, 'main']] },
      behaves('claude', { act: 'echo foreign > "$HOME/../other/foreign.txt"\ntouch new.txt', out: claudeOut() }),
    )),
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'Change the gateway worktree.' }, merge(
      { git: [['-C', REPO, 'worktree', 'add', '--detach', `${REPO}/.git/model-gateway/worktrees/kept-one`, 'main']] },
      behaves('claude', { act: 'echo kept > "$HOME/../repo/.git/model-gateway/worktrees/kept-one/kept.txt"\ntouch new.txt', out: claudeOut() }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // The check cannot start: there is no `sh` on PATH, so it reports 127 and the reason as its output.
  scenarios['run-write-check-cannot-start'] = [
    write({ client: 'claude', model: 'claude-haiku-5-5', prompt: 'No shell to check with.', escalate: false }, merge(
      clients('claude'),
      { shims: ['touch'] },
      repo('anthropic'),
      behaves('claude', { act: 'touch new.txt', out: claudeOut() }),
    )),
  ]

  // A writing run that fails is retried, then climbs: every attempt gets a worktree and a run directory of
  // its own, and the later attempts link to the first.
  scenarios['run-write-escalation'] = [
    write({ client: 'vibe', model: 'mistral-small-latest', prompt: 'Make it work.' }, merge(
      sandbox('vibe'),
      repo('mistral'),
      behaves('vibe', { err: 'first failure text', exit: 1 }),
      behaves('vibe', { err: '', exit: 2, out: journal('Second try.') }, 2),
      behaves('vibe', { act: 'touch new.txt', exit: 0, export: vibeExport(), out: journal('Climbed.\nSUMMARY: on the stronger model') }, 3),
    )),
  ]

  // A Codex run under a grant that lapses sooner than the run's own time limit: the run is held to what is
  // left of the grant, so the delegate and the check it runs are held to that too.
  scenarios['run-write-employer-grant'] = [
    write({ client: 'codex', model: 'gpt-5.5', prompt: 'Under a short grant.', time_limit_s: 900 }, merge(
      sandbox('codex'),
      repo('openai'),
      grants(grant({ cap_usd: 5, expires_at: '2026-11-03T12:01:30Z' })),
      behaves('codex', { act: 'touch new.txt', out: codexOut() }),
    )),
  ]

  // Every step snapshots the temporary directory and the repository's gateway directories besides the usual
  // roots; the second checkout the foreign-worktree scenario changes is snapshotted too.
  for (const name of Object.keys(scenarios)) {
    if (name.startsWith('run-write')) scenarios[name] = scenarios[name].map((one) => ({ ...one, roots: ROOTS }))
  }
  scenarios['run-write-foreign'] = scenarios['run-write-foreign'].map((one) => ({ ...one, roots: [...ROOTS, 'other'] }))
}
