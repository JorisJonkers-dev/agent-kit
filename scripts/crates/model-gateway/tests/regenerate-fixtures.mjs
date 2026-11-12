#!/usr/bin/env node
// Regenerates every tests/fixtures/<scenario>/expected.json by running the node
// runner (skills/workstation/model-gateway/model-gateway.mjs) over the scenario's
// inputs. The Rust tests replay the same scenarios through the library and compare
// with these files: stdout, stderr, exit code and the state files, byte for byte.
//
//   node scripts/crates/model-gateway/tests/regenerate-fixtures.mjs [scenario ...]
//
// A scenario is a directory with scenario.json { "steps": [...] } and optionally
// state/ (files copied into the state home, $XDG_STATE_HOME). A step is
// { "argv": [...], "now": "<ISO instant>", "env": {...}, "remove": [...], "mkdir": [...],
//   "write": { "<path>": "<text>" } }; paths are under the sandbox, @SANDBOX@ in argv, env
// or text is the sandbox's path, and a null env value unsets the variable. "exec" lists
// the written files (sandbox paths) to make executable; "shims" names programs of the host's
// PATH to link into the sandbox's bin/; "path" lists the sandbox directories that become
// PATH (else the host's own). Fake client programs are scripts written under bin/. The clock is
// fixed (fake-clock.mjs) at the step's "now", else 2026-11-03T12:00:00Z, and so is the monotonic
// reading (always 0, so a run's duration is 0); the sandbox's path is written @SANDBOX@ in every output.
// "git" lists git command lines (each an argv array) run with the host's git in the sandbox before
// the step, with a fixed identity and instants, to make the repositories a run works in. A step
// may also name "roots": sandbox directories snapshotted besides state/, home/ and work/.
// The random parts of a run id (@RAND1@, @RAND2@, ... in order of first appearance) and of the
// temporary directories a run makes (@TMP1@, ...) are masked per step, in file names too.
import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, 'fixtures')
const runner = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'model-gateway', 'model-gateway.mjs')
const FIXED_NOW = '2026-11-03T12:00:00Z'

/** Every file and directory under the roots, as `path -> text` (a directory is `path/ -> null`). */
function snapshot(sandbox, roots) {
  const files = {}
  const walk = (path) => {
    if (!existsSync(path)) return
    const rel = relative(sandbox, path)
    if (statSync(path).isDirectory()) {
      files[`${rel}/`] = null
      for (const name of readdirSync(path).sort()) walk(join(path, name))
    } else files[rel] = readFileSync(path, 'utf8')
  }
  for (const root of roots) walk(join(sandbox, root))
  return files
}

/**
 * The sandbox's path is @SANDBOX@, the random suffix of a grant id is @RAND@, and the change time and
 * inode of the client program a Vibe probe's cache entry names are @N@ (they differ from file to file).
 */
const normalise = (text, sandbox) =>
  text
    .split(sandbox)
    .join('@SANDBOX@')
    .replace(/(grant-\d{8}T\d{6}Z)-[0-9a-f]{6}/g, '$1-@RAND@')
    .replace(/("binary_(?:ctime_ms|ino)":)[0-9.eE+-]+/g, '$1@N@')

const RUN_ID = /(\d{8}T\d{6}Z)-([0-9a-f]{6})/g
const TMP_DIR = /(model-gateway-(?:run|worktree)-)([A-Za-z0-9]{6})/g
const LEDGER_FILE = 'state/agent-kit/model-gateway/ledger.jsonl'

/**
 * Numbers the random suffixes of run ids and temporary directories by first appearance, scanning
 * stdout, stderr, the ledger, then every other file in name order, and rewrites the output, the
 * file names and the file texts with the numbers. Two runs of one second then keep their own names.
 */
function maskRandoms({ stdout, stderr, files }) {
  const run = new Map()
  const tmp = new Map()
  const number = (map, found) => {
    if (!map.has(found)) map.set(found, map.size + 1)
    return map.get(found)
  }
  const scan = (text) => {
    if (text === null) return
    for (const match of text.matchAll(RUN_ID)) number(run, match[2])
    for (const match of text.matchAll(TMP_DIR)) number(tmp, match[2])
  }
  const order = Object.keys(files).sort()
  scan(stdout)
  scan(stderr)
  if (LEDGER_FILE in files) scan(files[LEDGER_FILE])
  for (const name of order) scan(files[name])
  for (const name of order) scan(name)
  const mask = (text) =>
    text
      .replace(RUN_ID, (_, stamp, suffix) => `${stamp}-@RAND${number(run, suffix)}@`)
      .replace(TMP_DIR, (_, prefix, suffix) => `${prefix}@TMP${number(tmp, suffix)}@`)
  return {
    stdout: mask(stdout),
    stderr: mask(stderr),
    // In the order of the masked names when there are random parts, so the golden does not depend on them.
    files: Object.fromEntries(
      Object.entries(files)
        .map(([name, text]) => [mask(name), text === null ? null : mask(text)])
        .sort(([left], [right]) => (run.size + tmp.size === 0 ? 0 : left < right ? -1 : left > right ? 1 : 0)),
    ),
  }
}

/** The first program of that name on the host's PATH. */
function hostProgram(name) {
  for (const dir of (process.env.PATH ?? '').split(':')) {
    if (dir !== '' && existsSync(join(dir, name))) return join(dir, name)
  }
  throw new Error(`no ${name} on PATH`)
}

/** The environment the git command lines of a step run in: a fixed identity and instants, no configuration of the host's. */
function gitEnv(sandbox) {
  return {
    PATH: process.env.PATH ?? '',
    HOME: join(sandbox, 'home'),
    GIT_CONFIG_NOSYSTEM: '1',
    GIT_AUTHOR_DATE: FIXED_NOW,
    GIT_COMMITTER_DATE: FIXED_NOW,
    GIT_TERMINAL_PROMPT: '0',
  }
}
const GIT_IDENTITY = ['-c', 'user.name=Fixture', '-c', 'user.email=fixture@example.invalid', '-c', 'commit.gpgsign=false', '-c', 'init.defaultBranch=main']

function runScenario(name) {
  const dir = join(fixtures, name)
  const scenario = JSON.parse(readFileSync(join(dir, 'scenario.json'), 'utf8'))
  const sandbox = realpathSync(mkdtempSync(join(tmpdir(), 'model-gateway-fixture-')))
  try {
    mkdirSync(join(sandbox, 'home'))
    mkdirSync(join(sandbox, 'work'))
    if (existsSync(join(dir, 'state'))) cpSync(join(dir, 'state'), join(sandbox, 'state'), { recursive: true })
    const steps = scenario.steps.map((step) => {
      for (const path of step.remove ?? []) rmSync(join(sandbox, path), { recursive: true, force: true })
      for (const path of step.mkdir ?? []) mkdirSync(join(sandbox, path), { recursive: true })
      for (const [path, text] of Object.entries(step.write ?? {})) {
        mkdirSync(dirname(join(sandbox, path)), { recursive: true })
        writeFileSync(join(sandbox, path), text.split('@SANDBOX@').join(sandbox))
      }
      for (const path of step.exec ?? []) chmodSync(join(sandbox, path), 0o755)
      for (const name of step.shims ?? []) {
        mkdirSync(join(sandbox, 'bin'), { recursive: true })
        rmSync(join(sandbox, 'bin', name), { force: true })
        symlinkSync(hostProgram(name), join(sandbox, 'bin', name))
      }
      for (const argv of step.git ?? []) {
        const made = spawnSync('git', [...GIT_IDENTITY, ...argv.map((arg) => arg.split('@SANDBOX@').join(sandbox))], { cwd: sandbox, env: gitEnv(sandbox), encoding: 'utf8' })
        if (made.status !== 0) throw new Error(`${name}: git ${argv.join(' ')}: ${made.stderr}`)
      }
      const env = {
        PATH: step.path === undefined ? process.env.PATH ?? '' : step.path.map((dir) => join(sandbox, dir)).join(':'),
        HOME: join(sandbox, 'home'),
        XDG_STATE_HOME: join(sandbox, 'state'),
        FIXED_NOW: step.now ?? FIXED_NOW,
        TZ: 'UTC',
      }
      for (const [key, value] of Object.entries(step.env ?? {})) {
        if (value === null) delete env[key]
        else env[key] = value.split('@SANDBOX@').join(sandbox)
      }
      const argv = step.argv.map((arg) => arg.split('@SANDBOX@').join(sandbox))
      const result = spawnSync(process.execPath, ['--import', join(fixtures, 'fake-clock.mjs'), runner, ...argv], {
        cwd: join(sandbox, 'work'),
        env,
        encoding: 'utf8',
      })
      const files = snapshot(sandbox, ['state', 'home', 'work', ...(step.roots ?? [])])
      const masked = maskRandoms({
        stdout: normalise(result.stdout, sandbox),
        stderr: normalise(result.stderr, sandbox),
        files: Object.fromEntries(Object.entries(files).map(([path, text]) => [normalise(path, sandbox), text === null ? null : normalise(text, sandbox)])),
      })
      return { exit: result.status, ...masked }
    })
    writeFileSync(join(dir, 'expected.json'), `${JSON.stringify({ steps }, null, 2)}\n`)
    console.log(`${name}: ${steps.length} steps`)
  } finally {
    rmSync(sandbox, { recursive: true, force: true })
  }
}

const wanted = process.argv.slice(2)
const names = readdirSync(fixtures).filter((entry) => existsSync(join(fixtures, entry, 'scenario.json')))
for (const name of wanted.length > 0 ? wanted : names) runScenario(name)
