// The scenarios for `grant reserve`: one Codex node start held against a workflow's grant. The grants
// live in the machine configuration, the ledger in the state home, and the clock is fixed per step.
// PATH is the sandbox's bin/ alone, with the `git` shim the grants reader asks. Imported by
// gen-scenarios.mjs, which writes them out.

const SBX = '@SANDBOX@'
const ENV_BASE = { XDG_CACHE_HOME: `${SBX}/cache` }
const GRANTS = 'home/.config/agent-kit/model-gateway-grants.json'
const LEDGER = 'state/agent-kit/model-gateway/ledger.jsonl'
const WORKFLOWS = 'state/agent-kit/model-gateway/workflows.json'
const OVERRIDE = 'home/.config/agent-kit/model-gateway.json'
const NOW = '2026-11-03T12:00:00.000Z'

export function addReserveScenarios(scenarios, step) {
  /** A step whose PATH is the sandbox's bin/, at its own instant. */
  const on = (argv, extra = {}) => step(argv, { path: ['bin'], ...extra, env: { ...ENV_BASE, ...(extra.env ?? {}) } })
  const text = (input) => (typeof input === 'string' ? input : JSON.stringify(input))
  const reserveStep = (input, extra) => on(['grant', 'reserve', '--json', text(input)], extra)
  const grantStep = (verb, input, extra) => on(['grant', verb, '--json', text(input)], extra)
  /** A start of node `node` in workflow `w` for codex, with the input fields `fields` and the step's `extra`. */
  const start = (node, fields = {}, extra = {}) => reserveStep({ workflow: 'w', client: 'codex', node, ...fields }, extra)

  const grant = (o) => ({ id: 'g1', workflow: 'w', client: 'codex', cap_usd: 1, question: 'q', answer: 'a', written_at: NOW, expires_at: '2026-11-04T12:00:00Z', ...o })
  const grantsFile = (value) => ({ [GRANTS]: text(value) })
  const grants = (...list) => grantsFile({ version: 1, grants: list })
  const workflowsFile = (...list) => ({ [WORKFLOWS]: text({ version: 1, workflows: list }) })
  const override = (value) => ({ [OVERRIDE]: text(value) })

  const run = (o) =>
    JSON.stringify({
      at: '2026-11-03T10:00:00.000Z',
      client: 'codex',
      cost_class: 'metered-employer',
      cost_usd: 0,
      duration_s: 3,
      family: 'openai',
      mode: 'read-only',
      model: 'gpt-5.5',
      outcome: 'ok',
      repository: null,
      run_id: 'r',
      workflow: 'w',
      ...o,
    })
  const reservation = (o) =>
    JSON.stringify({
      kind: 'reservation',
      run_id: 'res',
      workflow: 'w',
      client: 'codex',
      cost_class: 'metered-employer',
      model: 'node-start',
      estimate_usd: 0,
      reserved_at: '2026-11-03T11:00:00.000Z',
      expires_at: '2026-11-03T13:00:00.000Z',
      run_dir: `${SBX}/work/runs/res`,
      ...o,
    })
  const settled = (o) =>
    JSON.stringify({
      kind: 'settled',
      run_id: 'old',
      workflow: 'w',
      client: 'codex',
      cost_class: 'metered-employer',
      cost_usd: 0,
      reserved_at: '2026-11-03T09:00:00.000Z',
      settled_at: '2026-11-03T09:30:00.000Z',
      ...o,
    })
  const ledger = (...lines) => ({ [LEDGER]: `${lines.join('\n')}\n` })

  const gitDirs = (name) => [`work/${name}/.git/objects`, `work/${name}/.git/refs/heads`]
  const gitFiles = (name, config) => ({ [`work/${name}/.git/HEAD`]: 'ref: refs/heads/main\n', [`work/${name}/.git/config`]: config })

  // The cap fills one start at a time, a repeat of a node and attempt holds the one start it already
  // took, and the ledger line the start writes is the reservation the next start counts.
  scenarios['reserve-happy'] = [
    on(['grant', 'write', '--max-starts', '3', '--json', text({ workflow: 'w', client: 'codex', cap_usd: 5, question: 'may I?', answer: 'yes' })], { shims: ['git'] }),
    start('codex-1'),
    start('codex-2'),
    start('codex-3'),
    start('codex-4'),
    start('codex-1'),
    start('codex-2', { attempt: 1 }),
    start('codex-1', { attempt: 2 }),
  ]

  // A node and attempt that ask again hold the one start; every other attempt is its own start, and the
  // run id is a hash of the four fields, so a large attempt is a start of its own too.
  scenarios['reserve-attempts'] = [
    start('codex-1', {}, { write: grants(grant({ max_starts: 5 })), shims: ['git'] }),
    start('codex-1', { attempt: 2 }),
    start('codex-1', { attempt: 1 }),
    start('codex-1', { attempt: 0 }),
    start('codex-1', { attempt: 1e21 }),
    reserveStep('{"workflow":"w","client":"codex","node":"codex-1","attempt":9007199254740993}'),
    start('codex-1', { attempt: null }),
    start('codex-1', { attempt: 6 }),
  ]

  // A start holds for about a minute and then settles, and a settled start still counts: a start is
  // never given back.
  scenarios['reserve-settled'] = [
    start('codex-1', {}, { write: grants(grant({ max_starts: 2 })), shims: ['git'] }),
    start('codex-2', {}, { now: '2026-11-03T12:01:01Z' }),
    start('codex-3', {}, { now: '2026-11-03T12:01:02Z' }),
    start('codex-1', {}, { now: '2026-11-03T12:01:03Z' }),
    start('codex-2', {}, { now: '2026-11-03T12:02:03Z' }),
  ]

  // Every way a grant refuses a start: none at all, none with max_starts, one that lapsed, one whose
  // workflow ended, and the newest of several.
  scenarios['reserve-refusals'] = [
    start('codex-1', {}, { shims: ['git'] }),
    start('codex-1', {}, { write: grants(grant({})) }),
    start('codex-1', {}, { write: grants(grant({ max_starts: 3, ended_at: '2026-11-03T11:30:00Z' })) }),
    start('codex-1', {}, { write: grants(grant({ max_starts: 3, expires_at: '2026-11-03T11:00:00Z' })) }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 3 })), ...workflowsFile({ name: 'w', started_at: '2026-11-03T09:00:00Z', ended_at: '2026-11-03T10:00:00Z' }) } }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 3 })), ...workflowsFile({ name: 'w', started_at: '2026-11-02T11:00:00Z' }) } }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 5, written_at: '2026-11-03T11:00:00.000Z' }), grant({ id: 'g2', max_starts: 1, written_at: '2026-11-03T11:30:00.000Z' })), ...workflowsFile({ name: 'w', started_at: '2026-11-03T09:00:00Z' }) } }),
    start('codex-1', {}, { write: grants(grant({ max_starts: 1 }), grant({ id: 'g2', max_starts: 4 })) }),
    start('codex-1', {}, { write: grants(grant({ max_starts: 1, written_at: 'yesterday' })) }),
    start('codex-1', {}, { write: grants(grant({ workflow: 'other', max_starts: 3 })) }),
    start('codex-1', {}, { write: grants(grant({ client: 'hermes', max_starts: 3 })) }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 1 })), ...ledger(reservation({ run_id: 'before', reserved_at: '2026-11-03T11:00:00.000Z' })) } }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 1, cap_usd: 1 })), ...ledger(run({ run_id: 'spent', cost_usd: 1, at: '2026-11-03T11:30:00.000Z' })) } }),
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 1 })), ...ledger(reservation({ run_id: 'other-workflow', workflow: 'other' }), reservation({ run_id: 'other-client', client: 'hermes' }), reservation({ run_id: 'other-model', model: 'gpt-5.5' })) } }),
  ]

  // What a start must name, and what it may carry: the node, the attempt, the client and the workflow
  // are read before any grant is, and a node name is free text.
  const longNode = 'n'.repeat(200)
  scenarios['reserve-input'] = [
    reserveStep({}, { shims: ['git'] }),
    reserveStep({ node: '' }),
    reserveStep({ node: '   ' }),
    reserveStep({ node: 5 }),
    reserveStep({ node: 'n', attempt: -1 }),
    reserveStep({ node: 'n', attempt: 1.5 }),
    reserveStep({ node: 'n', attempt: '1' }),
    reserveStep({ node: 'n', attempt: true }),
    reserveStep('{"node":"n","attempt":1e999}'),
    reserveStep({ node: 'n' }),
    reserveStep({ node: 'n', client: '' }),
    reserveStep({ node: 'n', client: 5 }),
    reserveStep({ node: 'n', client: 'codex' }),
    reserveStep({ node: 'n', client: 'codex', workflow: 'bad name' }),
    reserveStep([]),
    reserveStep(null),
    reserveStep('"x"'),
    reserveStep({ workflow: 'w', client: 'nobody', node: 'n' }),
    reserveStep({ workflow: 'w', client: 'claude', node: 'n' }),
    reserveStep({ workflow: 'w', client: 'hermes', node: 'n' }),
    reserveStep({ workflow: 'w', client: 'codex', node: 'codex-1', ignored: true, attempt: 1 }, { write: grants(grant({ max_starts: 3 })) }),
    start('ünïcode-✓ "quoted" \\ back'),
    start(longNode),
    reserveStep({ workflow: 'a/b:c@d', client: 'codex', node: 'codex-1' }, { write: grants(grant({ workflow: 'a/b:c@d', max_starts: 2 })) }),
  ]

  // The files a start reads and writes: a ledger with lines it cannot read, a ledger that is not there
  // or is a directory, a grants file that is corrupt or a directory, and the reservations a start
  // settles on its way.
  scenarios['reserve-files'] = [
    start('codex-1', {}, {
      write: {
        ...grants(grant({ max_starts: 2 })),
        ...ledger(
          '{"torn":',
          'not json at all',
          '[]',
          reservation({ run_id: 'before', reserved_at: '2026-11-03T11:00:00.000Z' }),
          run({ run_id: 'ran', at: '2026-11-03T11:30:00.000Z' }),
        ),
      },
      shims: ['git'],
    }),
    start('codex-2', {}, { remove: [LEDGER] }),
    start('codex-3', {}, { remove: [LEDGER], mkdir: [LEDGER] }),
    start('codex-1', {}, { remove: [LEDGER], write: grantsFile('nope') }),
    start('codex-1', {}, { write: grantsFile({ version: 2, grants: [] }) }),
    start('codex-1', {}, { remove: [GRANTS], mkdir: [GRANTS] }),
    start('codex-1', {}, { remove: [GRANTS], write: { ...grants(grant({ max_starts: 2 })), ...ledger(reservation({ run_id: 'dead', expires_at: '2026-11-03T11:30:00.000Z' })) } }),
    start('codex-1', {}, {
      write: {
        ...grants(grant({ max_starts: 2 })),
        ...ledger(reservation({ run_id: 'done', run_dir: `${SBX}/work/runs/done` })),
        'work/runs/done/result.json': '{"cost_usd":0.5}',
      },
    }),
    start('codex-1', {}, { env: { XDG_STATE_HOME: null } }),
    start('codex-1', {}, { env: { XDG_STATE_HOME: 'relative/state' } }),
    start('codex-1', {}, { env: { HOME: null } }),
    start('codex-1', {}, { env: { XDG_CONFIG_HOME: `${SBX}/work/repo/conf` }, write: { ...gitFiles('repo', '[core]\n'), ...grants(grant({ max_starts: 2 })) }, mkdir: gitDirs('repo') }),
    start('codex-1', {}, { env: { XDG_CONFIG_HOME: `${SBX}/work/repo/conf/deeper/still` } }),
    start('codex-1', {}, { remove: ['bin/git'] }),
    start('codex-1', {}, { env: { XDG_CONFIG_HOME: `${SBX}/elsewhere` }, shims: ['git'] }),
  ]

  // The cost class a start checks is the machine's own: an override that makes codex subsidised refuses
  // the start, and one that declares another metered-employer client lets it hold a start.
  const custom = { custom: { binary: 'custom', cost_class: 'metered-employer', model_source: 'declared' } }
  scenarios['reserve-override'] = [
    start('codex-1', {}, { write: { ...grants(grant({ max_starts: 2 })), ...override({ clients: { codex: { cost_class: 'subsidised' } } }) }, shims: ['git'] }),
    start('codex-1', {}, { write: override({ clients: { codex: { cost_class: 'metered-personal' } } }) }),
    start('codex-1', {}, { write: override('not json') }),
    reserveStep({ workflow: 'w', client: 'custom', node: 'codex-1' }, { write: { ...grants(grant({ client: 'custom', max_starts: 2 })), ...override({ clients: custom }) } }),
    reserveStep({ workflow: 'w', client: 'custom', node: 'codex-2' }),
  ]
}
