// The scenarios for `batch`: a whole plan of delegations handed over in one call, checked as a whole
// before anything runs, then run through the same path as `run` (retry, climb, policy, caps and grants),
// with the delegations that depend on one that failed for good skipped. The fake client programs, the
// canned behaviour, the repositories, the grants and the machine overrides come from
// gen-scenarios-run.mjs; a step's PATH is the sandbox's bin/ alone, and every step snapshots the
// temporary directory and the repository's gateway directories (ROOTS) besides the usual roots.
//
// Determinism: delegations that run in parallel make the ledger's line order and the numbering of run ids
// racy, so every scenario here either chains its delegations with `depends_on` (a dependent starts only
// once the one it waits for is done) or sets `max_parallel_per_client` to 1 in the machine override, and
// each scenario says which it does. Time is fixed, and the random parts of run ids and temporary
// directories are numbered by first appearance (@RAND1@, @TMP1@) by regenerate-fixtures.mjs.
//
// Imported by gen-scenarios.mjs, which writes them out; regenerate-fixtures.mjs makes the goldens with the
// node runner.

import {
  REPO,
  ROOTS,
  VIBE_ISOLATION,
  behaves,
  claudeOut,
  clients,
  codexOut,
  grant,
  grants,
  hermesMachine,
  hermesMachineWith,
  hermesUsage,
  journal,
  merge,
  override,
  repo,
  runSteps,
  text,
  vibeExport,
} from './gen-scenarios-run.mjs'

export function addBatchScenarios(scenarios, step) {
  const { on } = runSteps(step)
  /** One `batch` call: the plan as the caller sends it, or its text when the shape itself is the point. */
  const batch = (input, extra) => on(['batch', '--json', text(input)], extra)
  /** A plan of the delegations given. */
  const plan = (...delegations) => ({ delegations, workflow: 'w' })
  /** A read-only delegation of a plan, with the fields these scenarios share. */
  const read = (id, input = {}) => ({ client: 'claude', mode: 'read-only', model: 'claude-haiku-5-5', prompt: `TASK ${id}`, ...input, id })

  // A chain of two read-only delegations: the second is launched only once the first is done, with the
  // first's id, output path and capped summary appended to its prompt as data produced by another model,
  // never its full output. The result lists both in plan order, the ledger carries a line per delegation
  // tagged with the batch and the delegation, and each run keeps its prompt, output and result in a run
  // directory of its own.
  // Determinism: `review` depends on `find`, so the two never overlap; one client, one call at a time.
  scenarios['batch-happy'] = [
    batch(plan(
      { client: 'vibe', id: 'find', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Find where the flag is read.' },
      { client: 'vibe', depends_on: ['find'], id: 'review', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Review the finding.' },
    ), merge(
      clients('vibe'),
      behaves('vibe', { export: vibeExport(), out: journal('The flag is read in main.ts.\nSUMMARY: main.ts reads the flag') }, 1),
      behaves('vibe', { export: vibeExport({ cost_usd: 0.02 }), out: journal('The finding holds.\nSUMMARY: the finding holds') }, 2),
      { env: VIBE_ISOLATION },
    )),
    // The same chain in a repository: the run directories sit under the repository's gateway directory,
    // and a model without a passing read-only proof works in a throwaway worktree of it.
    // Determinism: `summarise` depends on `read`.
    batch(plan(
      { client: 'vibe', id: 'read', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Read the readme.', repo: REPO },
      { client: 'vibe', depends_on: ['read'], id: 'summarise', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Summarise it.', repo: REPO },
    ), merge(
      repo('mistral'),
      behaves('vibe', { export: vibeExport(), out: journal('A demo.\nSUMMARY: a demo repository') }, 3),
      behaves('vibe', { export: vibeExport(), out: journal('Short.\nSUMMARY: short summary') }, 4),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // A writing delegation: the gateway makes a worktree off the base on a branch of its own, the delegate
  // edits there, the gateway stages what it wrote and runs the caller's check, and the report names the
  // worktree and the branch the change is kept on. A check that fails fails the delegation, and its
  // worktree is kept too.
  // Determinism: one delegation per step, so nothing runs beside it.
  const write = (id, input, extra) =>
    batch(plan({ base: 'main', check: 'test -f new.txt', client: 'claude', id, mode: 'write', model: 'claude-haiku-5-5', prompt: 'Add the new file.', repo: REPO, ...input }), extra)
  scenarios['batch-write'] = [
    write('add', {}, merge(
      clients('claude'),
      { shims: ['sh', 'touch'] },
      repo('anthropic'),
      behaves('claude', { act: 'echo changed > README.md\ntouch new.txt', out: claudeOut() }),
    )),
    write('break', { check: "echo 'the build broke'; exit 3", escalate: false, prompt: 'Break the build.' }, merge(
      behaves('claude', { act: 'echo broken > README.md', out: claudeOut() }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // A delegation that fails for good skips only what depends on it, transitively: `b` waits for `a` and
  // `c` waits for `b`, so both are skipped and name the delegation that stopped them, while `d`, which
  // waits for nothing, still runs.
  // Determinism: max_parallel_per_client is 1, so the delegations run one at a time in plan order.
  scenarios['batch-skipped'] = [
    batch(plan(
      read('a', { escalate: false, prompt: 'Fail.' }),
      read('b', { depends_on: ['a'], prompt: 'After a.' }),
      read('c', { depends_on: ['b'], prompt: 'After b.' }),
      read('d', { prompt: 'Independent.' }),
    ), merge(
      clients('claude'),
      override({ settings: { max_parallel_per_client: 1 } }),
      behaves('claude', { err: 'it broke', exit: 3, out: 'partial' }, 1),
      behaves('claude', { out: claudeOut('Independent.\nSUMMARY: d is fine') }, 2),
    )),
  ]

  // Every plan the gateway refuses before it launches anything: the shape of the input, the plan's ids and
  // dependencies, and each delegation's own refusal (an unknown model, the data policy, the caps). A
  // refused plan launches nothing, creates no worktree, starts no workflow and reserves nothing, so these
  // steps leave no call log, no run directory and no ledger line behind.
  // Determinism: nothing runs at all.
  const good = { client: 'vibe', mode: 'read-only', model: 'mistral-small-latest', prompt: 'p' }
  const hermes = { client: 'hermes', mode: 'read-only', model: 'claude-sonnet-4.6', prompt: 'p', provider: 'anthropic' }
  scenarios['batch-refusals'] = [
    batch('[]'),
    batch('null'),
    batch({ delegations: [{ id: 'a', ...good }] }),
    batch({ delegations: [], workflow: 'w' }),
    batch({ delegations: 'a', workflow: 'w' }),
    batch(plan(5)),
    // More delegations than this machine allows.
    batch(plan({ id: 'a', ...good }, { id: 'b', ...good }, { id: 'c', ...good }), override({ settings: { max_batch_delegations: 2 } })),
    batch(plan({ id: 'a', ...good }, { id: 'a', ...good })),
    batch(plan({ id: 'has space', ...good })),
    batch(plan({ id: 'a'.repeat(41), ...good })),
    // The keys a delegation of a batch may not carry.
    batch(plan({ id: 'a', workflow: 'other', ...good })),
    batch(plan({ id: 'a', delegations: [], ...good })),
    batch(plan({ id: 'a', batch: {}, ...good })),
    batch(plan({ id: 'a', depends_on: 'b', ...good })),
    batch(plan({ id: 'a', depends_on: [1], ...good })),
    // A delegation's own field, refused with the delegation named.
    batch(plan({ id: 'a', ...good, mode: 'plan' })),
    batch(plan({ id: 'a', depends_on: ['nope'], ...good })),
    batch(plan({ id: 'a', depends_on: ['b'], ...good }, { id: 'b', depends_on: ['a'], ...good })),
    // Every problem at once: an unknown model, and a family the repository's entry does not allow.
    batch(plan({ id: 'unknown', ...good, model: 'a-model-nobody-knows' }, { id: 'policy', ...good, repo: REPO }), merge(repo('google'))),
    // The plan's metered estimates together do not fit the caps, so the plan holds nothing.
    batch(plan({ id: 'first', ...hermes }, { id: 'second', ...hermes }), merge(hermesMachineWith({ metered_personal_caps: { per_day_usd: 0.1, per_workflow_usd: 0.05 } }))),
    // A delegate may not delegate on.
    batch(plan({ id: 'a', ...good }), { env: { AGENT_KIT_DELEGATION: '1' } }),
  ]

  // A delegation that fails is retried on the same model with the failure in its prompt, and a second
  // failure climbs to the next stronger permitted model; the report lists every attempt with its run id,
  // client, model and status, and the ledger carries a line per attempt.
  // Determinism: one delegation in the first step, and the dependent of the second waits for the climb.
  scenarios['batch-attempts'] = [
    batch(plan({ client: 'vibe', id: 'retry', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Make it work.' }), merge(
      clients('vibe'),
      behaves('vibe', { err: 'first failure text', exit: 1 }, 1),
      behaves('vibe', { exit: 0, export: vibeExport(), out: journal('Second try.\nSUMMARY: it works now') }, 2),
    )),
    batch(plan(
      { client: 'vibe', id: 'climb', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Make it work.' },
      { client: 'vibe', depends_on: ['climb'], id: 'after', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Use the climb.' },
    ), merge(
      behaves('vibe', { err: 'first failure text', exit: 1 }, 3),
      behaves('vibe', { err: '', exit: 2, out: journal('Second try.') }, 4),
      behaves('vibe', { exit: 0, export: vibeExport(), out: journal('Climbed.\nSUMMARY: on the stronger model') }, 5),
      behaves('vibe', { exit: 0, export: vibeExport(), out: journal('Used it.\nSUMMARY: after the climb') }, 6),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // A client that cannot start: the delegation is `unavailable`, its summary says why per candidate, and
  // the delegation that waits for it is skipped.
  // Determinism: `after` depends on `gone`, so nothing runs beside it.
  scenarios['batch-unavailable'] = [
    batch(plan(
      read('gone', { prompt: 'Cannot start.' }),
      read('after', { depends_on: ['gone'], prompt: 'After it.' }),
    ), clients('vibe')),
  ]

  // Metered-personal delegations are estimated and reserved together before the first one launches; a
  // delegation that never runs (skipped, because the one it waits for failed) has its reservation given
  // back with reason "not-run", and the result says how much that was. A plan whose delegations all run
  // settles every reservation at what its run cost, and gives nothing back.
  // Determinism: `after` depends on `first`, so the two never overlap.
  scenarios['batch-caps'] = [
    batch(plan(
      { client: 'hermes', escalate: false, id: 'first', mode: 'read-only', model: 'claude-sonnet-4.6', prompt: 'Estimate me.', provider: 'anthropic' },
      { client: 'hermes', depends_on: ['first'], id: 'after', mode: 'read-only', model: 'claude-sonnet-4.6', prompt: 'Then me.', provider: 'anthropic' },
    ), merge(
      clients('hermes'),
      hermesMachine,
      behaves('hermes', { out: 'It broke.\nSUMMARY: no', usage: hermesUsage({ failed: true }) }, 1),
    )),
    batch(plan(
      { client: 'hermes', id: 'one', mode: 'read-only', model: 'claude-sonnet-4.6', prompt: 'One.', provider: 'anthropic' },
      { client: 'hermes', depends_on: ['one'], id: 'two', mode: 'read-only', model: 'claude-sonnet-4.6', prompt: 'Two.', provider: 'anthropic' },
    ), merge(
      behaves('hermes', { out: 'Done.\nSUMMARY: one is done', usage: hermesUsage() }, 2),
      behaves('hermes', { out: 'Done.\nSUMMARY: two is done', usage: hermesUsage({ estimated_cost_usd: 0.004 }) }, 3),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // A metered-employer delegation runs under the workflow's grant: the plan checks the grant before
  // anything launches, and each delegation's reservation is held against the grant's cap.
  // Determinism: `after` depends on `first`, so the two never overlap.
  scenarios['batch-grant'] = [
    batch(plan(
      { client: 'codex', id: 'first', mode: 'read-only', model: 'gpt-5.5', prompt: 'Check the repository.', repo: REPO },
      { client: 'codex', depends_on: ['first'], id: 'after', mode: 'read-only', model: 'gpt-5.5', prompt: 'And again.', repo: REPO },
    ), merge(
      clients('codex'),
      repo('openai'),
      grants(grant({ cap_usd: 5 })),
      behaves('codex', { out: codexOut() }, 1),
      behaves('codex', { out: codexOut('Again.') }, 2),
    )),
  ]

  // Every step snapshots the temporary directory and the repository's gateway directories besides the usual
  // roots, as the `run` scenarios do.
  for (const name of Object.keys(scenarios)) {
    if (name.startsWith('batch-')) scenarios[name] = scenarios[name].map((one) => ({ ...one, roots: ROOTS }))
  }
}
