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
// fixed (fake-clock.mjs) at the step's "now", else 2026-11-03T12:00:00Z; the sandbox's
// path is written @SANDBOX@ in every output.
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

/** The first program of that name on the host's PATH. */
function hostProgram(name) {
  for (const dir of (process.env.PATH ?? '').split(':')) {
    if (dir !== '' && existsSync(join(dir, name))) return join(dir, name)
  }
  throw new Error(`no ${name} on PATH`)
}

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
      const files = snapshot(sandbox, ['state', 'home', 'work'])
      return {
        exit: result.status,
        stdout: normalise(result.stdout, sandbox),
        stderr: normalise(result.stderr, sandbox),
        files: Object.fromEntries(Object.entries(files).map(([path, text]) => [path, text === null ? null : normalise(text, sandbox)])),
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
