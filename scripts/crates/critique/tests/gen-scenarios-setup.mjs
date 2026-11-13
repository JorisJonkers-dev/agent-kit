#!/usr/bin/env node
// Writes the scenario.json files of setup (the goldens are expected.json, from regenerate-fixtures.mjs). The fake
// `vibe` and `agy` programs go in the sandbox's bin/, which a step's `path` makes the only place programs are found.
// A Vibe client's read-only proof is not here: it starts the runner's own search server, and the node runner's is
// `node critique.mjs`, so the proof's launch cannot be the same bytes (docs/RUST.md). Its parts are unit-tested.
import { createHash } from 'node:crypto'
import { mkdirSync, writeFileSync } from 'node:fs'
import { dirname, join } from 'node:path'
import { fileURLToPath } from 'node:url'

const fixtures = join(dirname(fileURLToPath(import.meta.url)), 'fixtures')
const SB = '@SANDBOX@'
const R = (dir) => `${SB}/repos/${dir}`
const q = (text) => `'${text.split("'").join("'\\''")}'`
const setup = (input, extra = {}) => ({ argv: ['setup', '--json', typeof input === 'string' ? input : JSON.stringify(input)], mkdir: ['bin'], path: ['bin'], ...extra })
const check = (id, extra = {}, input = {}) => setup({ action: 'check', check: id, ...input }, extra)
const RESULTS = 'home/.local/share/agent-kit/critique/machine-setup.json'
const LEDGER = 'home/.local/share/agent-kit/critique/ledger.jsonl'
const OVERRIDE = 'home/.config/agent-kit/critique-routing.json'
const CONFIG = 'home/.config/agent-kit/critique.json'
const DATA = ['home/.local/share/agent-kit/critique']
const scenarios = {}

// Fake programs. A fake `vibe` answers `--version`, then answers a launch by the folder its `--output-dir` names.
const GOOD_EXPORT = { config: { enable_mcp: false, enabled_tools: ['read_file'], include_project_context: false, outside_workdir: 'deny' }, cost_usd: 0.002, usage: { input_tokens: 1200, cached_input_tokens: 200, output_tokens: 300 } }
const journal = (text) => JSON.stringify([{ type: 'message', role: 'user', content: [{ text: 'q' }] }, { type: 'message', role: 'assistant', content: [{ text }] }])
const EMPTY = '```json\n{"findings": [], "no_findings_reason": "setup check: nothing to review"}\n```\n'
const outcome = ({ exported = GOOD_EXPORT, stdout = '', stderr = '', code = 0 }) =>
  `${exported === null ? '' : `printf '%s\\n' ${q(JSON.stringify(exported))} > "$out/export.json"\n`}printf '%s' ${q(stdout)}\nprintf '%s' ${q(stderr)} >&2\nexit ${code}`
const fakeVibe = (version, cases) =>
  `#!/bin/sh\nif [ "$1" = --version ]; then\nprintf '%s\\n' ${q(version)} 'second line'\nexit 0\nfi\nout=''\nwhile [ $# -gt 0 ]; do\n  if [ "$1" = --output-dir ]; then out="$2"; fi\n  shift\ndone\nmkdir -p "$out"\ncase "$out" in\n${Object.entries(cases)
    .map(([where, body]) => `*${where}/export)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const agyStream = (result) => `printf '%s\\n' ${q(JSON.stringify({ event: 'result', result }))}`
const fakeAgy = (version, cases, models = `printf '%s\\n' 'Fetching available models...' 'gemini-3.8-flash-high\\tGemini 3.8 Flash'\nexit 0`) =>
  `#!/bin/sh\nif [ "$1" = --version ]; then\nprintf '%s\\n' ${q(version)}\nexit 0\nfi\nif [ "$1" = models ]; then\n${models}\nfi\ncase "$PWD" in\n${Object.entries(cases)
    .map(([where, body]) => `*${where}/packet)\n${body}\n;;`)
    .join('\n')}\n*)\nexit 9\n;;\nesac\n`
const VIBE_HOME = { 'home/.vibe/.env': 'MISTRAL_API_KEY=sk-test\n', 'home/.vibe/agents/critic-mistral-small.toml': '# agent\n' }
const MISTRAL = '/setup-checks/vibe-mistral-small/critic'
const GEMINI = '/setup-checks/agy-gemini-flash-high'
const withVibe = (version, cases, write = {}) => ({ write: { 'bin/vibe': fakeVibe(version, cases), ...write }, exec: ['bin/vibe'], shims: ['env', 'mkdir'] })
const withAgy = (version, cases, models, write = {}) => ({ write: { 'bin/agy': fakeAgy(version, cases, models), ...write }, exec: ['bin/agy'], shims: ['env', 'mkdir'] })
const both = (vibe, agy, write = {}) => ({
  write: { 'bin/vibe': vibe.write['bin/vibe'], 'bin/agy': agy.write['bin/agy'], ...write },
  exec: ['bin/vibe', 'bin/agy'],
  shims: ['env', 'mkdir'],
})

// The machine as it comes: nothing checked, nothing installed, no program found.
scenarios['setup-list'] = [
  setup({}),
  setup({ action: 'list' }),
  setup('{"action":"nope"}'),
  setup('{"action":"list","repo":5}'),
  setup('[1]'),
  // The repository the owner is in: not opted in, no remote, nedap, and opted in.
  setup(
    { repo: R('s1') },
    {
      write: { 'repos/s1/a.md': 'a\n', 'repos/s2/a.md': 'a\n', 'repos/s3/a.md': 'a\n' },
      git: [
        ['init', '-q', '--template=', 'repos/s1'], ['-C', 'repos/s1', 'remote', 'add', 'origin', 'git@github.com:JorisJonkers-dev/example.git'],
        ['init', '-q', '--template=', 'repos/s2'],
        ['init', '-q', '--template=', 'repos/s3'], ['-C', 'repos/s3', 'remote', 'add', 'origin', 'git@github.com:nedap/thing.git'],
      ],
      shims: ['git'],
    },
  ),
  setup({ repo: R('s2') }, { shims: ['git'] }),
  setup({ repo: R('s3') }, { shims: ['git'] }),
  setup(
    { repo: R('s1') },
    {
      write: {
        'home/.config/agent-kit/critique-allowlist.json': `${JSON.stringify({ repositories: { 'github.com/jorisjonkers-dev/example': { code: true, families: ['mistral', 'google'], history: false, updated_at: '2026-11-01T00:00:00.000Z' } }, version: 1 })}\n`,
      },
      shims: ['git'],
    },
  ),
  // An override that names the removed per-flow Vibe cap, and a results file that does not read.
  setup({}, { write: { [OVERRIDE]: '{"budget":{"vibe_flow_cap_usd":5}}\n' } }),
  setup({}, { remove: [OVERRIDE], write: { [RESULTS]: '{"version":2,"checks":{}}\n' } }),
  setup({}, { write: { [RESULTS]: '{"version":1,"checks":{"x":{"checked_at":"t","detail":"d","inputs":{"files_hash":5},"result":"pass"}}}\n' } }),
  setup({}, { remove: [RESULTS], write: { [OVERRIDE]: '[1]\n' } }),
]

// Sign-in checks, and how a check becomes current, due again, or due because it failed.
scenarios['setup-sign-in'] = [
  check('vibe-signed-in'),
  check('vibe-signed-in', withVibe('vibe 1.2.3', {}, VIBE_HOME)),
  check('vibe-signed-in', { path: ['bin'] }),
  check('vibe-signed-in', { path: ['bin'] }, { force: true }),
  setup({}),
  check('vibe-signed-in', withVibe('vibe 1.2.4', {})),
  setup({}),
  check('vibe-signed-in', {}, { force: true }),
  check('vibe-signed-in', { remove: ['home/.vibe/.env'] }, { force: true }),
  setup({}),
  check('nope'),
  check('agy-signed-in', withAgy('agy 0.9', {}, undefined)),
  check('agy-signed-in', withAgy('agy 0.9', {}, "printf '%s\\n' 'Error: Please sign in to view available models.' >&2\nexit 1"), { force: true }),
  check('agy-signed-in', withAgy('agy 0.9', {}, "printf '%s\\n' 'Fetching available models...'\nexit 0"), { force: true }),
  check('agy-signed-in', { remove: ['bin/agy'] }, { force: true }),
  setup({}),
  setup('{"action":"check"}'),
  setup('{"action":"check","check":"vibe-signed-in","confirmed":"yes"}'),
]

// A critic answering the trivial prompt: it spends, so it runs only when confirmed, and the ledger counts it.
const probe = (stdout, extra = {}, code = 0) => outcome({ stdout: journal(stdout), code, ...extra })
scenarios['setup-probe'] = [
  check(
    'critic:vibe-mistral-small',
    both(
      withVibe('vibe 1.2.3', { [MISTRAL]: probe(EMPTY) }),
      withAgy('agy 0.9', { [GEMINI]: `${agyStream({ response: 'ok', structured_output: { findings: [], no_findings_reason: 'setup check: nothing to review' } })}\nexit 0` }),
      VIBE_HOME,
    ),
    {},
  ),
  check('critic:vibe-mistral-small', {}, { confirmed: true }),
  check('critic:vibe-mistral-small', {}, { confirmed: true }),
  check('critic:agy-gemini-flash-high', {}, {}),
  check('critic:agy-gemini-flash-high', {}, { confirmed: true }),
  setup({}),
  // The quota counts the launch, and the owner's limit goes in the override.
  check('agy-quota'),
  setup({ action: 'quota', daily_cap: 12, reserve_threshold: 6 }),
  setup({ action: 'quota', daily_cap: 12, reserve_threshold: 13 }),
  setup({ action: 'quota', daily_cap: 0, reserve_threshold: 1 }),
  setup({ action: 'quota', daily_cap: 12 }),
  check('agy-quota'),
  // The launches that fail: a bad exit, an answer that is not a review, a program that is gone, a client that is not ready.
  check('critic:vibe-mistral-small', withVibe('vibe 1.2.3', { [MISTRAL]: probe(EMPTY, { stderr: 'boom\n' }, 3) }), { confirmed: true, force: true }),
  check('critic:vibe-mistral-small', withVibe('vibe 1.2.3', { [MISTRAL]: probe('no findings here') }), { confirmed: true, force: true }),
  check('critic:agy-gemini-flash-high', withAgy('agy 0.9', { [GEMINI]: 'exit 5' }), { confirmed: true, force: true }),
  check('critic:agy-gemini-flash-high', withAgy('agy 0.9', {}, "printf '%s\\n' 'Error: Please sign in to view available models.' >&2\nexit 1"), { confirmed: true, force: true }),
  check('critic:vibe-mistral-small', { remove: ['home/.vibe/agents/critic-mistral-small.toml'] }, { confirmed: true, force: true }),
  setup({}, { roots: DATA }),
]

// A read-only proof of a client without a critic is recorded as unproven; agy's waits for #178.
scenarios['setup-read-only'] = [
  check('read-only:claude'),
  check('read-only:codex'),
  check('read-only:hermes'),
  check('read-only:opencode'),
  check('read-only:claude'),
  check('read-only:claude', {}, { force: true }),
  check('read-only:agy-gemini-flash-high', {}, { confirmed: true, force: true }),
  setup({ action: 'list' }),
]

// The quota alone, before and after a launch, and a limit that routing does not read.
scenarios['setup-quota'] = [
  check('agy-quota'),
  setup({ action: 'quota', daily_cap: 30, reserve_threshold: 5 }),
  check('agy-quota'),
  setup({ action: 'quota', daily_cap: 30, reserve_threshold: 30 }),
  setup({ action: 'quota', daily_cap: 7, reserve_threshold: 8 }),
  setup({ action: 'quota', daily_cap: 1.5, reserve_threshold: 1 }),
  setup({ action: 'quota', daily_cap: 30, reserve_threshold: 5 }, { write: { [OVERRIDE]: '{"antigravity_quota":"text"}\n' } }),
  setup({ action: 'quota', daily_cap: 9, reserve_threshold: 2 }, { write: { [OVERRIDE]: '{"antigravity_quota":{"reserved_stages":["spec"],"daily_cap":50},"budget":{"vibe_flow_cap_usd":1}}\n' } }),
  setup({}),
  setup({ action: 'quota', daily_cap: 9, reserve_threshold: 2 }, { write: { [OVERRIDE]: '{nope' } }),
  // Routing with no Antigravity client has no quota check.
  setup({ action: 'quota', daily_cap: 9, reserve_threshold: 2 }, {
    write: { [OVERRIDE]: `${JSON.stringify({ clients: { 'agy-gemini-flash-high': null } })}\n` },
  }),
]

// The kill switch: set, read back, kept beside the other keys, and refused over a file that is not an object.
scenarios['setup-kill-switch'] = [
  check('kill-switch'),
  setup({ action: 'kill-switch', disabled: true }),
  check('kill-switch'),
  setup({ action: 'kill-switch', disabled: false }),
  setup({ action: 'kill-switch', disabled: true }, { write: { [CONFIG]: '{"other":1,"disabled":false}\n' } }),
  check('kill-switch', { env: { CRITIQUE_OFF: '1' } }, { force: true }),
  setup({ action: 'kill-switch', disabled: false }, { env: { CRITIQUE_OFF: '1' } }),
  setup({ action: 'kill-switch', disabled: false }, { write: { [CONFIG]: '[1]\n' } }),
  setup({ action: 'kill-switch', disabled: false }, { write: { [CONFIG]: 'nope' } }),
  check('kill-switch', { write: { [CONFIG]: '{nope' } }, { force: true }),
  check('kill-switch', { write: { [CONFIG]: '{"disabled":"yes"}' }, roots: DATA }, { force: true }),
  setup({ action: 'kill-switch' }),
  setup({ action: 'kill-switch', disabled: 'yes' }),
  setup({}),
]

// The hooks: where they are wired, whether their scripts exist, and Codex's trust in each at its position.
const HOOKS = [
  { event: 'PreToolUse', matcher: 'Bash', name: 'critique-commit-gate' },
  { event: 'PreToolUse', matcher: 'Bash', name: 'critique-pr-gate' },
  { event: 'SessionStart', name: 'critique-session-start' },
]
const handler = (name, extra = {}) => ({ command: `hooks/${name}.sh`, type: 'command', ...extra })
const sortKeys = (value) =>
  Array.isArray(value) ? value.map(sortKeys) : value !== null && typeof value === 'object' ? Object.fromEntries(Object.keys(value).sort().map((key) => [key, sortKeys(value[key])])) : value
/** The hash Codex trusts a hook by, as Codex 0.166.0 computes it. */
const trustHash = (event, matcher, hook) => {
  const label = event === 'PreToolUse' ? 'pre_tool_use' : 'session_start'
  const normalized = {
    async: hook.async === true,
    command: hook.command,
    timeout: typeof hook.timeout === 'number' ? Math.max(1, hook.timeout) : 600,
    type: 'command',
    ...(typeof hook.statusMessage === 'string' ? { statusMessage: hook.statusMessage } : {}),
    ...(typeof hook.additionalContextLimit === 'number' && hook.additionalContextLimit !== 2500 ? { additionalContextLimit: hook.additionalContextLimit } : {}),
  }
  return `sha256:${createHash('sha256').update(JSON.stringify(sortKeys({ event_name: label, hooks: [normalized], ...(matcher === undefined ? {} : { matcher }) }))).digest('hex')}`
}
const hooksFile = (groups) => `${JSON.stringify({ hooks: groups })}\n`
const wired = (extra = {}) => {
  const groups = {}
  for (const hook of HOOKS) groups[hook.event] = [...(groups[hook.event] ?? []), { ...(hook.matcher === undefined ? {} : { matcher: hook.matcher }), hooks: [handler(hook.name, extra[hook.name])] }]
  return groups
}
const SCRIPTS = Object.fromEntries(HOOKS.map((hook) => [`work/hooks/${hook.name}.sh`, '#!/bin/sh\n']))
const trust = (file, states) =>
  states.map(([index, event, hash, enabled]) => `[hooks.state."${file}:${event}:${index}:0"]\n${hash === null ? '' : `trusted_hash = "${hash}"\n`}${enabled === undefined ? '' : `enabled = ${enabled}\n`}`).join('\n')
const hashes = (extra) => HOOKS.map((hook) => trustHash(hook.event, hook.matcher, handler(hook.name, extra?.[hook.name])))
// A relative CODEX_HOME keeps the sandbox's path out of the trust file, whose text the check's input hashes.
const CODEX = 'codex-home/hooks.json'
const ENV = { env: { CODEX_HOME: 'codex-home' } }
const allTrusted = (extra) => {
  const [a, b, c] = hashes(extra)
  return trust(CODEX, [[0, 'pre_tool_use', a], [1, 'pre_tool_use', b], [0, 'session_start', c]])
}
scenarios['setup-hooks'] = [
  check('hooks-installed'),
  // Claude Code alone, with every hook.
  check('hooks-installed', { mkdir: ['home/.claude'], write: { 'home/.claude/settings.json': hooksFile(wired()), ...SCRIPTS } }, { force: true }),
  // A hook missing, a script missing, a file that does not parse, then no file at all.
  check('hooks-installed', { write: { 'home/.claude/settings.json': hooksFile({ PreToolUse: wired().PreToolUse.slice(0, 1) }) } }, { force: true }),
  check('hooks-installed', { write: { 'home/.claude/settings.json': hooksFile(wired({ 'critique-pr-gate': { command: 'hooks/gone/critique-pr-gate.sh' }, 'critique-session-start': { command: 'hooks/gone/critique-session-start.sh' } })) } }, { force: true }),
  check('hooks-installed', { write: { 'home/.claude/settings.json': hooksFile({ PreToolUse: [{ matcher: 'Edit', hooks: [handler('critique-commit-gate')] }], SessionStart: 5 }) } }, { force: true }),
  check('hooks-installed', { write: { 'home/.claude/settings.json': '{nope' } }, { force: true }),
  check('hooks-installed', { remove: ['home/.claude/settings.json'] }, { force: true }),
  // Codex: untrusted, modified, disabled, and trusted at the position each now holds.
  check('hooks-installed', { ...ENV, mkdir: ['work/codex-home'], write: { 'work/codex-home/hooks.json': hooksFile(wired()), 'home/.claude/settings.json': hooksFile(wired()) } }, { force: true }),
  check('hooks-installed', { ...ENV, write: { 'work/codex-home/config.toml': allTrusted().replace('trusted_hash = "sha256:', 'trusted_hash = "sha256:0000') } }, { force: true }),
  check('hooks-installed', { ...ENV, write: { 'work/codex-home/config.toml': `${allTrusted()}\n[hooks.state."${CODEX}:session_start:0:0"]\nenabled = false\n` } }, { force: true }),
  check('hooks-installed', { ...ENV, write: { 'work/codex-home/config.toml': `${allTrusted()}\n[hooks.state."${CODEX}:pre_tool_use:0:0"]\nenabled = false\n[hooks.state."${CODEX}:pre_tool_use:1:0"]\nenabled = falsey\n` } }, { force: true }),
  check('hooks-installed', { ...ENV, write: { 'work/codex-home/config.toml': allTrusted() } }, { force: true }),
  check('hooks-installed', ENV, { force: false }),
  // Codex normalizes a handler before it hashes it: its timeout, status message and context limit count.
  check(
    'hooks-installed',
    {
      ...ENV,
      write: {
        'work/codex-home/hooks.json': hooksFile(wired({ 'critique-commit-gate': { timeout: 0, statusMessage: 'gating', additionalContextLimit: 100, async: true }, 'critique-session-start': { additionalContextLimit: 2500, timeout: 30 } })),
        'work/codex-home/config.toml': allTrusted({ 'critique-commit-gate': { timeout: 0, statusMessage: 'gating', additionalContextLimit: 100, async: true }, 'critique-session-start': { additionalContextLimit: 2500, timeout: 30 } }),
      },
    },
    { force: true },
  ),
  // Where the homes are can be set by the environment.
  check('hooks-installed', { env: { CLAUDE_CONFIG_DIR: `${SB}/elsewhere/claude`, CODEX_HOME: `${SB}/elsewhere/codex` }, mkdir: ['elsewhere/claude', 'elsewhere/codex'] }, { force: true }),
  check('hooks-installed', { env: { CLAUDE_CONFIG_DIR: `${SB}/elsewhere/claude`, CODEX_HOME: `${SB}/elsewhere/codex` }, write: { 'elsewhere/claude/settings.json': hooksFile(wired()) } }, { force: true }),
  setup({}, { env: { CLAUDE_CONFIG_DIR: null, CODEX_HOME: null } }),
]

for (const [name, steps] of Object.entries(scenarios)) {
  mkdirSync(join(fixtures, name), { recursive: true })
  writeFileSync(join(fixtures, name, 'scenario.json'), `${JSON.stringify({ assets: true, steps }, null, 2)}\n`)
}
