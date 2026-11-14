// The scenarios for `run` (read-only delegations, every client): fake client programs under bin/ that
// record how they were launched (argv, working directory, the environment that matters, stdin) and
// print canned output in the client's own shape. PATH is the sandbox's bin/ alone. Imported by
// gen-scenarios.mjs, which writes them out; regenerate-fixtures.mjs makes the goldens with the node runner.
// The helpers below (the fake programs, the canned behaviour, the repositories, the grants) are exported
// for gen-scenarios-write.mjs, which drives the same fakes through a writing run.
//
// How a fake client is told what to do: every program is the same script (fake()). It counts its own
// calls in home/calls/<client>.n and appends what it saw to home/calls/<client>.txt. What it prints
// and does comes from files under home/canned/, picked per call: `<client>.<kind>.<call number>` when
// that file exists, else `<client>.<kind>`. The kinds are `out` (stdout), `err` (stderr), `exit` (the
// exit code), `export` (Vibe's export.json, written to its --output-dir), `usage` (Hermes' usage report,
// written to its --usage-file), `sleep` (the program execs `sleep 30`, so a time limit stops it) and
// `act` (a shell snippet the program sources, to do something to the sandbox). Its status commands
// (`login status`, `auth list`) answer that it is signed in, are logged as `status ...` and are not calls.
//
// Run ids and temporary directories carry random parts, which regenerate-fixtures.mjs numbers (@RAND1@,
// @TMP1@). Time is fixed (and so is the monotonic reading, so a run's duration is always 0).

export const SBX = '@SANDBOX@'
export const ENV_BASE = { XDG_CACHE_HOME: `${SBX}/cache`, TMPDIR: `${SBX}/tmp` }
const CONFIG = 'home/.config/agent-kit'
const GRANTS = `${CONFIG}/model-gateway-grants.json`
const OVERRIDE = `${CONFIG}/model-gateway.json`
const ALLOWLIST = `${CONFIG}/critique-allowlist.json`
const ROUTING_OVERRIDE = `${CONFIG}/critique-routing.json`
const SETUP_RESULTS = 'home/.local/share/agent-kit/critique/machine-setup.json'
const STATE = 'state/agent-kit/model-gateway'
const LEDGER = `${STATE}/ledger.jsonl`
const WORKFLOWS = `${STATE}/workflows.json`
const LITELLM_CATALOGUE = 'cache/agent-kit/litellm-catalogue.json'
export const REPO = `${SBX}/repo`
export const NOW = '2026-11-03T12:00:00.000Z'
export const SHIMS = ['git', 'env', 'cat', 'mkdir', 'sleep', 'rm', 'mv']
export const ROOTS = ['tmp', 'repo/.git/model-gateway']

/** The variables a fake client reports on, each as its value or `<unset>`. */
const WATCHED_ENV = [
  'AGENT_KIT_DELEGATION',
  'MISTRAL_API_KEY',
  'VIBE_HOME',
  'VIBE_ENABLE_MCP',
  'VIBE_ENABLE_SUBAGENTS',
  'VIBE_INCLUDE_PROJECT_CONTEXT',
  'VIBE_OUTSIDE_WORKDIR',
  'VIBE_MCP_SERVERS',
]

/** One fake client program. `stdin` makes it keep what it was sent on stdin, as the client that reads its prompt there. */
export function fake(name, stdin) {
  const watched = WATCHED_ENV.map((variable) => `  echo "env ${variable}=\${${variable}-<unset>}"`).join('\n')
  return `#!/bin/sh
D="$HOME/calls"
C="$HOME/canned"
mkdir -p "$D"
case "$1" in
  login|auth) echo "status $*" >> "$D/${name}.txt"; echo "signed in with an api key"; exit 0 ;;
esac
n=0
[ -f "$D/${name}.n" ] && read n < "$D/${name}.n"
n=$((n + 1))
echo "$n" > "$D/${name}.n"
pick() { if [ -f "$C/${name}.$1.$n" ]; then echo "$C/${name}.$1.$n"; else echo "$C/${name}.$1"; fi; }
{
  echo "call $n"
  echo "cwd $(pwd -P)"
  for a in "$@"; do echo "arg $a"; done
${watched}
} >> "$D/${name}.txt"
${stdin ? `cat > "$D/${name}.$n.stdin"` : ''}
prev=
for a in "$@"; do
  case "$prev" in
    --output-dir) outdir=$a ;;
    --usage-file) usage=$a ;;
  esac
  prev=$a
done
f=$(pick act)
[ -f "$f" ] && . "$f"
f=$(pick export)
if [ -n "$outdir" ] && [ -f "$f" ]; then mkdir -p "$outdir"; cat "$f" > "$outdir/export.json"; fi
f=$(pick usage)
if [ -n "$usage" ] && [ -f "$f" ]; then cat "$f" > "$usage"; fi
f=$(pick sleep)
[ -f "$f" ] && exec sleep 30
f=$(pick out)
[ -f "$f" ] && cat "$f"
f=$(pick err)
[ -f "$f" ] && cat "$f" >&2
ex=0
f=$(pick exit)
[ -f "$f" ] && read ex < "$f"
exit "$ex"
`
}

/** The canned behaviour of one client for one call (every call when `n` is left out). */
export function canned(client, spec, n) {
  const at = n === undefined ? '' : `.${n}`
  const write = {}
  for (const [kind, content] of Object.entries(spec)) write[`home/canned/${client}.${kind}${at}`] = `${content}\n`
  return write
}

export const journal = (text) => JSON.stringify([{ type: 'message', role: 'assistant', content: [{ type: 'text', text }] }])
export const vibeExport = (extra = {}) => JSON.stringify({ usage: { input_tokens: 1000, output_tokens: 200, cached_input_tokens: 400 }, cost_usd: 0.0123, ...extra })
export const agyOut = (extra = {}) => JSON.stringify({ response: 'Read the files.\nSUMMARY: agy is fine', status: 'SUCCESS', usage: { input_tokens: 900, output_tokens: 90, cache_read_tokens: 300 }, ...extra })
export const claudeOut = (extra = {}) =>
  JSON.stringify({ type: 'result', result: 'Reviewed.\nSUMMARY: claude is fine', total_cost_usd: 0.05, usage: { input_tokens: 10, cache_read_input_tokens: 20, cache_creation_input_tokens: 5, output_tokens: 30 }, ...extra })
export const opencodeOut = (text = 'Looked it up.\nSUMMARY: opencode is fine') =>
  [
    { type: 'step_start' },
    { type: 'text', part: { text } },
    { type: 'step_finish', part: { cost: 0.004, tokens: { input: 100, output: 40, reasoning: 10, cache: { read: 30, write: 5 } } } },
  ]
    .map((event) => JSON.stringify(event))
    .join('\n')
export const hermesUsage = (extra = {}) => JSON.stringify({ input_tokens: 2000, output_tokens: 300, cache_read_tokens: 500, estimated_cost_usd: 0.002, failed: false, ...extra })
export const codexOut = (text = 'Checked.\nSUMMARY: codex is fine') =>
  [
    { type: 'thread.started', thread_id: 't' },
    { type: 'item.completed', item: { type: 'agent_message', text } },
    { type: 'turn.completed', usage: { input_tokens: 5000, cached_input_tokens: 1000, output_tokens: 400 } },
  ]
    .map((event) => JSON.stringify(event))
    .join('\n')

export const text = (input) => (typeof input === 'string' ? input : JSON.stringify(input))

/** Several step parts as one: files and environment merge, lists concatenate. */
export function merge(...parts) {
  const out = {}
  for (const part of parts) {
    for (const [key, value] of Object.entries(part ?? {})) {
      if (key === 'write' || key === 'env') out[key] = { ...out[key], ...value }
      else if (Array.isArray(value)) out[key] = [...(out[key] ?? []), ...value]
      else out[key] = value
    }
  }
  return out
}

/** The step builders a `run` scenario uses: a step whose PATH is the sandbox's bin/, at its own instant. */
export function runSteps(step) {
  const on = (argv, extra = {}) => step(argv, { path: ['bin'], ...extra, env: { ...ENV_BASE, ...(extra.env ?? {}) } })
  const run = (input, extra) => on(['run', '--json', text(typeof input === 'string' ? input : { workflow: 'w', ...input })], extra)
  const modelsStep = (input, extra) => on(['models', '--json', text(input)], extra)
  return { modelsStep, on, run }
}

/** The fake client programs, the tools they use, and the temporary directory of a first step. */
export const clients = (...names) => ({
  write: Object.fromEntries(names.map((name) => [`bin/${name}`, fake(name, name === 'claude' || name === 'codex')])),
  exec: names.map((name) => `bin/${name}`),
  shims: SHIMS,
  mkdir: ['tmp'],
})
/** What a client does from now on; a default (no call number) first forgets the earlier defaults. */
export const KINDS = ['out', 'err', 'exit', 'export', 'usage', 'sleep', 'act']
export const behaves = (client, spec, n) => ({
  ...(n === undefined ? { remove: KINDS.map((kind) => `home/canned/${client}.${kind}`) } : {}),
  write: canned(client, spec, n),
})
export const file = (path, value) => ({ write: { [path]: text(value) } })
export const override = (value) => file(OVERRIDE, value)
export const allowlist = (...families) =>
  file(ALLOWLIST, { version: 1, repositories: { 'github.com/jorisjonkers-dev/demo': { families, code: true, history: false } } })
export const grant = (o = {}) => ({ id: 'g1', workflow: 'w', client: 'codex', cap_usd: 1, question: 'q', answer: 'a', written_at: NOW, expires_at: '2026-11-04T12:00:00Z', ...o })
export const grants = (...list) => file(GRANTS, { version: 1, grants: list })
export const workflows = (...list) => file(WORKFLOWS, { version: 1, workflows: list })
export const ledgerLines = (...lines) => ({ write: { [LEDGER]: `${lines.join('\n')}\n` } })
export const spent = (o) =>
  JSON.stringify({
    at: '2026-11-03T10:00:00.000Z',
    client: 'hermes',
    cost_class: 'metered-personal',
    cost_usd: 1,
    duration_s: 3,
    family: 'anthropic',
    mode: 'read-only',
    model: 'claude-sonnet-4.6',
    outcome: 'ok',
    repository: null,
    run_id: 'earlier',
    workflow: 'w',
    ...o,
  })
/** A repository of its own with one commit and an allowlist entry for these families. */
export const repo = (...families) =>
  merge(
    {
      write: { 'repo/README.md': 'demo\n' },
      git: [
        ['init', '-q', '-b', 'main', REPO],
        ['-C', REPO, 'remote', 'add', 'origin', 'https://github.com/JorisJonkers-dev/demo.git'],
        ['-C', REPO, 'add', '-A'],
        ['-C', REPO, 'commit', '-q', '-m', 'init'],
      ],
    },
    allowlist(...families),
  )
/** The proof a model needs to work in the place itself: a critic client of the machine's routing that runs it, and setup's passing result. */
export const proven = (kind, model, result = 'pass') =>
  merge(
    file(ROUTING_OVERRIDE, { clients: { [`${kind}-proof`]: { kind, model } } }),
    file(SETUP_RESULTS, { checks: { [`read-only:${kind}-proof`]: { result } } }),
  )
/** The machine override that prices Hermes' model; `settings` adds settings of its own (a batch plan's caps, say). */
export const hermesMachineWith = (settings) =>
  override({
    clients: { hermes: { providers: ['anthropic'] } },
    models: [{ client: 'hermes', model: 'claude-sonnet-4.6', family: 'anthropic', tier: 'routine', notes: 'x', prices: { input: 3, cached_input: 0.3, output: 15 } }],
    ...(settings === undefined ? {} : { settings }),
  })
export const hermesMachine = merge(hermesMachineWith())
export const VIBE_ISOLATION = { MISTRAL_API_KEY: 'study-account-key', VIBE_HOME: `${SBX}/home/.vibe-private` }

export function addRunScenarios(scenarios, step) {
  const { modelsStep, run } = runSteps(step)
  const ok = (client) => behaves(client, {})

  // Each client's happy path: the launch it gets (flags, prompt, working directory, environment), the
  // files the run leaves (prompt, input, output, result), the ledger line, and the workflow it started.
  scenarios['run-vibe'] = [
    run({ client: 'vibe', mode: 'read-only', model: 'mistral-small-latest', prompt: 'Review the plan.' }, merge(
      clients('vibe'),
      behaves('vibe', { out: journal('The plan reads well.\nSUMMARY: plan is fine'), export: vibeExport() }),
      { env: VIBE_ISOLATION },
    )),
    run({ client: 'vibe', mode: 'read-only', model: 'deepseek-v4-1-flash-1017', prompt: 'Priced by tokens.', time_limit_s: 120, max_price_usd: 0.5 }, merge(
      behaves('vibe', { out: journal('No dollars given.\nSUMMARY: priced from the catalogue'), export: JSON.stringify({ usage: { input_tokens: 2000, output_tokens: 500, cached_input_tokens: 1000 } }) }),
      { env: VIBE_ISOLATION, now: '2026-11-03T12:05:00Z' },
    )),
    run({ client: 'vibe', mode: 'read-only', model: 'mistral-medium-3.5', prompt: 'A model with no prices.', carries: { code: false, history: false } }, merge(
      behaves('vibe', { out: journal('Free.'), export: '{}' }),
      { now: '2026-11-03T12:10:00Z' },
    )),
    run({ client: 'vibe', mode: 'read-only', model: 'glm-5-3', prompt: 'Long summary.' }, merge(
      behaves('vibe', { out: journal(`${'long '.repeat(400)}`), export: vibeExport({ cost_usd: 0 }) }),
      { now: '2026-11-03T12:15:00Z' },
    )),
  ]

  scenarios['run-agy'] = [
    run({ client: 'agy', mode: 'read-only', model: 'gemini-3.8-flash-high', prompt: 'Read the long document.' }, merge(clients('agy'), behaves('agy', { out: agyOut() }))),
    run({ client: 'agy', mode: 'read-only', model: 'gemini-3.8-flash-low', prompt: 'Short one.' }, merge(behaves('agy', { out: agyOut({ response: 'No summary line here.' }) }), { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'agy', mode: 'read-only', model: 'gemini-3.8-flash-medium', prompt: 'Stream.', time_limit_s: 30 }, merge(
      behaves('agy', { out: `${JSON.stringify({ event: 'init' })}\n${JSON.stringify({ event: 'result', result: JSON.parse(agyOut()) })}` }),
      { now: '2026-11-03T12:10:00Z' },
    )),
  ]

  scenarios['run-claude'] = [
    run({ client: 'claude', mode: 'read-only', model: 'claude-haiku-5-5', prompt: 'Summarise the diff.' }, merge(clients('claude'), behaves('claude', { out: claudeOut() }))),
    run({ client: 'claude', mode: 'read-only', model: 'claude-sonnet-5-5', prompt: 'List of events.' }, merge(
      behaves('claude', { out: JSON.stringify([{ type: 'system' }, JSON.parse(claudeOut({ total_cost_usd: 0 }))]) }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  scenarios['run-opencode'] = [
    run({ client: 'opencode', mode: 'read-only', model: 'opencode-go/glm-5.3-flash', prompt: 'Look it up.' }, merge(clients('opencode'), behaves('opencode', { out: opencodeOut() }))),
    run({ client: 'opencode', mode: 'read-only', model: 'opencode-go/kimi-k3.1-pro', prompt: 'A second one.', time_limit_s: 600 }, merge(
      behaves('opencode', { out: opencodeOut('Second.') }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  scenarios['run-hermes'] = [
    run({ client: 'hermes', mode: 'read-only', model: 'claude-sonnet-4.6', provider: 'anthropic', prompt: 'Ask Hermes.' }, merge(
      clients('hermes'),
      hermesMachine,
      behaves('hermes', { out: 'Hermes answers.\nSUMMARY: hermes is fine', usage: hermesUsage() }),
    )),
    run({ client: 'hermes', mode: 'read-only', model: 'claude-sonnet-4.6', provider: 'anthropic', prompt: 'No usage report.' }, merge(
      behaves('hermes', { out: 'Answer without a report.', usage: 'not json' }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  scenarios['run-codex'] = [
    run({ client: 'codex', mode: 'read-only', model: 'gpt-5.5', prompt: 'Check the repository.', repo: REPO, carries: { code: true, history: false } }, merge(
      clients('codex'),
      repo('openai'),
      grants(grant({ cap_usd: 5 })),
      behaves('codex', { out: codexOut() }),
    )),
    run({ client: 'codex', mode: 'read-only', model: 'gpt-5.5', prompt: 'And again.', repo: REPO, time_limit_s: 300 }, merge(
      behaves('codex', { out: codexOut('Again.') }),
      { now: '2026-11-03T12:05:00Z' },
    )),
  ]

  // Every way a launched run fails, one result each (no escalation): the reason, the files the failed run
  // leaves, and the ledger line it writes.
  scenarios['run-failures'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, prompt: 'Exits non-zero.' }, merge(
      clients('claude', 'opencode', 'agy', 'codex', 'hermes', 'vibe'),
      hermesMachine,
      behaves('claude', { out: 'partial', err: 'it broke', exit: 3 }),
    )),
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, prompt: 'Prints nothing.' }, merge(
      behaves('claude', { out: '' }), { now: '2026-11-03T12:01:00Z' })),
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, prompt: 'Claude reports an error.' }, merge(
      behaves('claude', { out: claudeOut({ is_error: true, result: 'the tool crashed' }) }), { now: '2026-11-03T12:02:00Z' })),
    run({ client: 'agy', model: 'gemini-3.8-flash-low', mode: 'read-only', escalate: false, prompt: 'Agy is denied.' }, merge(
      behaves('agy', { out: agyOut({ denied_actions: ['write_file'] }) }), { now: '2026-11-03T12:03:00Z' })),
    run({ client: 'agy', model: 'gemini-3.8-flash-low', mode: 'read-only', escalate: false, prompt: 'Agy times out itself.' }, merge(
      behaves('agy', { out: agyOut({ finished_reason: 'print_timeout' }) }), { now: '2026-11-03T12:04:00Z' })),
    run({ client: 'agy', model: 'gemini-3.8-flash-low', mode: 'read-only', escalate: false, prompt: 'Agy fails.' }, merge(
      behaves('agy', { out: agyOut({ status: 'ERROR' }) }), { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', escalate: false, prompt: 'Hermes fails.' }, merge(
      behaves('hermes', { out: 'Sorry.', usage: hermesUsage({ failed: true }) }), { now: '2026-11-03T12:06:00Z' })),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', escalate: false, prompt: 'Codex fails.', repo: REPO }, merge(
      repo('openai'),
      grants(grant({ cap_usd: 5 })),
      behaves('codex', { out: `${JSON.stringify({ type: 'turn.failed', error: { message: 'the sandbox refused' } })}` }),
      { now: '2026-11-03T12:07:00Z' })),
    run({ client: 'opencode', model: 'opencode-go/glm-5.3-flash', mode: 'read-only', escalate: false, prompt: 'Opencode errors.' }, merge(
      behaves('opencode', { out: JSON.stringify({ type: 'error', error: { message: 'something odd' } }) }), { now: '2026-11-03T12:08:00Z' })),
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', escalate: false, prompt: 'Vibe prints no journal.' }, merge(
      behaves('vibe', { out: 'not a journal', export: vibeExport() }), { now: '2026-11-03T12:09:00Z' })),
  ]

  scenarios['run-timeout'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, time_limit_s: 1, prompt: 'Too slow.' }, merge(
      clients('claude'), behaves('claude', { sleep: '1' }))),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', escalate: false, time_limit_s: 1, prompt: 'Codex too slow.', repo: REPO }, merge(
      clients('codex'), repo('openai'), grants(grant({ cap_usd: 5 })), behaves('codex', { sleep: '1' }), { now: '2026-11-03T12:05:00Z' })),
  ]

  // A client that says it is out of allowance, signed out, or that its account may not use the model: the
  // refusal reason, the reset it names (and the until-time the gateway keeps), and what `models` shows after.
  const probe = `#!/bin/sh
echo "Error: unknown model 'model-gateway-probe'. Available: mistral-small-latest, glm-5-3, deepseek-v4-1-flash-1017." >&2
exit 1
`
  scenarios['run-refusals-by-client'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, prompt: 'Rate limited.' }, merge(
      clients('claude', 'vibe', 'agy'),
      behaves('claude', { err: 'Error: rate limit exceeded. Resets in 2h30m.', exit: 1 }))),
    run({ client: 'agy', model: 'gemini-3.8-flash-low', mode: 'read-only', escalate: false, prompt: 'Quota.' }, merge(
      behaves('agy', { out: agyOut({ response: undefined, status: 'ERROR', error: { message: 'quota exceeded', status: 429 } }), exit: 0 }), { now: '2026-11-03T12:01:00Z' })),
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', escalate: false, prompt: 'Not signed in.' }, merge(
      behaves('vibe', { err: 'Error: 401 Unauthorized: invalid api key', exit: 1 }), { now: '2026-11-03T12:02:00Z' })),
    run({ client: 'claude', model: 'claude-sonnet-5-5', mode: 'read-only', escalate: false, prompt: 'Not logged in.' }, merge(
      behaves('claude', { out: claudeOut({ is_error: true, result: 'Not logged in. Please log in.' }), exit: 0 }), { now: '2026-11-03T12:03:00Z' })),
    run({ client: 'vibe', model: 'glm-5-3', mode: 'read-only', escalate: false, prompt: 'Hosted outside, named reset.' }, merge(
      behaves('vibe', { err: 'The model is hosted outside your region. Resets in 1h5m10s.', exit: 1 }), { now: '2026-11-03T12:04:00Z' })),
    run({ client: 'vibe', model: 'deepseek-v4-1-flash-1017', mode: 'read-only', escalate: false, prompt: 'Hosted outside, no reset.' }, merge(
      behaves('vibe', { err: 'deepseek-v4-1-flash-1017 is hosted outside your region', exit: 1 }), { now: '2026-11-03T12:05:00Z' })),
    modelsStep({}, merge(
      { write: { 'bin/vibe': probe, 'bin/agy': '#!/bin/sh\nexit 0\n' } },
      { now: '2026-11-03T12:06:00Z' })),
  ]

  scenarios['run-launch-failed'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', escalate: false, prompt: 'Cannot start.' }, merge(
      { mkdir: ['tmp', 'bin/claude'], shims: SHIMS })),
  ]

  // A failed attempt is retried on the same model with the failure in the prompt, then climbs to the next
  // stronger permitted model; the attempts link to the first.
  const prices = { mistral: 'mistral-small-latest' }
  scenarios['run-escalation-climb'] = [
    run({ client: 'vibe', model: prices.mistral, mode: 'read-only', prompt: 'Make it work.' }, merge(
      clients('vibe'),
      behaves('vibe', { err: 'first failure text', exit: 1 }),
      behaves('vibe', { out: journal('Second try.'), err: '', exit: 2 }, 2),
      behaves('vibe', { out: journal('Climbed.\nSUMMARY: on the stronger model'), export: vibeExport(), exit: 0 }, 3),
    )),
    run({ client: 'vibe', model: prices.mistral, mode: 'read-only', prompt: 'Never works.' }, merge(
      { remove: ['home/calls/vibe.n'], now: '2026-11-03T12:10:00Z' },
      behaves('vibe', { exit: 1 }, 3),
      behaves('vibe', { exit: 1 }, 1),
    )),
    run({ client: 'vibe', model: prices.mistral, mode: 'read-only', max_attempts: 2, prompt: 'Two attempts only.' }, merge(
      { remove: ['home/calls/vibe.n'], now: '2026-11-03T12:20:00Z' })),
    run({ client: 'vibe', model: prices.mistral, mode: 'read-only', max_attempts: 1, prompt: 'One attempt.' }, merge(
      { remove: ['home/calls/vibe.n'], now: '2026-11-03T12:30:00Z' })),
    run({ client: 'vibe', model: prices.mistral, mode: 'read-only', escalate: false, prompt: 'No escalation.' }, merge(
      { remove: ['home/calls/vibe.n'], now: '2026-11-03T12:40:00Z' })),
  ]

  // A client that cannot start is swapped for another of the same cost class; the family binds unless told
  // otherwise; when nothing can stand in the delegation is `unavailable`.
  scenarios['run-escalation-swap'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', prompt: 'Bound to the family.' }, merge(
      { mkdir: ['tmp'], shims: SHIMS },
      clients('vibe'))),
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', family_binding: false, prompt: 'Any family will do.' }, merge(
      behaves('vibe', { out: journal('Swapped.\nSUMMARY: answered by the swap'), export: vibeExport() }),
      { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', family_binding: false, escalate: false, prompt: 'No escalation, so the first attempt stays.' }, merge(
      { now: '2026-11-03T12:10:00Z' })),
  ]

  // An attempt that fails to start after a real one, and a climb with nowhere to go.
  scenarios['run-escalation-stopped'] = [
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', prompt: 'Fails, then the client is refused.' }, merge(
      clients('claude'),
      behaves('claude', { err: 'broke', exit: 1 }, 1),
      behaves('claude', { err: 'Error: rate limit exceeded', exit: 1 }, 2),
      behaves('claude', { err: 'Error: rate limit exceeded', exit: 1 }, 3),
      behaves('claude', { err: 'Error: rate limit exceeded', exit: 1 }, 4),
    )),
  ]

  // The strongest model of its family fails twice: there is nowhere to climb to.
  scenarios['run-escalation-top'] = [
    run({ client: 'claude', model: 'claude-opus-5-5', mode: 'read-only', prompt: 'The strongest model fails twice.' }, merge(
      clients('claude'),
      behaves('claude', { err: 'broke', exit: 1 }, 1),
      behaves('claude', { err: 'broke again', exit: 1 }, 2),
    )),
  ]

  // Metered-personal runs are estimated and held against the caps before they start, and settled after.
  scenarios['run-caps'] = [
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'Estimate me.' }, merge(
      clients('hermes'), hermesMachine,
      behaves('hermes', { out: 'Done.\nSUMMARY: ok', usage: hermesUsage() }))),
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'Per workflow.', workflow: 'busy' }, merge(
      ledgerLines(spent({ run_id: 'earlier', workflow: 'busy', cost_usd: 1.99 })),
      { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'Per day.', workflow: 'fresh' }, merge(
      ledgerLines(spent({ run_id: 'e1', workflow: 'a', cost_usd: 1.9 }), spent({ run_id: 'e2', workflow: 'b', cost_usd: 1.9 }), spent({ run_id: 'e3', workflow: 'c', cost_usd: 1.19 })),
      { now: '2026-11-03T12:10:00Z' })),
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'Yesterday does not count.', workflow: 'next' }, merge(
      ledgerLines(spent({ run_id: 'old', workflow: 'x', cost_usd: 4.99, at: '2026-11-02T10:00:00.000Z' })),
      { now: '2026-11-03T12:15:00Z' })),
    run({ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'A reservation that lapsed counts in full.', workflow: 'lapsed' }, merge(
      ledgerLines(JSON.stringify({
        kind: 'reservation', run_id: 'dead', workflow: 'lapsed', client: 'hermes', cost_class: 'metered-personal', model: 'claude-sonnet-4.6',
        estimate_usd: 1.99, reserved_at: '2026-11-03T10:00:00.000Z', expires_at: '2026-11-03T10:30:00.000Z', run_dir: `${SBX}/state/dead`,
      })),
      { now: '2026-11-03T12:20:00Z' })),
    // A model the catalogue prices nowhere cannot be estimated, so a metered run on it is refused.
    run({ client: 'hermes', model: 'claude-opus-5-5', provider: 'anthropic', mode: 'read-only', prompt: 'No price.' }, { now: '2026-11-03T12:25:00Z' }),
    run({ client: 'opencode', model: 'other/glm-5.3', mode: 'read-only', prompt: 'A metered provider of a subsidised client.' }, merge(
      clients('opencode'),
      override({
        clients: { hermes: { providers: ['anthropic'] }, opencode: { providers: ['opencode-go', 'other'] } },
        models: [
          { client: 'hermes', model: 'claude-sonnet-4.6', family: 'anthropic', tier: 'routine', notes: 'x', prices: { input: 3, cached_input: 0.3, output: 15 } },
          { client: 'opencode', model: 'other/glm-5.3', family: 'zhipu', tier: 'routine', notes: 'x', prices: { input: 1, cached_input: 0.1, output: 2 } },
        ],
      }),
      behaves('opencode', { out: opencodeOut() }),
      { now: '2026-11-03T12:30:00Z' })),
  ]

  // Metered-employer runs need a grant for the workflow; the run is held to the grant's cap and its time.
  scenarios['run-employer'] = [
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'No grant.', repo: REPO }, merge(
      clients('codex'), repo('openai'), behaves('codex', { out: codexOut() }))),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'A grant that ends in thirty seconds.', repo: REPO }, merge(
      grants(grant({ expires_at: '2026-11-03T12:00:30Z' })))),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'A grant that lapsed.', repo: REPO }, merge(
      grants(grant({ expires_at: '2026-11-03T12:00:30Z' })), { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'A grant too small.', repo: REPO }, merge(
      grants(grant({ cap_usd: 0.0001 })), { now: '2026-11-03T12:06:00Z' })),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'A grant that ends in ten minutes.', repo: REPO, time_limit_s: 900 }, merge(
      grants(grant({ cap_usd: 5, expires_at: '2026-11-03T12:16:00Z' })), { now: '2026-11-03T12:06:00Z' })),
    run({ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'The workflow ended.', repo: REPO }, merge(
      grants(grant({ cap_usd: 5 })),
      workflows({ name: 'w', started_at: '2026-11-03T09:00:00Z', ended_at: '2026-11-03T11:00:00Z' }),
      { now: '2026-11-03T12:07:00Z' })),
    // Another client of the escalation: a run that is not allowed to swap into the metered-employer class.
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', prompt: 'Never swaps into a costlier class.', family_binding: false, repo: REPO }, merge(
      allowlist('anthropic', 'openai'), { now: '2026-11-03T12:08:00Z' })),
  ]

  // What the run works in: the repository itself for a model whose read-only proof passed, a throwaway
  // worktree of it (removed after) for every other, a fresh empty directory with no repository.
  scenarios['run-workplace'] = [
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Unproven in a repository.', repo: REPO }, merge(
      clients('vibe'), repo('mistral', 'google'), behaves('vibe', { out: journal('ok\nSUMMARY: s'), export: vibeExport() }),
      {})),
    run({ client: 'vibe', model: 'mistral-medium-3.5', mode: 'read-only', prompt: 'Proven in a repository.', repo: REPO }, merge(
      proven('vibe', 'mistral-medium-3.5'), { now: '2026-11-03T12:05:00Z' })),
    run({ client: 'vibe', model: 'mistral-medium-3.5', mode: 'read-only', prompt: 'Proven, no repository.' }, merge(
      { now: '2026-11-03T12:10:00Z' })),
    run({ client: 'vibe', model: 'mistral-medium-3.5', mode: 'read-only', prompt: 'Setup says fail.', repo: REPO }, merge(
      proven('vibe', 'mistral-medium-3.5', 'fail'), { now: '2026-11-03T12:15:00Z' })),
    run({ client: 'vibe', model: 'glm-5-3', mode: 'read-only', prompt: 'Proven by a name of the catalogue.', repo: REPO }, merge(
      allowlist('mistral', 'zhipu'),
      file(ROUTING_OVERRIDE, { clients: { 'vibe-glm': { kind: 'vibe', model: 'zai-glm-5-3' } } }),
      file(SETUP_RESULTS, { checks: { 'read-only:vibe-glm': { result: 'pass' } } }),
      { now: '2026-11-03T12:20:00Z' })),
    run({ client: 'agy', model: 'gemini-3.8-flash-low', mode: 'read-only', prompt: 'agy proven.', repo: REPO }, merge(
      clients('agy'), allowlist('google'), behaves('agy', { out: agyOut() }),
      file(ROUTING_OVERRIDE, { clients: { 'agy-proof': { kind: 'agy', model: 'gemini-3.8-flash-low' } } }),
      file(SETUP_RESULTS, { checks: { 'read-only:agy-proof': { result: 'pass' } } }),
      { now: '2026-11-03T12:25:00Z' })),
    run({ client: 'claude', model: 'claude-haiku-5-5', mode: 'read-only', prompt: 'Claude is never proven.', repo: REPO }, merge(
      clients('claude'), allowlist('anthropic'), behaves('claude', { out: claudeOut() }),
      file(ROUTING_OVERRIDE, { clients: { 'claude-proof': { kind: 'claude', model: 'claude-haiku-5-5' } } }),
      file(SETUP_RESULTS, { checks: { 'read-only:claude-proof': { result: 'pass' } } }),
      { now: '2026-11-03T12:30:00Z' })),
    // The shipped routing names vibe-mistral-small for this model, so setup's result for it is the proof.
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Proven by the shipped routing.', repo: REPO }, merge(
      allowlist('mistral'),
      file(ROUTING_OVERRIDE, { clients: {} }),
      file(SETUP_RESULTS, { checks: { 'read-only:vibe-mistral-small': { result: 'pass' } } }),
      { now: '2026-11-03T12:32:00Z' })),
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Not a repository, an existing directory.', repo: `${SBX}/work` }, { now: '2026-11-03T12:35:00Z' }),
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'A path that is not there.', repo: `${SBX}/nowhere` }, { now: '2026-11-03T12:40:00Z' }),
  ]

  // The worktree a throwaway run made cannot be removed: the run still succeeds, and says so.
  scenarios['run-cleanup'] = [
    run({ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'The client breaks the repository.', repo: REPO }, merge(
      clients('vibe'), repo('mistral'),
      behaves('vibe', { out: journal('done\nSUMMARY: s'), export: vibeExport(), act: `rm "$HOME/../repo/.git/HEAD"` }),
      {})),
  ]

  // Refusals before anything is launched: nothing is written, nothing is reserved.
  const refusals = [
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Nested.' }, { env: { AGENT_KIT_DELEGATION: '1' } }],
    [{ client: 'vibe', model: 'a-model-nobody-knows', mode: 'read-only', prompt: 'x' }],
    [{ client: 'vibe', model: 'glm-5-3', mode: 'read-only', prompt: 'Disabled.' }, merge(override({ disabled: ['vibe:glm-5-3'] }))],
    [{ client: 'codex', model: 'gpt-5.5', mode: 'read-only', prompt: 'Codex without a repository is not permitted.' }, {}],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Repository without an entry.', repo: REPO }, merge(repo('mistral'), file(ALLOWLIST, { version: 1, repositories: {} }))],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Family not on the entry.', repo: REPO }, merge(allowlist('google'))],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'History.', repo: REPO, carries: { code: true, history: true } }, merge(allowlist('mistral'))],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Inline code.', carries: { code: true, history: false } }, {}],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Inline history.', carries: { code: false, history: true } }, {}],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'No remote.', repo: `${SBX}/bare` }, merge(
      { write: { 'bare/x': 'x' }, git: [['init', '-q', '-b', 'main', `${SBX}/bare`]] })],
    [{ client: 'opencode', model: 'anthropic/claude-sonnet-4.6', mode: 'read-only', prompt: 'Provider off the list.' }, {}],
    [{ client: 'opencode', model: 'opencode/gpt-5-nano', mode: 'read-only', prompt: 'Pay per token.' }, merge(override({ clients: { opencode: { providers: ['opencode', 'opencode-go'] } } }))],
    [{ client: 'opencode', model: 'noslash', mode: 'read-only', prompt: 'No provider in the name.' }, {}],
    [{ client: 'hermes', model: 'claude-sonnet-4.6', mode: 'read-only', prompt: 'Hermes names no provider.' }, merge(hermesMachine)],
    [{ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'openrouter', mode: 'read-only', prompt: 'OpenRouter.' }, {}],
    [{ client: 'hermes', model: 'openrouter/claude-sonnet-4.6', provider: 'anthropic', mode: 'read-only', prompt: 'OpenRouter by the model.' }, {}],
    [{ client: 'hermes', model: 'claude-sonnet-4.6', provider: 'azure', mode: 'read-only', prompt: 'Provider off the list.' }, {}],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Ended.', workflow: 'over' }, merge(workflows({ name: 'over', started_at: '2026-11-03T08:00:00Z', ended_at: '2026-11-03T09:00:00Z' }))],
    [{ client: 'vibe', model: 'mistral-small-latest', mode: 'read-only', prompt: 'Expired.', workflow: 'stale' }, merge(workflows({ name: 'stale', started_at: '2026-11-01T08:00:00Z' }))],
    // The live LiteLLM catalogue refuses a model it does not call usable, by any of its names, with the provider's reason.
    [{ client: 'vibe', model: 'glm-5-3', mode: 'read-only', prompt: 'Blocked by the provider.' }, merge(file(LITELLM_CATALOGUE, {
      updated_at: '2026-11-03T11:17:01Z',
      providers: {
        vibe: { models: [{ id: 'vibe/zai-glm-5-3', metadata: { status: 'blocked', reason: 'Model zai-glm-5-3 is a Labs model.', free: false } }] },
        'opencode-go': { models: [{ id: 'opencode-go/kimi-k2.7-code', metadata: { status: 'error', reason: '' } }] },
      },
    }))],
    [{ client: 'opencode', model: 'opencode-go/kimi-k2.7-code', mode: 'read-only', prompt: 'Probe failed.' }, {}],
  ]
  scenarios['run-refusals'] = refusals.map(([input, extra], index) => run(input, index === 0 ? merge(clients('vibe'), extra) : merge(extra)))

  // Every input shape `run` reads, refused before anything else is looked at.
  const good = { workflow: 'w', client: 'vibe', mode: 'read-only', model: 'mistral-small-latest', prompt: 'p' }
  const bad = (change) => run(JSON.stringify({ ...good, ...change }))
  const without = (key) => run(JSON.stringify(Object.fromEntries(Object.entries(good).filter(([name]) => name !== key))))
  scenarios['run-input'] = [
    run('[]'), run('null'), run('"x"'), run('5'),
    bad({ extras: {} }), bad({ extras: null }),
    without('workflow'), bad({ workflow: '' }), bad({ workflow: 'bad name' }), bad({ workflow: 5 }),
    without('client'), bad({ client: 'nobody' }), bad({ client: 5 }), bad({ client: 'VIBE' }),
    without('mode'), bad({ mode: 'plan' }), bad({ mode: 5 }),
    without('model'), bad({ model: '' }), bad({ model: 5 }),
    without('prompt'), bad({ prompt: '' }), bad({ prompt: ['x'] }),
    bad({ repo: '' }), bad({ repo: 5 }),
    bad({ carries: 'yes' }), bad({ carries: { code: true } }), bad({ carries: { code: 'y', history: false } }), bad({ carries: [] }),
    bad({ time_limit_s: 0 }), bad({ time_limit_s: -5 }), bad({ time_limit_s: 1.5 }), bad({ time_limit_s: '60' }), bad({ time_limit_s: null }),
    bad({ max_price_usd: 0 }), bad({ max_price_usd: -1 }), bad({ max_price_usd: '1' }), bad({ max_price_usd: null }),
    bad({ base: 'main' }), bad({ check: 'true' }),
    bad({ provider: '' }), bad({ provider: 5 }),
    bad({ escalate: 'no' }), bad({ family_binding: 1 }), bad({ headless: 'yes' }),
    bad({ max_attempts: 0 }), bad({ max_attempts: 1.5 }), bad({ max_attempts: '3' }), bad({ max_attempts: -1 }),
    // Writing runs: only what is refused before anything is launched.
    bad({ mode: 'write' }),
    bad({ mode: 'write', repo: REPO }),
    bad({ mode: 'write', repo: REPO, base: '' }),
    bad({ mode: 'write', repo: REPO, base: 'main' }),
    bad({ mode: 'write', repo: REPO, base: 'main', check: '' }),
    bad({ mode: 'write', base: 'main', check: 'true' }),
    run('{"workflow":"w","client":"vibe","mode":"read-only","model":"mistral-small-latest","prompt":"p","unknown_field":1,"time_limit_s":0}'),
  ]
  scenarios['run-input'][0] = merge(scenarios['run-input'][0], { mkdir: ['tmp'] })

  // Every step snapshots the temporary directory and the repository's gateway directories besides the usual roots.
  for (const name of Object.keys(scenarios)) {
    if (name.startsWith('run-')) scenarios[name] = scenarios[name].map((one) => ({ ...one, roots: ROOTS }))
  }
}
