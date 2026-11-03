import { execFile } from 'node:child_process'
import { existsSync } from 'node:fs'
import { mkdtemp, readFile, rm, writeFile } from 'node:fs/promises'
import { tmpdir } from 'node:os'
import { delimiter, join } from 'node:path'
import { promisify } from 'node:util'

import { afterEach, describe, expect, it } from 'vitest'

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

async function renderedPage(state: Record<string, unknown>): Promise<{ dir: string; page: string; path: string }> {
  const dir = await mkdtemp(join(tmpdir(), 'build-tracker-page-'))
  cleanups.push(() => rm(dir, { force: true, recursive: true }))
  await writeFile(join(dir, 'state.json'), JSON.stringify(state))
  const path = join(dir, 'tracker.html')
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
    expect(scripts).toEqual(['https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.min.js'])
    // A classic script with its integrity hash: it runs from a local file too, where a module would need CORS.
    expect(page).toMatch(/<script src="https:\/\/cdn\.jsdelivr\.net\/npm\/mermaid@11\.4\.1\/dist\/mermaid\.min\.js" integrity="sha384-[A-Za-z0-9+/]{64}" crossorigin="anonymous"><\/script>/)
    const stylesheets = [...page.matchAll(/<link\b[^>]*\bhref="([^"]+)"/g)].map((m) => String(m[1]))
    expect(stylesheets).toHaveLength(1)
    expect(stylesheets[0]).toMatch(/^https:\/\/fonts\.googleapis\.com\/css2\?/)
    const external = [...page.matchAll(/(?:src|href)="(https?:[^"]+)"/g)].map((m) => String(m[1]))
    expect(external.filter((url) => !url.startsWith('https://github.com/acme/widgets/issues/'))).toEqual([...stylesheets, ...scripts])
  })

  it('starts Mermaid on the graph without waiting for the page to be published', async () => {
    const { page } = await renderedPage(wideState())

    expect(page).toContain("window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict'")
    expect(page).toContain("window.mermaid.run({ querySelector: 'pre.mermaid' });")
    expect(page.indexOf('mermaid.min.js')).toBeLessThan(page.indexOf('window.mermaid.run('))
  })

  const chrome = findChrome()

  it.skipIf(chrome === undefined)('does not scroll sideways at 375 CSS px wide in a headless browser', { timeout: 60_000 }, async () => {
    const { dir, path } = await renderedPage(wideState())

    // Offline, so the test never reaches the network: the graph stays as its source text.
    const probe = await measure(String(chrome), dir, path, 375, true)

    expect(probe.clientWidth).toBeGreaterThan(300)
    expect(probe.clientWidth).toBeLessThanOrEqual(375)
    expect(probe.scrollWidth).toBeLessThanOrEqual(probe.clientWidth)
  })

  // Opt-in, as it fetches Mermaid from the CDN: BUILD_TRACKER_ONLINE=1 npx vitest run src/build-tracker-cli/page.test.ts
  it.skipIf(chrome === undefined || process.env.BUILD_TRACKER_ONLINE !== '1')(
    'renders its graph from a local file, one node per ticket, still without sideways scroll',
    { timeout: 60_000 },
    async () => {
      const { dir, path } = await renderedPage(wideState())

      const probe = await measure(String(chrome), dir, path, 375, false)

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
async function measure(chrome: string, dir: string, page: string, width: number, offline: boolean): Promise<Probe> {
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
    ...(offline ? ['--host-resolver-rules=MAP * ~NOTFOUND'] : []),
    '--dump-dom',
    `file://${wrapper}`,
  ]
  const { stdout } = await promisify(execFile)(chrome, args, { maxBuffer: 64 * 1024 * 1024, timeout: 50_000 })
  const attribute = /data-probe="([^"]*)"/.exec(stdout)?.[1]
  if (attribute === undefined) throw new Error('the probe never reported')
  return JSON.parse(attribute.replaceAll('&quot;', '"')) as Probe
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
