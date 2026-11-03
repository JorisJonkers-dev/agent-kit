import { buildBoard, SHOWN_LABELS, SHOWN_ORDER, type Board, type BoardSection, type BoardTicket, type Counts, type Shown } from './board.js'
import { fileLink, type TicketFiles } from './spec.js'
import type { BuildTrackerState } from './state.js'

/**
 * The pinned Mermaid build the page loads: a version and the sha384 of that
 * version's `mermaid.min.js`, changed together. A classic script with its
 * integrity hash, so the graph renders from a local file as well as when the
 * page is published. A browser refuses the script when the hash is wrong, so
 * a new pin needs the online check in `page.test.ts`.
 */
const MERMAID_PIN = {
  version: '11.4.1',
  integrity: 'sha384-rbtjAdnIQE/aQJGEgXrVUlMibdfTSa4PQju4HDhN3sR2PmaKFzhEafuePsl9H/9I',
} as const

const MERMAID_URL = `https://cdn.jsdelivr.net/npm/mermaid@${MERMAID_PIN.version}/dist/mermaid.min.js`

const FONTS_URL =
  'https://fonts.googleapis.com/css2?family=IBM+Plex+Mono&family=IBM+Plex+Sans+Condensed:wght@600&family=IBM+Plex+Sans:wght@400;600&display=swap'

/**
 * The page's light-mode colours, the one source of both the CSS tokens on
 * `:root` and the graph's fills, which Mermaid takes as literal colours.
 */
const LIGHT = {
  bg: '#f6f7f9',
  panel: '#ffffff',
  fg: '#1d2b3a',
  muted: '#5b6676',
  line: '#d9dee5',
  accent: '#2b6cb0',
  ok: '#2f7d55',
  run: '#c8871a',
  rev: '#7a5cc4',
  bad: '#c0392b',
  off: '#e6e9ee',
  'on-status': '#ffffff',
} as const

/** The colours dark mode redefines; the status colours stay as they are. */
const DARK = {
  bg: '#11161d',
  panel: '#18202a',
  fg: '#e3e8ef',
  muted: '#97a3b3',
  line: '#2a3542',
  accent: '#6aa5e8',
  off: '#25303c',
} as const

/** Each colour as a CSS custom property: `--bg:#f6f7f9; …`. */
function tokens(colours: Readonly<Record<string, string>>): string {
  return Object.entries(colours)
    .map(([name, value]) => `--${name}:${value};`)
    .join(' ')
}

/** One fill per shown status in the graph, the same colours the page's pills use. */
const GRAPH_CLASSES: Readonly<Record<Shown, string>> = {
  merged: `fill:${LIGHT.ok},stroke:${LIGHT.ok},color:${LIGHT['on-status']}`,
  review: `fill:${LIGHT.rev},stroke:${LIGHT.rev},color:${LIGHT['on-status']}`,
  running: `fill:${LIGHT.run},stroke:${LIGHT.run},color:${LIGHT['on-status']}`,
  ready: `fill:${LIGHT.panel},stroke:${LIGHT.accent},stroke-width:2px,color:${LIGHT.fg}`,
  owner: `fill:${LIGHT.bad},stroke:${LIGHT.bad},color:${LIGHT['on-status']}`,
  blocked: `fill:${LIGHT.off},stroke:#b8c0cc,color:${LIGHT.muted}`,
}

/** A blocker outside the spec: a dashed outline, no fill. */
const EXTERNAL_CLASS = `fill:none,stroke:#8a94a3,stroke-width:1.5px,stroke-dasharray:5 4,color:${LIGHT.muted}`

const GRAPH_TITLE_LIMIT = 35

/** A rendered page and the count per shown status it shows. */
export interface RenderedPage {
  readonly html: string
  readonly counts: Counts
}

/**
 * The build tracker page for one state. Everything on it comes from the
 * state but `updated`, so the same state renders the same page apart from
 * that line. A local spec's tickets link to their files where `files` says
 * they are shown, and have no link without it.
 */
export function renderPage(state: BuildTrackerState, updated: string, files: TicketFiles | undefined): RenderedPage {
  const board = buildBoard(state)
  return { counts: board.counts, html: pageHtml(state, board, updated, ticketLink(state, files)) }
}

function pageHtml(state: BuildTrackerState, board: Board, updated: string, link: TicketLink): string {
  const strip = SHOWN_ORDER.flatMap((shown) => {
    const count = board.counts[shown]
    return count === undefined ? [] : [`<span class="pill ${shown}">${SHOWN_LABELS[shown]} ${String(count)}</span>`]
  }).join('\n')
  const subtitle = [state.subtitle, `updated ${updated}`].filter((part) => part !== '').map(escapeHtml).join(' · ')
  return `<!doctype html>
<html lang="en">
<head>
<meta charset="utf-8">
<meta name="viewport" content="width=device-width, initial-scale=1">
<meta name="claude:viewport-fit-sides" content="cover">
<title>${escapeHtml(pageTitle(state))}</title>
<link rel="stylesheet" href="${FONTS_URL}">
<style>
${STYLE}
</style>
</head>
<body>
<main>
<header><h1>${escapeHtml(state.title)}</h1>
<p class="sub">${subtitle}</p></header>
<div class="strip">
${strip}
</div>
<div class="graph"><pre class="mermaid">
${escapeHtml(graph(board.tickets))}
</pre></div>
${board.sections.map((part) => sectionHtml(part, link)).join('\n')}
</main>
<script src="${MERMAID_URL}" integrity="${MERMAID_PIN.integrity}" crossorigin="anonymous"></script>
<script>
${MERMAID_START}
</script>
</body>
</html>
`
}

/** Where a ticket's link goes: its issue, or a local ticket's file; undefined for no link. */
type TicketLink = (ticket: BoardTicket) => string | undefined

function ticketLink(state: BuildTrackerState, files: TicketFiles | undefined): TicketLink {
  const spec = state.spec
  if (spec.kind === 'local') return (ticket) => (ticket.file === undefined || files === undefined ? undefined : fileLink(files, ticket.file))
  const { host, owner, repo } = spec.repository
  return (ticket) => `https://${host}/${owner}/${repo}/issues/${String(ticket.number)}`
}

/** Two to four words: `Spec #12 build tracker`, or the local spec's file name in front. */
function pageTitle(state: BuildTrackerState): string {
  if (state.spec.kind === 'github') return `Spec #${String(state.spec.number)} build tracker`
  const stem = (state.spec.path.split('/').at(-1) ?? '').replace(/\.md$/i, '')
  return `${stem.replace(/\s+/g, '-')} build tracker`
}

function sectionHtml(part: BoardSection, link: TicketLink): string {
  const items = part.tickets.map((ticket) => ticketHtml(ticket, link)).join('\n')
  return `<section><h2>${escapeHtml(part.name)} <small>${String(part.merged)}/${String(part.tickets.length)} merged</small></h2>
<ul>
${items}
</ul></section>`
}

function ticketHtml(ticket: BoardTicket, link: TicketLink): string {
  const number = String(ticket.number)
  const href = link(ticket)
  const anchor = href === undefined ? `<span class="n">#${number}</span>` : `<a href="${escapeHtml(href)}">#${number}</a>`
  const blockers =
    [
      ...ticket.blockedBy.map((blocker) => `#${String(blocker)}`),
      ...ticket.external.map((blocker) => escapeHtml(`${blocker.ref} (other spec${blocker.state === undefined ? '' : `, ${blocker.state}`})`)),
    ].join(', ') || 'none'
  const live = ticket.live ? ' <span class="live">live check</span>' : ''
  const note = ticket.note === '' ? '' : ` · ${escapeHtml(ticket.note)}`
  return (
    `<li><span class="pill ${ticket.shown}">${SHOWN_LABELS[ticket.shown]}</span>` +
    `${anchor} <span class="t">${escapeHtml(ticket.title)}</span>${live}` +
    `<span class="meta">blocked by ${blockers}${note}</span></li>`
  )
}

/**
 * The Mermaid source of the dependency graph: one node per ticket, one edge
 * per blocker. A blocker outside the spec is one dashed node, however many
 * tickets wait on it, with a dotted edge to each.
 */
function graph(tickets: readonly BoardTicket[]): string {
  const lines = ['flowchart LR']
  for (const ticket of tickets) {
    lines.push(`  t${String(ticket.number)}["#${String(ticket.number)} ${graphLabel(ticket.title)}"]:::${ticket.shown}`)
  }
  const outside = new Map<string, string>()
  for (const ticket of tickets) {
    for (const blocker of ticket.external) {
      if (outside.has(blocker.ref)) continue
      const id = `x${String(outside.size + 1)}`
      outside.set(blocker.ref, id)
      lines.push(`  ${id}["${graphLabel(blocker.ref)} · other spec"]:::external`)
    }
  }
  for (const ticket of tickets) {
    for (const blocker of ticket.blockedBy) lines.push(`  t${String(blocker)} --> t${String(ticket.number)}`)
    for (const blocker of ticket.external) lines.push(`  ${String(outside.get(blocker.ref))} -.-> t${String(ticket.number)}`)
  }
  for (const shown of SHOWN_ORDER) lines.push(`  classDef ${shown} ${GRAPH_CLASSES[shown]}`)
  lines.push(`  classDef external ${EXTERNAL_CLASS}`)
  return lines.join('\n')
}

/** A title shortened for a graph node, with Mermaid's own entity codes for the characters its labels reserve. */
function graphLabel(raw: string): string {
  // A line break would end the node's line in the Mermaid source.
  const title = raw.replace(/\s+/g, ' ')
  const characters = Array.from(title)
  const short = characters.length > GRAPH_TITLE_LIMIT ? `${characters.slice(0, GRAPH_TITLE_LIMIT - 1).join('')}…` : title
  return short.replaceAll('#', '#35;').replaceAll('"', '#quot;')
}

function escapeHtml(text: string): string {
  return text.replaceAll('&', '&amp;').replaceAll('<', '&lt;').replaceAll('>', '&gt;').replaceAll('"', '&quot;')
}

/**
 * Starts Mermaid in the page's theme. Without the script (offline, or
 * blocked) the graph stays as its source text and the rest of the page is
 * whole.
 */
const MERMAID_START = `(function () {
  if (!window.mermaid) return;
  var root = document.documentElement;
  var dark = root.dataset.theme === 'dark' ||
    (root.dataset.theme !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default' });
  window.mermaid.run({ querySelector: 'pre.mermaid' });
})();`

/* Layout: one column, summary strip, dependency graph, then one section per milestone. */
const STYLE = `:root { ${tokens(LIGHT)}
  --display:"IBM Plex Sans Condensed", "Arial Narrow", sans-serif; --body:"IBM Plex Sans", system-ui, sans-serif; --mono:"IBM Plex Mono", ui-monospace, monospace;
  color-scheme:light; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { ${tokens(DARK)} color-scheme:dark; } }
:root[data-theme="dark"] { ${tokens(DARK)} color-scheme:dark; }
* { box-sizing:border-box; }
body { margin:0; background:var(--bg); color:var(--fg); font-family:var(--body); padding-block:24px 48px; padding-left:max(16px, env(safe-area-inset-left, 0px)); padding-right:max(16px, env(safe-area-inset-right, 0px)); overflow-wrap:anywhere; }
main { max-width:1100px; margin:0 auto; display:grid; grid-template-columns:minmax(0, 1fr); gap:20px; }
h1 { font-family:var(--display); font-size:1.9rem; margin:0; text-wrap:balance; }
.sub { color:var(--muted); margin:4px 0 0; }
.strip { display:flex; flex-wrap:wrap; gap:8px; }
.graph { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:12px; overflow-x:auto; }
.graph pre { margin:0; }
section { background:var(--panel); border:1px solid var(--line); border-radius:8px; padding:14px 16px; }
h2 { font-family:var(--display); font-size:1.15rem; margin:0 0 8px; }
h2 small { font-family:var(--mono); font-size:.75rem; color:var(--muted); font-weight:400; margin-left:6px; }
ul { list-style:none; margin:0; padding:0; display:grid; gap:6px; }
li { display:flex; flex-wrap:wrap; align-items:baseline; gap:6px 8px; min-width:0; }
li a, li .n { font-family:var(--mono); color:var(--accent); text-decoration:none; font-variant-numeric:tabular-nums; }
li a:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.t { min-width:0; }
.meta { flex-basis:100%; color:var(--muted); font-size:.8rem; padding-left:92px; }
.pill { display:inline-block; min-width:78px; text-align:center; font-size:.72rem; letter-spacing:.04em; text-transform:uppercase; border-radius:999px; padding:2px 8px; font-family:var(--mono); }
.merged { background:var(--ok); color:var(--on-status); } .running { background:var(--run); color:var(--on-status); } .review { background:var(--rev); color:var(--on-status); }
.ready { border:1.5px solid var(--accent); color:var(--accent); } .owner { background:var(--bad); color:var(--on-status); } .blocked { background:var(--off); color:var(--muted); }
.live { font-size:.7rem; font-family:var(--mono); color:var(--bad); border:1px solid var(--bad); border-radius:4px; padding:0 5px; }
@media (max-width:520px) { .meta { padding-left:0; } }`
