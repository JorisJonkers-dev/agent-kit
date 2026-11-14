// What a confined Pi launch needs in a scenario's sandbox, shared by the fan-out, refute and setup generators: a fake
// `agent-launcher` that answers `confine --check`, keeps a launch's grants beside the packet (`launcher.txt`) and runs
// what follows `--`; a fake secret store that hands out `sk-<NAME>`; and the home's Pi providers, whose keys it reads.
// The fake `pi` adds AGENT to keep what its agent directory held while it ran (`agent-models.json`, `agent-env.txt`).

const q = (text) => `'${text.split("'").join("'\\''")}'`

/** The launcher; `refuse` maps a packet directory suffix to the stderr line of a confinement it cannot apply (exit 125). */
export const fakeLauncher = (refuse = {}) =>
  `#!/bin/sh\ncase " $* " in *" --check "*) printf '%s\\n' 'confine: ok'; exit 0;; esac\ncase "$PWD" in\n${Object.entries(refuse)
    .map(([where, said]) => `*${where}/packet) printf '%s\\n' ${q(said)} >&2; exit 125;;`)
    .join('\n')}\n*) ;;\nesac\nprintf '%s\\n' "$@" > "$PWD/../launcher.txt"\nwhile [ "$1" != "--" ]; do shift; done\nshift\nexec "$@"\n`

/** A launcher whose `--check` says this machine cannot confine a critic. */
export const refusingLauncher = `#!/bin/sh\nprintf '%s\\n' 'agent-launcher: confine: the confinement cannot be applied here: Landlock ABI 3 cannot limit the network; nothing started' >&2\nexit 125\n`

const SECRET = `#!/bin/sh\nprintf 'sk-%s\\n' "$2"\n`

const provider = (route, model) => ({
  api: 'openai-completions',
  apiKey: `!@SANDBOX@/bin/agent-kit-secret get LITELLM_${route.toUpperCase().split('-').join('_')}_KEY`,
  baseUrl: 'https://litellm.example.test/v1',
  models: [{ cost: { input: 0.5, output: 1 }, id: model }],
})

const HOME_MODELS = `${JSON.stringify(
  { providers: { 'litellm-opencode-go': provider('opencode-go', 'deepseek-v4-flash'), 'litellm-vibe-study': provider('vibe-study', 'glm-5-3') } },
  null,
  2,
)}\n`

/** The files to write into the sandbox, and the ones of them to make executable. */
export const confineWrite = (refuse) => ({ 'bin/agent-launcher': fakeLauncher(refuse), 'bin/agent-kit-secret': SECRET, 'home/.pi/agent/models.json': HOME_MODELS })
export const CONFINE_EXEC = ['bin/agent-launcher', 'bin/agent-kit-secret']

/** The fake pi's lines that keep what its agent directory held. */
export const AGENT = `cat "$PI_CODING_AGENT_DIR/models.json" > "$PWD/../agent-models.json"\nprintf '%s\\n' "$PI_CODING_AGENT_DIR" "$TMPDIR" > "$PWD/../agent-env.txt"\n`
