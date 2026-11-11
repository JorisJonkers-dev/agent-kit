//! The build tracker page for one state: one column, a summary strip, the
//! dependency graph as Mermaid source, then one section per milestone.

use crate::board::{build_board, sections, BoardSection, BoardTicket, Shown, SHOWN_ORDER};
use crate::spec::{file_link, TicketFiles};
use crate::state::{BuildTrackerState, SpecReference};
use crate::text::{is_space, without_md};

/// The pinned Mermaid build the page loads: a version and the sha384 of that version's `mermaid.min.js`.
const MERMAID_VERSION: &str = "11.4.1";
const MERMAID_INTEGRITY: &str = "sha384-rbtjAdnIQE/aQJGEgXrVUlMibdfTSa4PQju4HDhN3sR2PmaKFzhEafuePsl9H/9I";

const FONTS_URL: &str = "https://fonts.googleapis.com/css2?family=IBM+Plex+Mono&family=IBM+Plex+Sans+Condensed:wght@600&family=IBM+Plex+Sans:wght@400;600&display=swap";

/// The page's light-mode colours: both the CSS tokens on `:root` and the graph's fills.
const LIGHT: [(&str, &str); 12] = [
    ("bg", "#f6f7f9"),
    ("panel", "#ffffff"),
    ("fg", "#1d2b3a"),
    ("muted", "#5b6676"),
    ("line", "#d9dee5"),
    ("accent", "#2b6cb0"),
    ("ok", "#2f7d55"),
    ("run", "#c8871a"),
    ("rev", "#7a5cc4"),
    ("bad", "#c0392b"),
    ("off", "#e6e9ee"),
    ("on-status", "#ffffff"),
];

/// The colours dark mode redefines; the status colours stay as they are.
const DARK: [(&str, &str); 7] = [
    ("bg", "#11161d"),
    ("panel", "#18202a"),
    ("fg", "#e3e8ef"),
    ("muted", "#97a3b3"),
    ("line", "#2a3542"),
    ("accent", "#6aa5e8"),
    ("off", "#25303c"),
];

fn light(name: &str) -> &'static str {
    LIGHT.iter().find(|(key, _)| *key == name).map_or("", |(_, value)| value)
}

/// Each colour as a CSS custom property: `--bg:#f6f7f9; …`.
fn tokens(colours: &[(&str, &str)]) -> String {
    colours.iter().map(|(name, value)| format!("--{name}:{value};")).collect::<Vec<_>>().join(" ")
}

/// One fill per shown status in the graph, the same colours the page's pills use.
fn graph_class(shown: Shown) -> String {
    let on = light("on-status");
    match shown {
        Shown::Merged => format!("fill:{0},stroke:{0},color:{on}", light("ok")),
        Shown::Review => format!("fill:{0},stroke:{0},color:{on}", light("rev")),
        Shown::Running => format!("fill:{0},stroke:{0},color:{on}", light("run")),
        Shown::Ready => format!("fill:{},stroke:{},stroke-width:2px,color:{}", light("panel"), light("accent"), light("fg")),
        Shown::Owner => format!("fill:{0},stroke:{0},color:{on}", light("bad")),
        Shown::Blocked => format!("fill:{},stroke:#b8c0cc,color:{}", light("off"), light("muted")),
    }
}

const GRAPH_TITLE_LIMIT: usize = 35;

pub struct RenderedPage {
    pub html: String,
    pub counts: Vec<(Shown, usize)>,
}

/// The build tracker page for one state. Everything on it comes from the state but `updated`. A local spec's tickets
/// link to their files where `files` says they are shown, and have no link without it.
pub fn render_page(state: &BuildTrackerState, updated: &str, files: Option<&TicketFiles>) -> RenderedPage {
    let board = build_board(state);
    let parts = sections(state, &board.tickets);
    let link = |ticket: &BoardTicket| -> Option<String> {
        match &state.spec {
            SpecReference::Local { .. } => ticket.ticket.file.as_deref().zip(files).and_then(|(file, files)| file_link(files, file)),
            SpecReference::Github { repository, .. } => Some(format!("https://{}/{}/{}/issues/{}", repository.host, repository.owner, repository.repo, ticket.ticket.number)),
        }
    };
    let strip: Vec<String> = board.counts.iter().map(|(shown, count)| format!("<span class=\"pill {}\">{} {count}</span>", shown.name(), shown.label())).collect();
    let stamp = format!("updated {updated}");
    let subtitle: Vec<String> = [state.subtitle.as_str(), stamp.as_str()].iter().filter(|part| !part.is_empty()).map(|part| escape_html(part)).collect();
    let body: Vec<String> = parts.iter().map(|part| section_html(part, &link)).collect();
    let html = format!(
        "<!doctype html>
<html lang=\"en\">
<head>
<meta charset=\"utf-8\">
<meta name=\"viewport\" content=\"width=device-width, initial-scale=1\">
<meta name=\"claude:viewport-fit-sides\" content=\"cover\">
<title>{title}</title>
<link rel=\"stylesheet\" href=\"{FONTS_URL}\">
<style>
{style}
</style>
</head>
<body>
<main>
<header><h1>{heading}</h1>
<p class=\"sub\">{subtitle}</p></header>
<div class=\"strip\">
{strip}
</div>
<div class=\"graph\"><pre class=\"mermaid\">
{graph}
</pre></div>
{body}
</main>
<script src=\"https://cdn.jsdelivr.net/npm/mermaid@{MERMAID_VERSION}/dist/mermaid.min.js\" integrity=\"{MERMAID_INTEGRITY}\" crossorigin=\"anonymous\"></script>
<script>
{MERMAID_START}
</script>
</body>
</html>
",
        title = escape_html(&page_title(state)),
        style = style(),
        heading = escape_html(&state.title),
        subtitle = subtitle.join(" · "),
        strip = strip.join("\n"),
        graph = escape_html(&graph(&board.tickets)),
        body = body.join("\n"),
    );
    RenderedPage { counts: board.counts, html }
}

/// Two to four words: `Spec #12 build tracker`, or the local spec's file name in front.
fn page_title(state: &BuildTrackerState) -> String {
    match &state.spec {
        SpecReference::Github { number, .. } => format!("Spec #{number} build tracker"),
        SpecReference::Local { path } => {
            let name = path.rsplit('/').next().unwrap_or("");
            format!("{} build tracker", collapse(without_md(name), "-"))
        }
    }
}

/// Every run of whitespace replaced by `with`, as `replace(/\s+/g, with)`.
fn collapse(text: &str, with: &str) -> String {
    let mut out = String::new();
    let mut in_space = false;
    for c in text.chars() {
        if is_space(c) {
            if !in_space {
                out.push_str(with);
            }
            in_space = true;
        } else {
            out.push(c);
            in_space = false;
        }
    }
    out
}

fn section_html(part: &BoardSection, link: &dyn Fn(&BoardTicket) -> Option<String>) -> String {
    let items: Vec<String> = part.tickets.iter().map(|ticket| ticket_html(ticket, link)).collect();
    format!(
        "<section><h2>{} <small>{}/{} merged</small></h2>\n<ul>\n{}\n</ul></section>",
        escape_html(&part.name),
        part.merged,
        part.tickets.len(),
        items.join("\n")
    )
}

fn ticket_html(ticket: &BoardTicket, link: &dyn Fn(&BoardTicket) -> Option<String>) -> String {
    let item = ticket.ticket;
    let number = item.number;
    let anchor = match link(ticket) {
        Some(href) => format!("<a href=\"{}\">#{number}</a>", escape_html(&href)),
        None => format!("<span class=\"n\">#{number}</span>"),
    };
    let mut blockers: Vec<String> = item.blocked_by.iter().map(|blocker| format!("#{blocker}")).collect();
    for blocker in &item.external {
        let state = blocker.state.as_ref().map(|state| format!(", {state}")).unwrap_or_default();
        blockers.push(escape_html(&format!("{} (other spec{state})", blocker.reference)));
    }
    let blockers = if blockers.is_empty() { "none".to_string() } else { blockers.join(", ") };
    let live = if item.live { " <span class=\"live\">live check</span>" } else { "" };
    let note = if item.note.is_empty() { String::new() } else { format!(" · {}", escape_html(&item.note)) };
    format!(
        "<li><span class=\"pill {}\">{}</span>{anchor} <span class=\"t\">{}</span>{live}<span class=\"meta\">blocked by {blockers}{note}</span></li>",
        ticket.shown.name(),
        ticket.shown.label(),
        escape_html(&item.title)
    )
}

/// The Mermaid source of the dependency graph: one node per ticket, one edge per blocker. A blocker outside the spec is
/// one dashed node, however many tickets wait on it, with a dotted edge to each.
fn graph(tickets: &[BoardTicket]) -> String {
    let mut lines = vec!["flowchart LR".to_string()];
    for ticket in tickets {
        let number = ticket.ticket.number;
        lines.push(format!("  t{number}[\"#{number} {}\"]:::{}", graph_label(&ticket.ticket.title), ticket.shown.name()));
    }
    let mut outside: Vec<(&str, String)> = Vec::new();
    for ticket in tickets {
        for blocker in &ticket.ticket.external {
            if outside.iter().any(|(reference, _)| *reference == blocker.reference) {
                continue;
            }
            let id = format!("x{}", outside.len() + 1);
            lines.push(format!("  {id}[\"{} · other spec\"]:::external", graph_label(&blocker.reference)));
            outside.push((&blocker.reference, id));
        }
    }
    for ticket in tickets {
        let number = ticket.ticket.number;
        lines.extend(ticket.ticket.blocked_by.iter().map(|blocker| format!("  t{blocker} --> t{number}")));
        for blocker in &ticket.ticket.external {
            let id = outside.iter().find(|(reference, _)| *reference == blocker.reference).map_or("", |(_, id)| id.as_str());
            lines.push(format!("  {id} -.-> t{number}"));
        }
    }
    lines.extend(SHOWN_ORDER.iter().map(|shown| format!("  classDef {} {}", shown.name(), graph_class(*shown))));
    lines.push(format!(
        "  classDef external fill:none,stroke:#8a94a3,stroke-width:1.5px,stroke-dasharray:5 4,color:{}",
        light("muted")
    ));
    lines.join("\n")
}

/// A title shortened for a graph node, with Mermaid's own entity codes for the characters its labels reserve.
fn graph_label(raw: &str) -> String {
    // A line break would end the node's line in the Mermaid source.
    let title = collapse(raw, " ");
    let short = if title.chars().count() > GRAPH_TITLE_LIMIT {
        format!("{}…", title.chars().take(GRAPH_TITLE_LIMIT - 1).collect::<String>())
    } else {
        title
    };
    short.replace('#', "#35;").replace('"', "#quot;")
}

fn escape_html(text: &str) -> String {
    text.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;").replace('"', "&quot;")
}

/// Starts Mermaid in the page's theme. Without the script (offline, or blocked) the graph stays as its source text and
/// the rest of the page is whole.
const MERMAID_START: &str = "(function () {
  if (!window.mermaid) return;
  var root = document.documentElement;
  var dark = root.dataset.theme === 'dark' ||
    (root.dataset.theme !== 'light' && window.matchMedia('(prefers-color-scheme: dark)').matches);
  window.mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: dark ? 'dark' : 'default' });
  window.mermaid.run({ querySelector: 'pre.mermaid' });
})();";

fn style() -> String {
    format!(
        ":root {{ {light}
  --display:\"IBM Plex Sans Condensed\", \"Arial Narrow\", sans-serif; --body:\"IBM Plex Sans\", system-ui, sans-serif; --mono:\"IBM Plex Mono\", ui-monospace, monospace;
  color-scheme:light; }}
@media (prefers-color-scheme: dark) {{ :root:not([data-theme=\"light\"]) {{ {dark} color-scheme:dark; }} }}
:root[data-theme=\"dark\"] {{ {dark} color-scheme:dark; }}
{STYLE_BODY}",
        light = tokens(&LIGHT),
        dark = tokens(&DARK)
    )
}

/// The page's layout rules, after the colour tokens.
const STYLE_BODY: &str = include_str!("style.css");
