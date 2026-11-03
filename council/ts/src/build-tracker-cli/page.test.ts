import { spawn } from 'node:child_process'
import { createHash } from 'node:crypto'
import { existsSync } from 'node:fs'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, join } from 'node:path'

import { afterEach, describe, expect, it, vi } from 'vitest'

import { defaultBuildTrackerRuntime, runBuildTrackerCli } from './index.js'

/**
 * The artifact page contract, checked on a rendered page: the static rules
 * by reading it, the width rule in a headless browser.
 */

const cleanups: (() => Promise<void>)[] = []

afterEach(async () => {
  await Promise.all(cleanups.splice(0).map((cleanup) => cleanup()))
})

const LONG = 'an_unbreakable_token_like_a_commit_sha_0123456789abcdef0123456789abcdef0123456789abcdef'

/** A state that tries to push the page wide: long words everywhere, many tickets, milestones. */
function wideState(): Record<string, unknown> {
  const tickets: Record<string, unknown> = {}
  for (let n = 1; n <= 30; n += 1) {
    tickets[String(n)] = {
      blocked_by: n === 1 ? [] : [n - 1, ...(n > 2 ? [1] : [])],
      live: n % 10 === 0,
      note: n % 3 === 0 ? `see ${LONG}` : undefined,
      status: ['merged', 'running', 'review', 'owner', 'todo'][n % 5],
      title: n % 4 === 0 ? `ticket with a ${LONG} in its title` : `ticket number ${String(n)}`,
    }
  }
  return {
    milestones: [
      { closes: 10, name: `First milestone ${LONG}` },
      { closes: 20, name: 'Second milestone' },
    ],
    spec: 'acme/widgets#100',
    subtitle: `integration branch feat/${LONG}`,
    tickets,
    title: `Widgets build ${LONG}`,
    version: 1,
  }
}

/**
 * The sha384 of each Mermaid version's `mermaid.min.js`, as the online check
 * found it on the CDN. To move to a new version: change the pin in
 * `page.ts`, run the online check, and add the version with its hash here.
 */
const VERIFIED_MERMAID: Readonly<Record<string, string>> = {
  '11.4.1': 'sha384-rbtjAdnIQE/aQJGEgXrVUlMibdfTSa4PQju4HDhN3sR2PmaKFzhEafuePsl9H/9I',
}

/** The page's Mermaid script tag: its URL, the version in that URL, and its integrity hash. */
function mermaidScript(page: string): { integrity: string; src: string; version: string } {
  const tag = /<script src="([^"]*\/mermaid@([^/"]+)\/dist\/mermaid\.min\.js)" integrity="([^"]*)"/.exec(page)
  if (tag === null) throw new Error('the page has no Mermaid script tag with an integrity hash')
  return { integrity: String(tag[3]), src: String(tag[1]), version: String(tag[2]) }
}

async function renderedPage(state: Record<string, unknown>): Promise<{ dir: string; page: string; path: string }> {
  const dir = await mkdtemp(join(tmpdir(), 'build-tracker-page-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  await writeFile(join(dir, 'state.json'), JSON.stringify(state))
  const path = join(dir, 'page.html')
  const result = await runBuildTrackerCli(['render', '--state', join(dir, 'state.json'), '--out', path], defaultBuildTrackerRuntime())
  expect(result.exitCode).toBe(0)
  return { dir, page: await readFile(path, 'utf8'), path }
}

describe('build tracker page contract', () => {
  it('has a two-to-four-word title', async () => {
    const { page } = await renderedPage(wideState())

    const title = /<title>([^<]*)<\/title>/.exec(page)?.[1] ?? ''
    expect(title).toBe('Spec #100 build tracker')
    expect(title.split(/\s+/).length).toBeGreaterThanOrEqual(2)
    expect(title.split(/\s+/).length).toBeLessThanOrEqual(4)
  })

  it('defines colour tokens on :root, redefines them for dark mode both ways, and gives body a background', async () => {
    const { page } = await renderedPage(wideState())
    const style = /<style>([\s\S]*?)<\/style>/.exec(page)?.[1] ?? ''

    expect(style).toMatch(/^\s*:root \{[^}]*--bg:#f6f7f9;[^}]*--fg:#1d2b3a;/)
    expect(style).toContain('@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { --bg:#11161d;')
    expect(style).toContain(':root[data-theme="dark"] { --bg:#11161d;')
    expect(style).toMatch(/\nbody \{[^}]*background:var\(--bg\);/)
    expect(style).toMatch(/\nbody \{[^}]*padding-left:max\(16px,[^}]*padding-right:max\(16px,/)
  })

  it('loads scripts only from a pinned CDN version and stylesheets only from Google Fonts', async () => {
    const { page } = await renderedPage(wideState())

    const scripts = [...page.matchAll(/<script\b[^>]*\bsrc="([^"]+)"/g)].map((m) => String(m[1]))
    expect(scripts).toHaveLength(1)
    expect(scripts[0]).toMatch(/^https:\/\/cdn\.jsdelivr\.net\/npm\/mermaid@\d+\.\d+\.\d+\/dist\/mermaid\.min\.js$/)
    // A classic script with its integrity hash: it runs from a local file too, where a module would need CORS.
    expect(page).toContain(`<script src="${String(scripts[0])}" integrity="${mermaidScript(page).integrity}" crossorigin="anonymous"></script>`)
    const stylesheets = [...page.matchAll(/<link\b[^>]*\bhref="([^"]+)"/g)].map((m) => String(m[1]))
    expect(stylesheets).toHaveLength(1)
    expect(stylesheets[0]).toMatch(/^https:\/\/fonts\.googleapis\.com\/css2\?/)
    const external = [...page.matchAll(/(?:src|href)="(https?:[^"]+)"/g)].map((m) => String(m[1]))
    expect(external.filter((url) => !url.startsWith('https://github.com/acme/widgets/issues/'))).toEqual([...stylesheets, ...scripts])
  })

  it('loads a Mermaid version whose hash was checked against the CDN file', async () => {
    const { page } = await renderedPage(wideState())

    // A wrong hash makes every browser refuse the script, and nothing on the page says so.
    expect(VERIFIED_MERMAID[mermaidScript(page).version]).toBe(mermaidScript(page).integrity)
  })

  // Opt-in, as it downloads Mermaid from the CDN: BUILD_TRACKER_ONLINE=1 npx vitest run src/build-tracker-cli/page.test.ts
  it.skipIf(process.env.BUILD_TRACKER_ONLINE !== '1')('pins the sha384 of the Mermaid file the CDN serves', { timeout: 60_000 }, async () => {
    const { page } = await renderedPage(wideState())
    const { integrity, src } = mermaidScript(page)

    const response = await fetch(src, { signal: AbortSignal.timeout(50_000) })
    expect(response.status).toBe(200)
    const served = `sha384-${createHash('sha384').update(Buffer.from(await response.arrayBuffer())).digest('base64')}`
    expect(integrity).toBe(served)
  })

  it('starts Mermaid on the graph without waiting for the page to be published', async () => {
    const { page } = await renderedPage(wideState())

    expect(page).toContain("window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict'")
    expect(page).toContain("window.mermaid.run({ querySelector: 'pre.mermaid' });")
    expect(page.indexOf('mermaid.min.js')).toBeLessThan(page.indexOf('window.mermaid.run('))
  })

  it.each([
    ['skips the browser tests when no Chrome is found and none is required', undefined, false, true],
    ['runs the browser tests when Chrome is found', '/bin/chrome', false, false],
    ['runs the browser tests, to fail them, when Chrome is required and none is found', undefined, true, false],
  ])('%s', (_name, found, required, skip) => {
    expect(browserGate(found, required).skip).toBe(skip)
  })

  it('fails a browser test that needs Chrome when it is required and none is found', () => {
    expect(() => browserGate(undefined, true).browser()).toThrow(
      'BUILD_TRACKER_REQUIRE_CHROME=1, but no Chrome was found: set CHROME_BIN or put google-chrome on PATH',
    )
  })

  const gate = browserGate(findChrome(), process.env.BUILD_TRACKER_REQUIRE_CHROME === '1')

  it.skipIf(gate.skip)('does not scroll sideways at 375 CSS px wide in a headless browser', { timeout: 60_000 }, async () => {
    const { dir, path } = await renderedPage(wideState())

    // Offline, so the test never reaches the network: the graph stays as its source text.
    const probe = await measure(gate.browser(), dir, path, 375, [])

    expect(probe.clientWidth).toBeGreaterThan(300)
    expect(probe.clientWidth).toBeLessThanOrEqual(375)
    expect(probe.scrollWidth).toBeLessThanOrEqual(probe.clientWidth)
  })

  it.skipIf(gate.skip)('sees the sideways scroll of a page whose long words do not wrap', { timeout: 60_000 }, async () => {
    const { dir, page } = await renderedPage(wideState())
    const unwrapped = join(dir, 'unwrapped.html')
    expect(page).toContain(' overflow-wrap:anywhere;')
    await writeFile(unwrapped, page.replaceAll(' overflow-wrap:anywhere;', ''))

    const probe = await measure(gate.browser(), dir, unwrapped, 375, [])

    expect(probe.scrollWidth).toBeGreaterThan(probe.clientWidth)
  })

  it('leaves no kill timer behind when the browser stops before its dump', async () => {
    vi.useFakeTimers({ toFake: ['setTimeout', 'clearTimeout'] })
    try {
      await expect(dumpDom('/bin/sh', ['-c', 'exit 1'])).rejects.toThrow('before its DOM dump')
      expect(vi.getTimerCount()).toBe(0)
    } finally {
      vi.useRealTimers()
    }
  })

  it('reads the probe without waiting for the browser to shut down', { timeout: 20_000 }, async () => {
    const dir = await mkdtemp(join(tmpdir(), 'build-tracker-page-'))
    cleanups.push(() => rm(dir, { force: true, recursive: true }))
    // A browser that prints a complete dump, then hangs in teardown in a process of its own.
    const browser = join(dir, 'hanging-browser')
    const probe = '{&quot;scrollWidth&quot;:300,&quot;clientWidth&quot;:375,&quot;nodes&quot;:0}'
    await writeFile(browser, `#!/bin/sh\necho '<html><body data-probe="${probe}"></body></html>'\nsleep 30\n`, { mode: 0o755 })

    const started = Date.now()
    const measured = await measure(browser, dir, join(dir, 'page.html'), 375, [])

    expect(measured).toEqual({ clientWidth: 375, nodes: 0, scrollWidth: 300 })
    expect(Date.now() - started).toBeLessThan(10_000)
  })

  // Opt-in, as it fetches Mermaid from the CDN: BUILD_TRACKER_ONLINE=1 npx vitest run src/build-tracker-cli/page.test.ts
  it.skipIf(gate.skip || process.env.BUILD_TRACKER_ONLINE !== '1')(
    'renders its graph from a local file, one node per ticket, still without sideways scroll',
    { timeout: 60_000 },
    async () => {
      const { dir, page, path } = await renderedPage(wideState())

      // The Mermaid CDN only; the fonts stay offline.
      const probe = await measure(gate.browser(), dir, path, 375, [new URL(mermaidScript(page).src).host])

      expect(probe.nodes).toBe(30)
      expect(probe.scrollWidth).toBeLessThanOrEqual(probe.clientWidth)
    },
  )
})

interface Probe {
  readonly scrollWidth: number
  readonly clientWidth: number
  readonly nodes: number
}

/**
 * Opens the page in an iframe of `width` CSS px (a headless window has a
 * larger minimum width) and reads the page document's widths once it has
 * settled.
 */
async function measure(chrome: string, dir: string, page: string, width: number, reachable: readonly string[]): Promise<Probe> {
  const wrapper = join(dir, 'probe.html')
  await writeFile(
    wrapper,
    `<!doctype html><html><body><iframe id="f" src="${page}" style="width:${String(width)}px;height:900px;border:0"></iframe>
<script>document.getElementById('f').addEventListener('load', function () { setTimeout(function () {
  var d = document.getElementById('f').contentDocument, r = d.documentElement;
  document.body.setAttribute('data-probe', JSON.stringify({ scrollWidth: r.scrollWidth, clientWidth: r.clientWidth,
    nodes: d.querySelectorAll('pre.mermaid svg g.node').length }));
}, 3000) })</script></body></html>`,
  )
  const args = [
    '--headless=new',
    '--disable-gpu',
    '--no-sandbox',
    '--no-first-run',
    '--allow-file-access-from-files',
    `--user-data-dir=${join(dir, 'profile')}`,
    '--window-size=800,1000',
    '--virtual-time-budget=15000',
    // Every host but the reachable ones fails to resolve.
    `--host-resolver-rules=${['MAP * ~NOTFOUND', ...reachable.map((host) => `EXCLUDE ${host}`)].join(', ')}`,
    '--dump-dom',
    `file://${wrapper}`,
  ]
  const attribute = /data-probe="([^"]*)"/.exec(await dumpDom(chrome, args))?.[1]
  if (attribute === undefined) throw new Error('the probe never reported')
  return JSON.parse(attribute.replaceAll('&quot;', '"')) as Probe
}

/**
 * Runs a browser until it has printed its DOM dump, then kills it and every
 * process it started, and resolves with the dump.
 *
 * The browser's own shutdown is no part of the measurement. Under load,
 * Chrome on macOS can hang in teardown after a complete dump until its
 * teardown watchdog ends it with exit code 2, about 15 s later. Waiting for
 * that exit made the width test fail on a dump it had already received.
 */
function dumpDom(browser: string, args: readonly string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    // Its own process group, so one signal reaches the browser's helpers too.
    const child = spawn(browser, args, { detached: true, stdio: ['ignore', 'pipe', 'pipe'] })
    let stdout = ''
    let stderr = ''
    let dumped = false
    const end = (): void => {
      try {
        if (child.pid !== undefined) process.kill(-child.pid, 'SIGKILL')
      } catch {
        // The group is gone already.
      }
    }
    const timer = setTimeout(end, 50_000)
    const finish = (): void => {
      clearTimeout(timer)
      // Again once the browser is gone: a helper it started while the first signal went out escaped that one.
      end()
      resolve(stdout)
    }
    const fail = (error: Error): void => {
      clearTimeout(timer)
      reject(error)
    }
    child.stdout.setEncoding('utf8').on('data', (chunk: string) => {
      stdout += chunk
      if (!dumped && stdout.includes('</html>')) {
        dumped = true
        end()
      }
    })
    child.stderr.setEncoding('utf8').on('data', (chunk: string) => {
      stderr += chunk
    })
    child.on('error', fail)
    // Not 'close': a helper that outlives the browser would hold its stdout open.
    child.on('exit', () => {
      if (dumped) finish()
    })
    // The browser may exit before its last output is read; this decides once all of it is.
    child.on('close', (code, signal) => {
      if (dumped) finish()
      else fail(new Error(`the browser stopped (${String(code ?? signal)}) before its DOM dump: ${stderr.slice(-2000)}`))
    })
  })
}

/** A Chrome or Chromium on this machine: CHROME_BIN, the macOS app, or one on PATH. */
function findChrome(): string | undefined {
  const candidates = [
    process.env.CHROME_BIN,
    '/Applications/Google Chrome.app/Contents/MacOS/Google Chrome',
    ...(process.env.PATH ?? '')
      .split(delimiter)
      .flatMap((dir) => ['google-chrome', 'google-chrome-stable', 'chromium', 'chromium-browser'].map((name) => join(dir, name))),
  ]
  return candidates.find((candidate) => candidate !== undefined && candidate !== '' && existsSync(candidate))
}

/**
 * Whether the browser tests skip, and the browser they run in. Without a
 * Chrome they skip, unless BUILD_TRACKER_REQUIRE_CHROME=1 (as CI sets it)
 * requires one: then they run and fail, so a runner that lost its Chrome
 * cannot pass them by skipping.
 */
function browserGate(found: string | undefined, required: boolean): { readonly skip: boolean; browser(): string } {
  return {
    browser: () => {
      if (found === undefined) {
        throw new Error('BUILD_TRACKER_REQUIRE_CHROME=1, but no Chrome was found: set CHROME_BIN or put google-chrome on PATH')
      }
      return found
    },
    skip: found === undefined && !required,
  }
}
