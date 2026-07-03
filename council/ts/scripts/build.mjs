import { createHash } from 'node:crypto'
import { spawn } from 'node:child_process'
import { chmod, copyFile, mkdir, readdir, readFile, rm, stat, writeFile } from 'node:fs/promises'
import { basename, dirname, relative, resolve } from 'node:path'
import process from 'node:process'

const packageRoot = resolve(import.meta.dirname, '..')
const councilRoot = resolve(packageRoot, '..')
const sourceRoot = resolve(packageRoot, 'src')
const distDirName = 'ts-dist'
const distDir = resolve(councilRoot, distDirName)
const launcherPath = resolve(councilRoot, 'council.mjs')
const inventoryPath = resolve(packageRoot, 'toolkit-files.json')
const tsc = resolve(packageRoot, 'node_modules/typescript/bin/tsc')

await rm(distDir, { force: true, recursive: true })
await run(process.execPath, [tsc, '-p', resolve(packageRoot, 'tsconfig.build.json')])
await copyRuntimeAssets(sourceRoot, distDir)
await writeRuntimePackageJson()
await writeLauncher()
await writeInventory()

async function run(command, args) {
  await new Promise((resolvePromise, reject) => {
    const child = spawn(command, args, { cwd: packageRoot, stdio: 'inherit' })
    child.on('error', reject)
    child.on('close', (code) => {
      if (code === 0) {
        resolvePromise()
      } else {
        reject(new Error(`${basename(command)} exited with code ${String(code)}`))
      }
    })
  })
}

async function copyRuntimeAssets(sourceDir, targetDir) {
  for (const path of await walk(sourceDir)) {
    if (path.endsWith('.ts') || path.endsWith('.tsx') || path.endsWith('.map')) continue
    const rel = relative(sourceDir, path)
    const target = resolve(targetDir, rel)
    await mkdir(dirname(target), { recursive: true })
    await copyFile(path, target)
  }
}

async function writeRuntimePackageJson() {
  await writeFile(
    resolve(distDir, 'package.json'),
    `${JSON.stringify(
      {
        private: true,
        type: 'module',
        engines: { node: '>=22' },
      },
      null,
      2,
    )}\n`,
  )
}

async function writeLauncher() {
  await writeFile(
    launcherPath,
    `#!/usr/bin/env node
import { runCli } from './${distDirName}/cli/index.js'

const result = await runCli(process.argv.slice(2))
if (result.stdout) process.stdout.write(result.stdout)
if (result.stderr) process.stderr.write(result.stderr)
process.exitCode = result.exitCode
`,
  )
  await chmod(launcherPath, 0o755)
}

async function writeInventory() {
  const files = []
  for (const path of await shippedFiles()) {
    const rel = relative(councilRoot, path).split('/').join('/')
    const fileStat = await stat(path)
    files.push({
      path: rel,
      mode: rel === 'council.mjs' ? '0755' : '0644',
      sha256: await sha256(path),
      bytes: fileStat.size,
    })
  }
  files.sort((left, right) => left.path.localeCompare(right.path))
  await writeFile(
    inventoryPath,
    `${JSON.stringify(
      {
        generatedBy: 'council/ts/scripts/build.mjs',
        root: 'council',
        files,
      },
      null,
      2,
    )}\n`,
  )
}

async function shippedFiles() {
  const files = []
  for (const path of await walk(councilRoot, { skipSourcePackage: true })) {
    const rel = relative(councilRoot, path).split('/').join('/')
    if (!isShippedToolkitFile(rel)) continue
    files.push(path)
  }
  return files
}

function isShippedToolkitFile(rel) {
  const parts = rel.split('/')
  if (rel === 'README.md') return false
  if (rel === 'council.mjs' || rel === 'council.toml') return true
  if (rel.endsWith('.map') || rel.endsWith('.pyc') || rel.endsWith('.tsbuildinfo')) return false
  if (parts.includes('__pycache__') || parts.includes('node_modules') || parts.includes('coverage')) return false
  return parts[0] === 'prompts' || parts[0] === 'schemas' || parts[0] === distDirName
}

async function walk(root, options = {}) {
  const entries = await readdir(root, { withFileTypes: true })
  const files = []
  for (const entry of entries) {
    const path = resolve(root, entry.name)
    if (entry.isDirectory()) {
      if (shouldSkipDirectory(entry.name, options)) continue
      files.push(...(await walk(path, options)))
    } else if (entry.isFile()) {
      files.push(path)
    }
  }
  return files
}

function shouldSkipDirectory(name, options) {
  if (name === 'node_modules' || name === 'coverage' || name === '__pycache__') return true
  return options.skipSourcePackage === true && name === 'ts'
}

async function sha256(path) {
  return createHash('sha256').update(await readFile(path)).digest('hex')
}
