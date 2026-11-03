import { buildBoard, SHOWN_LABELS, SHOWN_ORDER, type BoardSection, type BoardTicket, type Shown } from './board.js'
import type { TrackerState } from './state.js'

/**
 * The pinned Mermaid build the page loads. A classic script with its
 * integrity hash, so the graph renders from a local file as well as when the
 * page is published.
 */
export const MERMAID_URL = 'https://cdn.jsdelivr.net/npm/mermaid@11.4.1/dist/mermaid.min.js'
export const MERMAID_INTEGRITY = 'sha384-rbtjAdnIQE/aQJGEgXrVUlMibdfTSa4PQju4HDhN3sR2PmaKFzhEafuePsl9H/9I'

const FONTS_URL =
  'https://fonts.googleapis.com/css2?family=IBM+Plex+Mono&family=IBM+Plex+Sans+Condensed:wght@600&family=IBM+Plex+Sans:wght@400;600&display=swap'

/** One fill per shown status in the graph, the same colours the page's pills use. */
const GRAPH_CLASSES: Readonly<Record<Shown, string>> = {
  merged: 'fill:#2f7d55,stroke:#2f7d55,color:#fff',
  review: 'fill:#7a5cc4,stroke:#7a5cc4,color:#fff',
  running: 'fill:#c8871a,stroke:#c8871a,color:#fff',
  ready: 'fill:#fff,stroke:#2b6cb0,stroke-width:2px,color:#1d2b3a',
  owner: 'fill:#c0392b,stroke:#c0392b,color:#fff',
  blocked: 'fill:#e6e9ee,stroke:#b8c0cc,color:#5b6676',
}

const GRAPH_TITLE_LIMIT = 35

/**
 * The tracker page for one state. Everything on it comes from the state but
 * `updated`, so the same state renders the same page apart from that line.
 */
export function renderPage(state: TrackerState, updated: string): string {
  const board = buildBoard(state)
  const issueBase = `https://github.com/${state.spec.owner}/${state.spec.repo}/issues/`
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
<title>Spec #${String(state.spec.number)} build tracker</title>
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
${board.sections.map((part) => sectionHtml(part, issueBase)).join('\n')}
</main>
<script src="${MERMAID_URL}" integrity="${MERMAID_INTEGRITY}" crossorigin="anonymous"></script>
<script>
${MERMAID_START}
</script>
</body>
</html>
`
}

function sectionHtml(part: BoardSection, issueBase: string): string {
  const items = part.tickets.map((ticket) => ticketHtml(ticket, issueBase)).join('\n')
  return `<section><h2>${escapeHtml(part.name)} <small>${String(part.merged)}/${String(part.tickets.length)} merged</small></h2>
<ul>
${items}
</ul></section>`
}

function ticketHtml(ticket: BoardTicket, issueBase: string): string {
  const number = String(ticket.number)
  const blockers = ticket.blockedBy.map((blocker) => `#${String(blocker)}`).join(', ') || 'none'
  const live = ticket.live ? ' <span class="live">live check</span>' : ''
  const note = ticket.note === '' ? '' : ` · ${escapeHtml(ticket.note)}`
  return (
    `<li><span class="pill ${ticket.shown}">${SHOWN_LABELS[ticket.shown]}</span>` +
    `<a href="${issueBase}${number}">#${number}</a> <span class="t">${escapeHtml(ticket.title)}</span>${live}` +
    `<span class="meta">blocked by ${blockers}${note}</span></li>`
  )
}

/** The Mermaid source of the dependency graph: one node per ticket, one edge per blocker. */
function graph(tickets: readonly BoardTicket[]): string {
  const lines = ['flowchart LR']
  for (const ticket of tickets) {
    lines.push(`  t${String(ticket.number)}["#${String(ticket.number)} ${graphLabel(ticket.title)}"]:::${ticket.shown}`)
  }
  for (const ticket of tickets) {
    for (const blocker of ticket.blockedBy) lines.push(`  t${String(blocker)} --> t${String(ticket.number)}`)
  }
  for (const shown of SHOWN_ORDER) lines.push(`  classDef ${shown} ${GRAPH_CLASSES[shown]}`)
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
const STYLE = `:root { --bg:#f6f7f9; --panel:#ffffff; --fg:#1d2b3a; --muted:#5b6676; --line:#d9dee5; --accent:#2b6cb0;
  --ok:#2f7d55; --run:#c8871a; --rev:#7a5cc4; --bad:#c0392b; --off:#e6e9ee; --on-status:#ffffff;
  --display:"IBM Plex Sans Condensed", "Arial Narrow", sans-serif; --body:"IBM Plex Sans", system-ui, sans-serif; --mono:"IBM Plex Mono", ui-monospace, monospace;
  color-scheme:light; }
@media (prefers-color-scheme: dark) { :root:not([data-theme="light"]) { --bg:#11161d; --panel:#18202a; --fg:#e3e8ef; --muted:#97a3b3; --line:#2a3542; --accent:#6aa5e8; --off:#25303c; color-scheme:dark; } }
:root[data-theme="dark"] { --bg:#11161d; --panel:#18202a; --fg:#e3e8ef; --muted:#97a3b3; --line:#2a3542; --accent:#6aa5e8; --off:#25303c; color-scheme:dark; }
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
li a { font-family:var(--mono); color:var(--accent); text-decoration:none; font-variant-numeric:tabular-nums; }
li a:focus-visible { outline:2px solid var(--accent); outline-offset:2px; }
.t { min-width:0; }
.meta { flex-basis:100%; color:var(--muted); font-size:.8rem; padding-left:92px; }
.pill { display:inline-block; min-width:78px; text-align:center; font-size:.72rem; letter-spacing:.04em; text-transform:uppercase; border-radius:999px; padding:2px 8px; font-family:var(--mono); }
.merged { background:var(--ok); color:var(--on-status); } .running { background:var(--run); color:var(--on-status); } .review { background:var(--rev); color:var(--on-status); }
.ready { border:1.5px solid var(--accent); color:var(--accent); } .owner { background:var(--bad); color:var(--on-status); } .blocked { background:var(--off); color:var(--muted); }
.live { font-size:.7rem; font-family:var(--mono); color:var(--bad); border:1px solid var(--bad); border-radius:4px; padding:0 5px; }
@media (max-width:520px) { .meta { padding-left:0; } }`
