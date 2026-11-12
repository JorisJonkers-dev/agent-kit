// The scenarios for "worktrees clean": the writing worktrees a workflow's runs kept, removed with
// their branches. The git the command runs is a fake bin/git shell script written under bin/, so
// every call it makes lands in the golden's files (work/git-calls) and no real repository is
// touched; PATH is the sandbox's bin/ alone, and the clock is fixed per step. Imported by
// gen-scenarios.mjs, which writes them out.

const SBX = '@SANDBOX@'

export function addWorktreeScenarios(scenarios, step) {
  /** A step whose PATH is the sandbox's bin/, at its own instant. */
  const on = (argv, extra = {}) => step(argv, { path: ['bin'], ...extra })
  const text = (input) => (typeof input === 'string' ? input : JSON.stringify(input))
  const cleanStep = (input, extra) => on(['worktrees', 'clean', '--json', text(input)], extra)
  const lines = (...body) => body.join('\n')
  const script = (name, body) => ({ ['bin/' + name]: '#!/bin/sh\n' + body + '\n' })
  const dirs = (...names) => names.map((name) => 'work/' + name)

  // The fake git: it logs the call it was given, and decides by the names in it. A worktree path
  // holding "failrm" makes "worktree remove" fail with a message, one holding "failquiet" fails
  // without a word; a repo holding "failprune" makes "worktree prune" fail (prune is given no
  // path, so the repo is what names it), and a branch holding "failbranch" makes "branch -D"
  // fail. A "worktree remove" that succeeds takes the directory with it, as git does.
  const GIT = script(
    'git',
    lines(
      'echo "$@" >> "$HOME/../work/git-calls"',
      'case "$3 $4" in',
      '  "worktree remove")',
      '    case "$6" in',
      '      *failrm*) echo "fatal: $6 is not a working tree" >&2; exit 128 ;;',
      '      *failprune*) echo "fatal: cannot remove $6" >&2; exit 128 ;;',
      '      *failquiet*) exit 1 ;;',
      '    esac',
      '    rm -rf "$6"',
      '    exit 0 ;;',
      '  "worktree prune")',
      '    case "$*" in',
      '      *failprune*) echo "error: failed to prune worktrees" >&2; exit 1 ;;',
      '      *failquiet*) exit 1 ;;',
      '    esac',
      '    exit 0 ;;',
      '  "branch -D")',
      '    case "$5" in',
      '      *failbranchquiet*) exit 1 ;;',
      '      *failbranch*) echo "error: the branch $5 is not fully merged" >&2; exit 1 ;;',
      '    esac',
      '    exit 0 ;;',
      'esac',
      'exit 0',
    ),
  )

  const KEPT = 'state/agent-kit/model-gateway/kept-worktrees.json'
  /** One kept worktree as the file holds it: the fields kept.add writes, then whatever else. */
  const wt = (o) => ({ repo: SBX + '/work/repo-a', path: SBX + '/work/wt-a', branch: 'gateway/a', run_id: 'a', workflow: 'w1', ...o })
  /** The kept file as the gateway writes it: two-space indentation, a newline at the end. */
  const keptPretty = (worktrees) => ({ [KEPT]: JSON.stringify({ version: 1, worktrees }, null, 2) + '\n' })
  /** The kept file as some other writer left it, compact or not JSON at all. */
  const keptFile = (value) => ({ [KEPT]: text(value) })

  const A = wt({ at: '2026-11-03T10:00:00.000Z', note: 'kept for the owner' })
  const B = wt({ repo: SBX + '/work/repo-b', path: SBX + '/work/wt-b', branch: 'gateway/b', run_id: 'b', workflow: 'w2' })
  const C = wt({ at: '2026-11-03T10:05:00.000Z', path: SBX + '/work/wt-c', branch: 'gateway/c', run_id: 'c' })
  const D = wt({ repo: SBX + '/work/repo-c', path: SBX + '/work/wt-d', branch: 'gateway/d', run_id: 'd', workflow: 'w3', extra: { nested: true } })
  const E = wt({ path: SBX + '/work/wt-e', branch: 'gateway/e', run_id: 'dup', workflow: 'w2' })
  const F = wt({ path: SBX + '/work/wt-f', branch: 'gateway/f', run_id: 'dup', workflow: 'w3' })
  const H = wt({ path: SBX + '/work/wt-h', branch: 'gateway/h', run_id: 'h', workflow: 'w9' })

  // Only the chosen workflow's worktrees go, the rest stay in the file as they were written, and a
  // second run over the same workflow finds nothing left to do.
  scenarios['worktrees-clean-workflow'] = [
    cleanStep({ workflow: 'w1' }, {
      write: { ...GIT, ...keptPretty([A, B, C, D]) },
      exec: ['bin/git'],
      mkdir: dirs('repo-a', 'repo-b', 'repo-c', 'wt-a', 'wt-b', 'wt-c', 'wt-d', 'wt-e', 'wt-f', 'wt-h'),
    }),
    cleanStep({ workflow: 'w1' }),
    cleanStep({ workflow: 'w2' }),
    cleanStep({ workflow: 'nobody' }),
    cleanStep({ workflow: 'w2' }, { write: keptPretty([E, F]) }),
    cleanStep({ all: true }),
    cleanStep({ all: true }),
    cleanStep({ workflow: 'nobody' }, { write: keptFile({ version: 1, worktrees: [H] }) }),
  ]

  // Every kept worktree goes, whatever came of it: one whose worktree git cannot remove stays in
  // the file, one whose branch git cannot delete is still removed.
  const ok1 = wt({ path: SBX + '/work/wt-ok1', branch: 'gateway/ok1', run_id: 'ok1', workflow: 'w1' })
  const bad1 = wt({ path: SBX + '/work/wt-failrm1', branch: 'gateway/bad1', run_id: 'bad1', workflow: 'w1' })
  const ok2 = wt({ path: SBX + '/work/wt-ok2', branch: 'gateway/failbranch2', run_id: 'ok2', workflow: 'w2' })
  const ok3 = wt({ path: SBX + '/work/wt-ok3', branch: 'gateway/ok3', run_id: 'ok3', workflow: 'w2' })
  scenarios['worktrees-clean-all'] = [
    cleanStep({ all: true }, {
      write: { ...GIT, ...keptPretty([ok1, bad1, ok2, ok3]) },
      exec: ['bin/git'],
      mkdir: dirs('repo-a', 'wt-ok1', 'wt-failrm1', 'wt-ok2', 'wt-ok3'),
    }),
    cleanStep({ all: true }),
    cleanStep({ all: true }, { write: keptPretty([ok1, ok3]), mkdir: dirs('wt-ok1', 'wt-ok3') }),
    cleanStep({ all: true }),
    cleanStep({ all: true }),
  ]

  // What git says when it will not remove a worktree, and what the command does about it: a
  // worktree git refuses is deleted by hand and pruned, and only a prune that also fails leaves a
  // failed entry; a branch git will not delete is reported beside the worktree it removed.
  const r1 = wt({ path: SBX + '/work/wt-failrm1', branch: 'gateway/r1', run_id: 'r1' })
  const r2 = wt({ repo: SBX + '/work/repo-failprune', path: SBX + '/work/wt-failprune2', branch: 'gateway/r2', run_id: 'r2' })
  const r3 = wt({ path: SBX + '/work/wt-ok3', branch: 'gateway/failbranch3', run_id: 'r3' })
  const r4 = wt({ path: SBX + '/work/wt-failquiet4', branch: 'gateway/r4', run_id: 'r4' })
  const r5 = wt({ repo: SBX + '/work/repo-failquiet5', path: SBX + '/work/wt-failquiet5', branch: 'gateway/r5', run_id: 'r5' })
  const r6 = wt({ path: SBX + '/work/wt-ok6', branch: 'gateway/failbranchquiet6', run_id: 'r6' })
  const r7 = wt({ path: SBX + '/work/wt-failrm7', branch: 'gateway/r7', run_id: 'r7' })
  const r8 = wt({ path: SBX + '/work/wt dir/ünï ✓', branch: 'gateway/r8', run_id: 'r8' })
  const one = (entry, extra = {}) => cleanStep({ all: true }, { write: keptPretty([entry]), ...extra })
  scenarios['worktrees-clean-failures'] = [
    cleanStep({ all: true }, { write: { ...GIT, ...keptPretty([r1]) }, exec: ['bin/git'], mkdir: dirs('repo-a', 'wt-failrm1') }),
    one(r2, { mkdir: dirs('repo-failprune', 'wt-failprune2') }),
    one(r3, { mkdir: dirs('wt-ok3') }),
    one(r4, { mkdir: dirs('wt-failquiet4') }),
    one(r5, { mkdir: dirs('repo-failquiet5', 'wt-failquiet5') }),
    one(r6, { mkdir: dirs('wt-ok6') }),
    one(r7),
    one(r8, { mkdir: ['work/wt dir/ünï ✓'] }),
    one(r1, { remove: ['bin/git'] }),
    cleanStep({ all: true }),
  ]

  // The selection: the workflow that ended, or all, not both and not neither, and the arguments
  // that carry it.
  scenarios['worktrees-clean-input'] = [
    cleanStep({}),
    cleanStep({ workflow: 'w1', all: true }),
    cleanStep({ all: false }),
    cleanStep({ all: 'true' }),
    cleanStep({ all: 1 }),
    cleanStep({ all: null }),
    cleanStep({ workflow: '' }),
    cleanStep({ workflow: 5 }),
    cleanStep({ workflow: null }),
    cleanStep({ workflow: 'bad name' }),
    cleanStep({ workflow: '-lead' }),
    cleanStep({ workflow: 'a'.repeat(81) }),
    cleanStep({ workflow: 'ünï' }),
    cleanStep('[]'),
    cleanStep('5'),
    cleanStep('null'),
    cleanStep('"text"'),
    cleanStep('not json'),
    on(['worktrees', 'clean']),
    on(['worktrees', 'clean', '--json']),
    on(['worktrees', 'clean', '--json', '{}', 'extra']),
    on(['worktrees', 'clean', '--bogus', '{}']),
    on(['worktrees', 'clean', '--input', 'missing.json']),
    on(['worktrees', 'clean', '--input', 'in.json'], { write: { 'work/in.json': '{"workflow":"w1"}' } }),
    on(['worktrees', 'clean', '--input', 'in.json'], { write: { 'work/in.json': '{"all":true}' } }),
  ]

  // The kept file itself: not there yet, not JSON, not the shape it must be, a version this port
  // does not know (it reads the worktrees whatever the version says), a directory in its place,
  // and the state directory the environment names.
  const S = wt({ path: SBX + '/work/wt-s', branch: 'gateway/s', run_id: 's' })
  scenarios['worktrees-clean-state'] = [
    cleanStep({ all: true }, { write: GIT, exec: ['bin/git'] }),
    cleanStep({ all: true }, { write: keptFile('not json') }),
    cleanStep({ all: true }, { write: keptFile('[]') }),
    cleanStep({ all: true }, { write: keptFile('{}') }),
    cleanStep({ all: true }, { write: keptFile('null') }),
    cleanStep({ all: true }, { write: keptFile({ version: 1 }) }),
    cleanStep({ all: true }, { write: keptFile({ version: 1, worktrees: {} }) }),
    cleanStep({ all: true }, { write: keptFile({ version: 2, worktrees: [S] }), mkdir: dirs('wt-s') }),
    cleanStep({ all: true }, { write: keptFile({ version: 1, worktrees: [S] }), mkdir: dirs('wt-s') }),
    cleanStep({ all: true }, { remove: [KEPT], mkdir: [KEPT] }),
    cleanStep({ all: true }, { remove: ['state'], write: { 'state/agent-kit': 'x' } }),
    cleanStep({ all: true }, { env: { XDG_STATE_HOME: null, HOME: null } }),
    cleanStep({ all: true }, { env: { XDG_STATE_HOME: 'relative/state' } }),
    cleanStep({ all: true }, { env: { XDG_STATE_HOME: '' } }),
    cleanStep({ all: true }, { env: { XDG_STATE_HOME: SBX + '/work/elsewhere' } }),
  ]

  // Kept entries that name no repo, branch, run_id or workflow, or name one with something that is not
  // a string: the node runner hands whatever it finds to git as JavaScript writes it ("undefined", "5"),
  // and this port does the same. An entry with no path is not here: the node runner cannot delete it
  // by hand, and this port refuses it with a reason of its own (see the unit tests).
  scenarios['worktrees-malformed-entries'] = [
    cleanStep({ all: true }, {
      write: {
        ...GIT,
        ...keptPretty([
          { repo: SBX + '/work/repo-a', path: SBX + '/work/wt-a', branch: 'gateway/a', run_id: 'a', workflow: 'w1' },
          { path: SBX + '/work/wt-b', branch: 'gateway/b', run_id: 'b', workflow: 'w1' },
          { repo: SBX + '/work/repo-a', path: SBX + '/work/wt-d', run_id: 'd', workflow: 'w1' },
          { repo: SBX + '/work/repo-a', path: SBX + '/work/wt-e', branch: 'gateway/e', workflow: 'w1' },
          { repo: SBX + '/work/repo-a', path: SBX + '/work/wt-f', branch: 'gateway/f', run_id: 'f' },
          { repo: 5, path: SBX + '/work/wt-g', branch: 'gateway/g', run_id: 'g', workflow: 'w1' },
          { repo: SBX + '/work/repo-a', path: SBX + '/work/wt-h', branch: 7, run_id: 'h', workflow: 'w1' },
        ]),
      },
      exec: ['bin/git'],
      mkdir: dirs('repo-a', 'wt-a', 'wt-b', 'wt-d', 'wt-e', 'wt-f', 'wt-g', 'wt-h'),
    }),
    cleanStep({ workflow: 'w1' }),
    cleanStep({ all: true }),
    cleanStep({ all: true }),
  ]
}
