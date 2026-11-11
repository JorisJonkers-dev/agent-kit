#!/usr/bin/env node
// Regenerates every tests/fixtures/<scenario>/expected.json by running the node
// runner (skills/workstation/build-tracker/build-tracker.mjs) over the
// scenario's inputs. The Rust tests replay the same scenarios through the
// library and compare with these files, which is the proof of output parity:
// stdout, stderr, exit code, the state file bytes and the page bytes.
//
//   node scripts/crates/build-tracker/tests/regenerate-fixtures.mjs [scenario ...]
//
// A scenario is a directory with:
//   scenario.json  { "repo": "widgets", "remote": "<url>" | null, "steps": [...] }
//   repo/          files copied into the git repository the steps run in
//   state/         files copied into the state home ($XDG_STATE_HOME)
//   outside/       files copied beside the repository, outside any git repository
//   gh/            what the fake gh (fixtures/bin/gh) answers; a step may name another folder
// A step is { "argv": [...], "cwd": "<under the sandbox>", "gh": "<folder>", "env": {...},
// "remove": [...], "mkdir": [...], "write": { "<path>": "<text>" } }; the paths are
// under the sandbox, and @SANDBOX@ in argv or text is the sandbox's path. The clock is
// fixed at 2026-11-03T12:00:00Z (fake-clock.mjs), and the sandbox's path is written
// @SANDBOX@ in every output, so the goldens do not depend on where they were made.
import { spawnSync } from 'node:child_process'
import { cpSync, existsSync, mkdirSync, mkdtempSync, readdirSync, readFileSync, realpathSync, rmSync, statSync, writeFileSync } from 'node:fs'
import { tmpdir } from 'node:os'
import { dirname, join, relative } from 'node:path'
import { fileURLToPath } from 'node:url'

const here = dirname(fileURLToPath(import.meta.url))
const fixtures = join(here, 'fixtures')
const runner = join(here, '..', '..', '..', '..', 'skills', 'workstation', 'build-tracker', 'build-tracker.mjs')
const FIXED_NOW = '2026-11-03T12:00:00Z'

const gitEnv = {
  GIT_CONFIG_GLOBAL: '/dev/null',
  GIT_CONFIG_NOSYSTEM: '1',
  GIT_AUTHOR_NAME: 'fixture',
  GIT_AUTHOR_EMAIL: 'fixture@example.invalid',
  GIT_COMMITTER_NAME: 'fixture',
  GIT_COMMITTER_EMAIL: 'fixture@example.invalid',
}

function git(cwd, ...args) {
  const result = spawnSync('git', args, { cwd, env: { ...gitEnv, PATH: process.env.PATH ?? '' }, encoding: 'utf8' })
  if (result.status !== 0) throw new Error(`git ${args.join(' ')}: ${result.stderr}`)
}

function copyIfPresent(from, to) {
  if (existsSync(from)) cpSync(from, to, { recursive: true })
}

/** Every file and directory under `root`, as `path -> text` (a directory is `path/ -> null`). */
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

function normalise(text, sandbox) {
  return text.split(sandbox).join('@SANDBOX@')
}

function runScenario(name) {
  const dir = join(fixtures, name)
  const scenario = JSON.parse(readFileSync(join(dir, 'scenario.json'), 'utf8'))
  const sandbox = realpathSync(mkdtempSync(join(tmpdir(), 'build-tracker-fixture-')))
  try {
    const repo = join(sandbox, 'work', scenario.repo ?? 'widgets')
    mkdirSync(repo, { recursive: true })
    mkdirSync(join(sandbox, 'home'))
    copyIfPresent(join(dir, 'repo'), repo)
    copyIfPresent(join(dir, 'state'), join(sandbox, 'state'))
    copyIfPresent(join(dir, 'outside'), join(sandbox, 'outside'))
    git(repo, 'init', '-q')
    if (scenario.remote) git(repo, 'remote', 'add', 'origin', scenario.remote)
    const steps = scenario.steps.map((step) => {
      for (const path of step.remove ?? []) rmSync(join(sandbox, path), { recursive: true, force: true })
      for (const path of step.mkdir ?? []) mkdirSync(join(sandbox, path), { recursive: true })
      for (const [path, text] of Object.entries(step.write ?? {})) {
        mkdirSync(dirname(join(sandbox, path)), { recursive: true })
        writeFileSync(join(sandbox, path), text.split('@SANDBOX@').join(sandbox))
      }
      const env = {
        ...gitEnv,
        PATH: `${join(fixtures, 'bin')}:${process.env.PATH ?? ''}`,
        HOME: join(sandbox, 'home'),
        XDG_STATE_HOME: join(sandbox, 'state'),
        GH_FIXTURE: join(dir, step.gh ?? 'gh'),
        FIXED_NOW,
      }
      for (const [key, value] of Object.entries(step.env ?? {})) {
        if (value === null) delete env[key]
        else env[key] = value.split('@SANDBOX@').join(sandbox)
      }
      const argv = step.argv.map((arg) => arg.split('@SANDBOX@').join(sandbox))
      const result = spawnSync(process.execPath, ['--import', join(fixtures, 'fake-clock.mjs'), runner, ...argv], {
        cwd: join(sandbox, step.cwd ?? join('work', scenario.repo ?? 'widgets')),
        env,
        encoding: 'utf8',
      })
      const files = snapshot(sandbox, ['state', 'out'])
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
