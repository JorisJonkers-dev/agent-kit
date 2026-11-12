// The scenarios for `models`, `usage`, `grant write` and `grant list`: fake client programs written
// under bin/ (shell scripts), PATH is the sandbox's bin/ alone (plus the `git` shim where a step names
// one), the clock is fixed per step. Imported by gen-scenarios.mjs, which writes them out.

const SBX = '@SANDBOX@'
const ENV_BASE = { XDG_CACHE_HOME: `${SBX}/cache` }

export function addClientScenarios(scenarios, step) {
  /** A step whose PATH is the sandbox's bin/, at its own instant. */
  const on = (argv, extra = {}) => step(argv, { path: ['bin'], ...extra, env: { ...ENV_BASE, ...(extra.env ?? {}) } })
  const text = (input) => (typeof input === 'string' ? input : JSON.stringify(input))
  const modelsStep = (input, extra) => on(['models', '--json', text(input)], extra)
  const usageStep = (input, extra) => on(['usage', '--json', text(input)], extra)
  const grantStep = (verb, input, extra) => on(['grant', verb, '--json', text(input)], extra)
  const script = (name, body) => ({ [`bin/${name}`]: `#!/bin/sh\n${body}\n` })
  const execs = (...names) => names.map((name) => `bin/${name}`)

  const VIBE = (ids) =>
    script(
      'vibe',
      `echo "$@" >> "$HOME/../work/vibe-calls"
echo "warning: could not reach the telemetry host" >&2
echo "Error: unknown model 'model-gateway-probe'. Available: ${ids}." >&2
exit 1`,
    )
  const AGY = script(
    'agy',
    `printf 'Fetching available models...\\n'
printf 'gemini-3.8-flash-high\\tGemini 3.8 Flash High\\n'
printf 'gemini-3.8-flash-low\\tGemini Low\\n'
printf 'gemini-9-ultra\\tNew one\\n'
printf 'claude-sonnet-x\\tOther family\\n'
printf 'a line without a tab\\n'
printf '\\tno id\\n'`,
  )
  const OPENCODE = script(
    'opencode',
    `case "$1" in
  models)
    printf 'opencode-go/glm-5.3\\nopencode-go/glm-5.3-flash\\nopencode-go/kimi-k3.1-pro\\nopencode/gpt-5-nano\\n  anthropic/claude-sonnet-4.6  \\nopencode-go/brand-new\\nopencode-go/glm-5.3\\nnoslash\\n/leading\\ntrailing/\\n\\n' ;;
  auth) printf 'opencode-go  api key\\n' ;;
esac`,
  )
  const HERMES = script(
    'hermes',
    `case "$1" in
  config) echo '{"default":"anthropic/claude-sonnet-4.6","provider":"openrouter"}' ;;
  auth) echo 'openrouter  api key' ;;
esac`,
  )
  const CODEX = script('codex', `case "$1" in
  login) echo 'Logged in using ChatGPT' ;;
esac`)
  const CLAUDE = script('claude', 'exit 0')
  const ALL_CLIENTS = {
    ...VIBE('deepseek-v4-1-flash-1017, glm-5-3, mistral-small-latest, zai-glm-5-3-flash, brand-new-id'),
    ...AGY,
    ...OPENCODE,
    ...HERMES,
    ...CODEX,
    ...CLAUDE,
  }
  const ALL_EXEC = execs('vibe', 'agy', 'opencode', 'hermes', 'codex', 'claude')

  const VIBE_TOML = '[[models]]\nname = "deepseek-v4-1-flash"\nalias = "deepseek-v4-1-flash-1017"\n\n[[models]]\nalias = "glm-5-3"\nname = "glm-5.3"\n'
  scenarios['models-clients'] = [
    modelsStep({}, { write: ALL_CLIENTS, exec: ALL_EXEC }),
    modelsStep({}),
    modelsStep({}, { now: '2026-11-03T12:09:59Z' }),
    modelsStep({}, { now: '2026-11-03T12:10:00Z' }),
    modelsStep({}, { now: '2026-11-03T12:10:01Z' }),
    modelsStep({}, { now: '2026-11-03T12:05:00Z' }),
    modelsStep({}, { write: VIBE('glm-5-3, mistral-large-4-0-3'), exec: execs('vibe') }),
    modelsStep({}),
    modelsStep({}, { write: { ...VIBE('deepseek-v4-1-flash-1017, glm-5-3'), 'home/.vibe/config.toml': VIBE_TOML }, exec: execs('vibe') }),
    modelsStep({}, { write: { 'vibehome/config.toml': '[models.glm-5-3]\nname = "glm-5.3"\n' }, env: { VIBE_HOME: `${SBX}/vibehome` } }),
    modelsStep({}, { write: { 'home/.vibe/config.toml': 'this is = not [valid' } }),
    modelsStep({}, { write: { 'home/.vibe/config.toml': '[[models]]\nalias = "model-gateway-probe"\n' } }),
    modelsStep({}, { write: { 'home/.vibe/config.toml': '' }, env: { XDG_CACHE_HOME: `${SBX}/cache2` } }),
    modelsStep({}, { env: { XDG_CACHE_HOME: 'relative-cache' } }),
    modelsStep({}, { env: { XDG_CACHE_HOME: null } }),
    modelsStep({}, { remove: ['work/vibe-calls'], write: { 'cache/model-gateway/vibe-available.json': 'not json' } }),
    modelsStep({}, { write: { 'cache/model-gateway/vibe-available.json': '[]' } }),
  ]

  const LEDGER = 'state/agent-kit/model-gateway/ledger.jsonl'
  const run = (o) =>
    JSON.stringify({
      at: '2026-11-03T10:00:00.000Z',
      client: 'vibe',
      cost_class: 'subsidised',
      cost_usd: 0,
      duration_s: 3,
      family: 'deepseek',
      mode: 'read-only',
      model: 'glm-5-3',
      outcome: 'ok',
      repository: null,
      run_id: 'r',
      workflow: 'w',
      ...o,
    })
  const ledger = (...lines) => ({ [LEDGER]: `${lines.join('\n')}\n` })

  scenarios['models-failures'] = [
    modelsStep({}, {
      write: {
        ...script('vibe', 'exit 0'),
        ...script('agy', 'echo boom >&2; exit 2'),
        ...script('opencode', 'echo "  first line  " >&2; echo second >&2; exit 3'),
        ...script('hermes', 'echo "{}"'),
        ...CLAUDE,
      },
      exec: execs('vibe', 'agy', 'opencode', 'hermes', 'claude'),
    }),
    modelsStep({}, {
      write: { ...VIBE('good-1, bad id, good-2'), ...script('hermes', 'echo \'"gpt-5.5"\''), ...script('opencode', 'exit 1') },
      exec: execs('vibe', 'hermes', 'opencode'),
    }),
    modelsStep({}, {
      write: {
        ...script('vibe', 'printf "Available: a\\033[31mb\\n" >&2'),
        ...script('hermes', 'echo \'{"default":"","provider":3}\''),
        ...script('opencode', 'printf "opencode-go/glm-5.3\\n"; exit 0'),
        ...script('agy', 'exit 0'),
      },
      exec: execs('vibe', 'hermes', 'opencode', 'agy'),
    }),
    modelsStep({}, {
      write: {
        ...script('hermes', 'echo \'{"default":"gemini-3.8-flash-high"}\''),
        ...script('vibe', 'echo "Available: glm-5-3, deepseek-v4-1-flash-1017" >&2'),
        ...script('opencode', 'echo "garbage"'),
      },
      exec: execs('hermes', 'vibe', 'opencode'),
    }),
    modelsStep({}, { write: { ...script('hermes', 'echo "not json"') }, exec: execs('hermes') }),
    modelsStep({}, { write: { ...script('hermes', 'echo "[1]"') }, exec: execs('hermes') }),
    modelsStep({}, { write: { ...script('hermes', 'exit 4') }, exec: execs('hermes') }),
    modelsStep({}, { remove: ['bin/vibe', 'bin/agy', 'bin/opencode', 'bin/hermes', 'bin/claude'] }),
    modelsStep({}, { write: { ...script('claude', 'exit 0') }, exec: execs('claude'), path: [] }),
    modelsStep({}, {
      mkdir: ['bin2'],
      path: ['bin2', 'bin'],
      write: { 'bin2/codex': '#!/bin/sh\nexit 0\n', 'bin/claude': '#!/bin/sh\nexit 0\n' },
      exec: ['bin2/codex'],
    }),
    modelsStep({}, { write: { 'bin/notexec': 'x' }, mkdir: ['bin/codex'] }),
  ]

  const OVR = 'home/.config/agent-kit/model-gateway.json'
  const ovr = (value, extra = {}) => modelsStep({}, { write: { [OVR]: text(value) }, ...extra })
  const newClient = { custom: { binary: 'custom', cost_class: 'metered-personal', model_source: 'declared' } }
  const newModel = [{ client: 'custom', model: 'c-1', family: 'openai', tier: 'hard', notes: 'n' }]
  scenarios['models-override'] = [
    modelsStep({}, {
      write: {
        [OVR]: JSON.stringify({ disabled: ['vibe:glm-5-3', 'opencode:opencode-go/glm-5.3-flash', 'claude:claude-haiku-5-5'] }),
        ...ALL_CLIENTS,
      },
      exec: ALL_EXEC,
    }),
    ovr({
      clients: {
        vibe: { families: ['deepseek'] },
        opencode: { providers: ['opencode-go', 'other'], provider_cost_classes: { other: 'metered-employer' } },
        agy: { families: ['google', 'anthropic'] },
      },
    }),
    ovr({
      models: [
        { client: 'vibe', model: 'glm-5-3', tier: 'frontier', notes: 'changed' },
        { client: 'claude', model: 'claude-x', family: 'anthropic', tier: 'trivial', notes: 'new', names: ['cx'] },
        { client: 'vibe', model: 'brand-new-id', family: 'mistral', tier: 'routine', notes: 'now known' },
      ],
    }),
    ovr({ clients: newClient, models: newModel }, { write: { [OVR]: JSON.stringify({ clients: newClient, models: newModel }), 'bin/custom': '#!/bin/sh\nexit 0\n' }, exec: ['bin/custom'] }),
    ovr({ settings: { plan_warning_ratio: 0.5, metered_personal_caps: { per_day_usd: 1 }, output_allowance_tokens: { hard: 7 } }, version: 1 }),
    ovr('{}'), ovr('[]'), ovr('null'), ovr('not json'), ovr(''), ovr({ version: 2 }), ovr({ extra: 1 }),
    ovr({ clients: [] }), ovr({ clients: { vibe: 3 } }), ovr({ clients: { vibe: { nope: 1 } } }), ovr({ clients: { vibe: { binary: '' } } }),
    ovr({ clients: { vibe: { cost_class: 'free' } } }), ovr({ clients: { vibe: { model_source: 'magic' } } }), ovr({ clients: { vibe: { families: 'google' } } }),
    ovr({ clients: { vibe: { families: ['google', ''] } } }), ovr({ clients: { vibe: { provider_cost_classes: [] } } }),
    ovr({ clients: { vibe: { provider_cost_classes: { p: 'x' } } } }), ovr({ clients: { vibe: { providers: [1] } } }),
    ovr({ clients: { vibe: { plan_limits: {} } } }), ovr({ clients: { vibe: { plan_limits: [1] } } }),
    ovr({ clients: { vibe: { plan_limits: [{ window: 'year', limit_usd: 1 }] } } }), ovr({ clients: { vibe: { plan_limits: [{ window: 'day' }] } } }),
    ovr({ clients: { vibe: { plan_limits: [{ window: 'day', limit_usd: 1, limit_runs: 1 }] } } }),
    ovr({ clients: { vibe: { plan_limits: [{ window: 'day', limit_usd: 0 }] } } }), ovr({ clients: { vibe: { plan_limits: [{ window: 'day', limit_usd: 'x' }] } } }),
    ovr({ clients: { vibe: { plan_limits: [{ window: 'day', limit_runs: 1.5 }] } } }),
    ovr({ clients: { vibe: { plan_limits: [{ window: '5h', limit_runs: 2 }, { window: 'month', limit_usd: 9.5 }] } } }),
    ovr({ clients: { fresh: { binary: 'fresh' } } }), ovr({ clients: { fresh: { binary: 'f', cost_class: 'subsidised' } } }),
    ovr({ models: {} }), ovr({ models: [1] }), ovr({ models: [{ client: 'vibe' }] }), ovr({ models: [{ client: 'vibe', model: 'zz' }] }),
    ovr({ models: [{ client: 'vibe', model: 'zz', family: 'x', tier: 'huge', notes: '' }] }),
    ovr({ models: [{ client: 'vibe', model: 'zz', family: 'x', tier: 'hard', notes: 3 }] }),
    ovr({ models: [{ client: 'vibe', model: 'glm-5-3', prices: 3 }] }),
    ovr({ models: [{ client: 'vibe', model: 'glm-5-3', prices: { input: 1, output: 0, cached_input: 0 } }] }),
    ovr({ models: [{ client: 'vibe', model: 'glm-5-3', prices: { input: 1, output: 1, cached_input: -1 } }] }),
    ovr({ models: [{ client: 'vibe', model: 'glm-5-3', prices: { input: 1, output: 1 } }] }),
    ovr({ models: [{ client: 'vibe', model: 'glm-5-3', prices: { input: 1, output: 1, cached_input: 0 }, names: [''] }] }),
    ovr({ settings: [] }), ovr({ settings: { summary_max_chars: 'x' } }), ovr({ settings: { max_parallel_per_client: 0 } }),
    ovr({ settings: { max_batch_delegations: 1.5 } }), ovr({ settings: { plan_warning_ratio: 2 } }), ovr({ settings: { metered_personal_caps: [] } }),
    ovr({ settings: { metered_personal_caps: { per_day_usd: 'x' } } }), ovr({ settings: { output_allowance_tokens: [] } }),
    ovr({ settings: { output_allowance_tokens: { hard: 0 } } }), ovr({ settings: { output_allowance_tokens: { frontier: 'x' } } }),
    ovr({ disabled: 'x' }), ovr({ disabled: [''] }), ovr({ disabled: [] }),
    modelsStep({}, { remove: [OVR], mkdir: [OVR] }),
    modelsStep({}, { env: { XDG_CONFIG_HOME: `${SBX}/cfg` }, write: { 'cfg/agent-kit/model-gateway.json': '{"disabled":["claude:claude-opus-5-5"]}' } }),
    modelsStep({}, { env: { XDG_CONFIG_HOME: 'relative' } }),
    modelsStep({}, { env: { HOME: null, XDG_CONFIG_HOME: null } }),
    modelsStep({}, { env: { HOME: null, XDG_CONFIG_HOME: `${SBX}/cfg` } }),
  ]

  const reserve = (o) =>
    JSON.stringify({
      kind: 'reservation',
      run_id: 'res',
      workflow: 'w',
      client: 'hermes',
      cost_class: 'metered-personal',
      model: 'm',
      estimate_usd: 0.25,
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
      client: 'hermes',
      cost_class: 'metered-personal',
      cost_usd: 0.1,
      reserved_at: '2026-11-03T09:00:00.000Z',
      settled_at: '2026-11-03T09:30:00.000Z',
      ...o,
    })

  scenarios['models-ledger'] = [
    modelsStep({}, {
      write: {
        ...ALL_CLIENTS,
        ...ledger(
          run({ model: 'glm-5-3', outcome: 'failed', reason: 'rate-limited', unavailable_until: '2026-11-03T18:00:00.000Z', at: '2026-11-03T11:00:00.000Z' }),
          run({ model: 'zai-glm-5-3-flash', outcome: 'failed', reason: 'model-unavailable', at: '2026-11-03T11:30:00.000Z' }),
          run({ client: 'opencode', model: 'opencode-go/glm-5.3', outcome: 'failed', reason: 'rate-limited', unavailable_until: '2026-11-03T11:59:59.000Z', at: '2026-11-03T11:00:00.000Z' }),
          run({ client: 'opencode', model: 'opencode-go/kimi-k3.1-pro', outcome: 'failed', reason: 'exit-1', at: '2026-11-03T11:00:00.000Z' }),
          '{"torn":',
          '',
          '   ',
          run({ client: 'claude', model: 'claude-opus-5-5', outcome: 'failed', reason: 'rate-limited', at: '2026-11-03T11:00:00.000Z' }),
          run({ client: 'claude', model: 'claude-opus-5-5', outcome: 'ok', at: '2026-11-03T11:10:00.000Z' }),
          run({ client: 'claude', model: 'claude-sonnet-5-5', outcome: 'failed', reason: 'rate-limited', unavailable_until: '2026-11-03T20:00:00.000Z', at: '2026-11-03T11:00:00.000Z' }),
          run({ client: 'claude', model: 'claude-sonnet-5-5', outcome: 'failed', reason: 'rate-limited', unavailable_until: '2026-11-03T19:00:00.000Z', at: '2026-11-03T11:30:00.000Z' }),
          reserve({ run_id: 'x', client: 'codex', cost_class: 'metered-employer', model: 'gpt-5.5', run_dir: '/nowhere' }),
        ),
      },
      exec: ALL_EXEC,
    }),
    modelsStep({}, { now: '2026-11-03T19:30:00Z' }),
    modelsStep({}, { now: '2026-11-04T00:00:00Z' }),
    modelsStep({}, { write: { [LEDGER]: '' } }),
    modelsStep({}, { remove: [LEDGER], mkdir: [LEDGER] }),
  ]

  const ALLOWLIST = 'home/.config/agent-kit/critique-allowlist.json'
  const allow = (value) => ({ [ALLOWLIST]: text(value) })
  const repoStep = (repo, extra = {}) => modelsStep({ repo: `${SBX}/work/${repo}` }, { shims: ['git'], ...extra })
  const gitDirs = (name) => [`work/${name}/.git/objects`, `work/${name}/.git/refs/heads`]
  const gitFiles = (name, config) => ({ [`work/${name}/.git/HEAD`]: 'ref: refs/heads/main\n', [`work/${name}/.git/config`]: config })
  scenarios['models-repository'] = [
    repoStep('repo', {
      write: {
        ...ALL_CLIENTS,
        ...gitFiles('repo', '[core]\n\trepositoryformatversion = 0\n[remote "origin"]\n\turl = git@github.com:Owner/Repo.git\n\tfetch = +refs/heads/*:refs/remotes/origin/*\n'),
        ...allow({ version: 1, repositories: { 'github.com/owner/repo': { families: ['deepseek', 'zhipu', 'anthropic'], code: true, history: false }, 'github.com/owner/other': { families: [] } } }),
      },
      mkdir: [...gitDirs('repo'), 'work/repo/sub'],
      exec: ALL_EXEC,
    }),
    repoStep('repo/sub'),
    repoStep('repo', { write: allow({ version: 1, repositories: { 'github.com/owner/repo': { families: [] } } }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: {} }) }),
    repoStep('repo', { remove: [ALLOWLIST] }),
    repoStep('repo', { write: allow('nope') }),
    repoStep('repo', { write: allow({ version: 2, repositories: {} }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: { 'github.com/owner/repo': { families: ['zhipu'], code: 'yes' } } }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: { 'github.com/owner/repo': { families: ['zhipu'], history: true } } }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: { 'github.com/owner/repo': { families: 'zhipu' } } }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: { 'github.com/owner/repo': 5 } }) }),
    repoStep('repo', { write: allow({ version: 1, repositories: [] }) }),
    repoStep('nedap', { write: gitFiles('nedap', '[remote "origin"]\n\turl = https://github.com/Nedap/thing.git\n'), mkdir: gitDirs('nedap') }),
    repoStep('noremote', { write: gitFiles('noremote', '[core]\n\tbare = false\n'), mkdir: gitDirs('noremote') }),
    repoStep('plain', { mkdir: ['work/plain'], env: { GIT_CEILING_DIRECTORIES: SBX } }),
    repoStep('missing'),
    modelsStep({ repo: `${SBX}/work/repo` }, { remove: ['bin/git'] }),
    repoStep('repo', {
      write: {
        ...gitFiles('repo', '[remote "upstream"]\n\turl = https://example.com/team/proj\n[remote "origin"]\n\turl = ssh://git@host.example:2222/Group/Sub/Proj.git/\n'),
        ...allow({ version: 1, repositories: { 'host.example/group/sub/proj': { families: ['deepseek'], code: true, history: true } } }),
      },
    }),
    repoStep('repo', { write: gitFiles('repo', '[remote "b"]\n\turl = nonsense\n[remote "a"]\n\turl = https://github.com/x/y\n') }),
    modelsStep({ repo: 5 }), modelsStep({ repo: '' }), modelsStep({ repo: null }), modelsStep({ repo: [] }),
    modelsStep({}, { shims: ['git'] }),
  ]

  const tokens = (input, output, cached) => ({ tokens: { input, output, cached } })
  scenarios['usage'] = [
    usageStep({}, {
      write: {
        ...ALL_CLIENTS,
        ...ledger(
          run({ run_id: 'a1', at: '2026-11-03T09:00:00.000Z', cost_usd: 0.1234567891, ...tokens(100, 50, 10) }),
          run({ run_id: 'a2', at: '2026-11-03T09:30:00.000Z', cost_usd: 0.2, workflow: 'other' }),
          run({ run_id: 'a3', at: '2026-11-02T23:59:59.999Z', cost_usd: 1, ...tokens(1, 2, 3) }),
          run({ run_id: 'a4', at: '2026-11-01T01:00:00.000Z', cost_usd: 5, ...tokens(9, 9, 9) }),
          run({ run_id: 'a5', client: 'opencode', at: '2026-11-03T08:00:00.000Z', cost_usd: 0.05, ...tokens(10, 10, 0) }),
          run({ run_id: 'a6', client: 'hermes', at: '2026-11-03T08:00:00.000Z', cost_usd: 0.3, cost_class: 'metered-personal', model: 'm' }),
          run({ run_id: 'a7', client: 'codex', at: '2026-11-03T08:30:00.000Z', cost_usd: 1.5, cost_class: 'metered-employer', ...tokens(5, 5, 5) }),
          '{"bad',
          '',
          run({ run_id: 'a8', client: 'claude', at: '2026-11-03T11:50:00.000Z', cost_usd: 0, outcome: 'failed', reason: 'exit-1' }),
          reserve({ run_id: 'live', estimate_usd: 0.4 }),
          reserve({ run_id: 'expired', estimate_usd: 0.6, reserved_at: '2026-11-03T07:00:00.000Z', expires_at: '2026-11-03T08:00:00.000Z', run_dir: `${SBX}/work/runs/expired` }),
          reserve({ run_id: 'finished', estimate_usd: 0.7, client: 'codex', cost_class: 'metered-employer', run_dir: `${SBX}/work/runs/finished` }),
          reserve({ run_id: 'ran', estimate_usd: 0.8 }),
          run({ run_id: 'ran', client: 'hermes', cost_class: 'metered-personal', cost_usd: 0.05 }),
          reserve({ run_id: 'done', estimate_usd: 0.9 }),
          settled({ run_id: 'done' }),
          reserve({ run_id: 'badresult', estimate_usd: 0.2, expires_at: '2026-11-03T11:30:00.000Z', run_dir: `${SBX}/work/runs/badresult` }),
          reserve({ run_id: 'nocost', estimate_usd: 0.2, expires_at: '2026-11-03T11:31:00.000Z', run_dir: `${SBX}/work/runs/nocost` }),
          reserve({ run_id: 'negcost', estimate_usd: 0.2, expires_at: '2026-11-03T11:32:00.000Z', run_dir: `${SBX}/work/runs/negcost` }),
        ),
        'work/runs/finished/result.json': '{"cost_usd":0.35}',
        'work/runs/badresult/result.json': 'not json',
        'work/runs/nocost/result.json': '{"status":"ok"}',
        'work/runs/negcost/result.json': '{"cost_usd":-1}',
      },
      exec: ALL_EXEC,
    }),
    usageStep({}),
    usageStep({ days: 1 }), usageStep({ days: 30, workflow: 'other' }), usageStep({ workflow: 'nothing-here' }), usageStep({ days: 365 }),
    usageStep({ days: 0 }), usageStep({ days: -1 }), usageStep({ days: 1.5 }), usageStep({ days: '3' }), usageStep({ days: null }),
    usageStep({ workflow: '' }), usageStep({ workflow: 'bad name' }), usageStep({ workflow: 3 }),
    usageStep({ ignored: true }, { now: '2026-11-04T00:00:01Z' }),
    usageStep({}, {
      write: {
        [OVR]: JSON.stringify({
          clients: {
            vibe: { plan_limits: [{ window: '5h', limit_usd: 0.5 }, { window: 'day', limit_runs: 3 }, { window: 'week', limit_usd: 100 }, { window: 'month', limit_runs: 4 }] },
            opencode: { plan_limits: [{ window: 'day', limit_usd: 0.05 }] },
            codex: { plan_limits: [{ window: 'day', limit_runs: 1 }] },
          },
          settings: { plan_warning_ratio: 0.5, availability_window_minutes: 90 },
        }),
      },
    }),
    usageStep({}, {
      write: {
        ...script('codex', 'case "$1" in login) echo "Not logged in" >&2; exit 1 ;; esac'),
        ...script('hermes', 'case "$1" in auth) exit 0 ;; esac'),
        ...script('opencode', 'case "$1" in auth) echo "unknown command" >&2; exit 2 ;; esac'),
      },
      exec: execs('codex', 'hermes', 'opencode'),
    }),
    usageStep({}, {
      write: {
        ...script('codex', 'case "$1" in login) echo "weird" >&2; exit 5 ;; esac'),
        ...script('hermes', 'case "$1" in auth) printf "\\n  first\\nsecond\\n" ;; esac'),
        ...script('opencode', 'case "$1" in auth) exit 9 ;; esac'),
      },
    }),
    usageStep({}, { write: { ...script('codex', 'case "$1" in login) printf "\\n   \\n"; exit 0 ;; esac') } }),
    usageStep({}, { write: { ...script('codex', 'case "$1" in login) echo "Unauthenticated" ; exit 1 ;; esac') } }),
    usageStep({}, { write: { ...script('codex', 'case "$1" in login) echo "login required" >&2; exit 1 ;; esac') } }),
    usageStep({}, { remove: ['bin/vibe', 'bin/agy', 'bin/opencode', 'bin/hermes', 'bin/codex', 'bin/claude'] }),
    usageStep({}, { write: { [LEDGER]: '' } }),
    usageStep({}, { remove: [LEDGER] }),
    usageStep({}, {
      write: ledger(
        run({ run_id: 'x1', client: 'vibe', outcome: 'failed', reason: 'rate-limited', at: '2026-11-03T11:30:00.000Z' }),
        run({ run_id: 'x2', client: 'agy', model: 'm', outcome: 'failed', reason: 'auth', at: '2026-11-03T11:40:00.000Z' }),
        run({ run_id: 'x3', client: 'opencode', outcome: 'failed', reason: 'rate-limited', at: '2026-11-03T08:00:00.000Z' }),
        run({ run_id: 'x4', client: 'claude', outcome: 'failed', reason: 'rate-limited', unavailable_until: '2026-11-03T15:00:00.000Z', at: '2026-11-03T11:40:00.000Z' }),
        run({ run_id: 'x5', client: 'claude', outcome: 'failed', reason: 'model-unavailable', at: '2026-11-03T11:41:00.000Z' }),
        run({ run_id: 'x6', client: 'hermes', outcome: 'failed', reason: 'rate-limited', at: '2026-11-03T11:00:00.000Z' }),
        run({ run_id: 'x7', client: 'hermes', outcome: 'ok', at: '2026-11-03T11:10:00.000Z' }),
        run({ run_id: 'x8', client: 'hermes', outcome: 'failed', reason: 'auth', at: '2026-11-03T11:20:00.000Z' }),
        run({ run_id: 'x9', client: 'vibe', outcome: 'failed', at: '2026-11-03T11:45:00.000Z' }),
      ),
    }),
    usageStep({}, { remove: [OVR, LEDGER], mkdir: [LEDGER] }),
  ]

  const GRANTS = 'home/.config/agent-kit/model-gateway-grants.json'
  const NOW_ISO = '2026-11-03T12:00:00.000Z'
  const grant = (o) => ({ id: 'g1', workflow: 'w', client: 'codex', cap_usd: 1, question: 'q', answer: 'a', written_at: NOW_ISO, expires_at: '2026-11-04T12:00:00Z', ...o })
  const grantsFile = (value) => ({ [GRANTS]: text(value) })
  const write = (workflow, extra, cap = 1) => grantStep('write', { workflow, client: 'codex', cap_usd: cap, question: 'q', answer: 'a' }, extra)
  scenarios['grants'] = [
    grantStep('write', { workflow: 'spec-1', client: 'codex', cap_usd: 5, question: 'may I?', answer: 'yes' }, { write: ALL_CLIENTS, exec: ALL_EXEC, shims: ['git'] }),
    grantStep('list', {}),
    on(['workflow', 'start', '--json', '{"name":"spec-1"}']),
    grantStep('write', { workflow: 'spec-1', client: 'codex', cap_usd: 5, question: 'may I?', answer: 'yes' }),
    grantStep('write', { workflow: 'spec-1', client: 'codex', cap_usd: 3, question: 'again?', answer: 'no' }, { now: '2026-11-03T12:30:00Z' }),
    grantStep('list', {}, { now: '2026-11-03T12:45:00Z' }),
    on(['workflow', 'start', '--json', '{"name":"spec-2"}'], { now: '2026-11-03T13:00:00Z' }),
    write('spec-2', { now: '2026-11-03T14:00:00Z' }, 0.5),
    write('unstarted', { now: '2026-11-03T14:00:00Z' }, 1.25),
    grantStep('list', {}, { now: '2026-11-03T14:30:00Z' }),
    grantStep('list', {}, { now: '2026-11-04T12:00:00Z' }),
    grantStep('list', {}, { now: '2026-11-04T13:00:00Z' }),
    grantStep('list', {}, { now: '2026-11-04T14:00:00Z' }),
    grantStep('list', {}, { now: '2026-11-04T14:00:01Z' }),
    on(['workflow', 'end', '--json', '{"name":"spec-1"}'], { now: '2026-11-04T15:00:00Z' }),
    write('spec-1', { now: '2026-11-04T15:00:00Z' }),
    grantStep('list', {}, { now: '2026-11-04T15:00:00Z' }),
    write('spec-2', { now: '2026-11-04T15:00:00Z' }),
    write('w', { env: { AGENT_KIT_DELEGATION: '1' } }),
    grantStep('write', { workflow: 'w', client: 'claude', cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', { workflow: 'w', client: 'hermes', cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', { workflow: 'w', client: 'nobody', cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', {}),
    write('w', {}, 0), write('w', {}, -1), write('w', {}, '5'), write('w', {}, 1e999),
    grantStep('write', { workflow: 'w', client: 'codex', cap_usd: 1, question: '  ', answer: 'a' }),
    grantStep('write', { workflow: 'w', client: 'codex', cap_usd: 1, question: 'q', answer: '' }),
    grantStep('write', { workflow: 'w', client: 5, cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', { workflow: 'bad name', client: 'codex', cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', { workflow: 'w', cap_usd: 1, question: 'q', answer: 'a' }),
    grantStep('write', { workflow: 'w', client: 'codex', cap_usd: 1, question: 'line\nbreak', answer: 'quote"d' }),
    grantStep('list', { anything: 1 }),
    grantStep('list', '[]'),
    grantStep('list', {}, { write: grantsFile('nope') }),
    grantStep('list', {}, { write: grantsFile('[]') }),
    grantStep('list', {}, { write: grantsFile({ version: 2, grants: [] }) }),
    grantStep('list', {}, { write: grantsFile({ version: 1, grants: {} }) }),
    grantStep('list', {}, { write: grantsFile({ version: 1, grants: [1] }) }),
    ...[
      { cap_usd: 0 }, { cap_usd: 'x' }, { answer: '' }, { client: 1 }, { expires_at: '2026-11-04' }, { expires_at: '2026-02-30T00:00:00Z' },
      { written_at: 'now' }, { ended_at: 5 }, { id: '' }, { question: null }, { workflow: [] },
    ].map((o) => grantStep('list', {}, { write: grantsFile({ version: 1, grants: [grant(o)] }) })),
    grantStep('list', {}, {
      write: {
        ...grantsFile({
          version: 1,
          grants: [
            grant({ written_at: '2026-11-03T11:00:00Z', expires_at: '2026-11-04T11:00:00.500Z', ended_at: '2026-11-03T11:30:00Z' }),
            grant({ id: 'g2', workflow: 'w2', cap_usd: 2, written_at: '2026-11-03T11:00:00Z', expires_at: '2026-11-04T11:00:00Z' }),
            grant({ id: 'g3', workflow: 'w3', written_at: '2026-11-03T10:00:00Z', expires_at: '2026-11-04T10:00:00Z' }),
          ],
        }),
        ...ledger(
          run({ run_id: 's1', client: 'codex', cost_class: 'metered-employer', workflow: 'w3', cost_usd: 1, at: '2026-11-03T10:30:00.000Z' }),
          reserve({ run_id: 'r2', client: 'codex', cost_class: 'metered-employer', workflow: 'w2', estimate_usd: 0.5, reserved_at: '2026-11-03T11:30:00.000Z' }),
          settled({ run_id: 's9', client: 'codex', cost_class: 'metered-employer', workflow: 'w2', cost_usd: 0.25, reason: 'expired', reserved_at: '2026-11-03T11:15:00.000Z' }),
        ),
      },
    }),
    write('w2', { write: ledger(run({ run_id: 'z' }), 'not json at all') }),
    write('w9', { remove: [LEDGER], mkdir: [LEDGER] }),
    grantStep('list', {}, { remove: [LEDGER] }),
    write('w9', { remove: [GRANTS], mkdir: [GRANTS] }),
    write('w9', {
      remove: [GRANTS],
      env: { XDG_CONFIG_HOME: `${SBX}/work/repo/conf` },
      write: gitFiles('repo', '[core]\n'),
      mkdir: gitDirs('repo'),
    }),
    grantStep('list', {}, { env: { XDG_CONFIG_HOME: `${SBX}/work/repo/conf/deeper/still` } }),
    grantStep('list', {}, { env: { XDG_CONFIG_HOME: `${SBX}/work/repo/.git` } }),
    write('w9', { env: { XDG_CONFIG_HOME: `${SBX}/outside/new/place` } }),
    grantStep('list', {}, { remove: ['bin/git'] }),
    grantStep('list', {}, { env: { HOME: null, XDG_CONFIG_HOME: null } }),
    write('w9', { env: { XDG_CONFIG_HOME: `${SBX}/blockedcfg` }, write: { blockedcfg: 'file' }, shims: ['git'] }),
  ]
}
