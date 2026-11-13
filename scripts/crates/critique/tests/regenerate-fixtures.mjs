#!/usr/bin/env node
// Regenerates every tests/fixtures/<scenario>/expected.json by running the node
// runner (skills/workstation/critique/critique.mjs) over the scenario's inputs.
// The Rust tests replay the same scenarios through the library and compare with
// these files: stdout, stderr, exit code and every file under the sandbox, byte
// for byte.
//
//   node scripts/crates/critique/tests/regenerate-fixtures.mjs [scenario ...]
//
// A scenario is a directory with scenario.json { "steps": [...] }. A step is
// { "argv": [...], "now": "<ISO instant>", "env": {...}, "remove": [...], "mkdir": [...],
//   "write": { "<path>": "<text>" }, "exec": [...], "shims": [...], "path": [...], "git": [[...]],
//   "roots": [...], "stdin": "<text>" }; paths are under the sandbox, @SANDBOX@ in argv, env or text is the
// sandbox's path, and a null env value unsets the variable. HOME is sandbox/home, the
// working directory sandbox/work, and the clock is fixed (fake-clock.mjs) at the step's "now".
// The sandbox's path is written @SANDBOX@ in every output.
import { spawnSync } from 'node:child_process'
import { chmodSync, cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, symlinkSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, 'fixtures')
const runner = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'critique', 'critique.mjs')
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

/** The sandbox's path is @SANDBOX@ in every output, and the folder the runner and its shipped files sit in is /assets. */
const normalise = (text, sandbox) => text.split(sandbox).join('@SANDBOX@').split(dirname(runner)).join('/assets')

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
  const sandbox = realpathSync(mkdtempSync(join(tmpdir(), 'critique-fixture-')))
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
      for (const line of step.git ?? []) {
        // Leading NAME=value words set that variable for the command (a committer date, say).
        const found = line.findIndex((word) => !/^[A-Z_]+=/.test(word))
        const assigned = found === -1 ? line.length : found
        const extra = Object.fromEntries(line.slice(0, assigned).map((word) => [word.slice(0, word.indexOf('=')), word.slice(word.indexOf('=') + 1).split('@SANDBOX@').join(sandbox)]))
        const argv = line.slice(assigned)
        const made = spawnSync('git', [...GIT_IDENTITY, ...argv.map((arg) => arg.split('@SANDBOX@').join(sandbox))], { cwd: sandbox, env: { ...gitEnv(sandbox), ...extra }, encoding: 'utf8' })
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
        input: (step.stdin ?? '').split('@SANDBOX@').join(sandbox),
        encoding: 'utf8',
      })
      const files = snapshot(sandbox, ['state', 'home', 'work', ...(step.roots ?? [])])
      return {
        exit: result.status,
        stdout: normalise(result.stdout, sandbox),
        stderr: normalise(result.stderr, sandbox),
        files: Object.fromEntries(Object.entries(files).map(([path, text]) => [normalise(path, sandbox), text === null ? null : normalise(text, sandbox)])),
      }
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
