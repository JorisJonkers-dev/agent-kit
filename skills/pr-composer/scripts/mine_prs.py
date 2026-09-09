#!/usr/bin/env python3
"""Turn a `gh pr list --state merged --json ...` dump into a house-style profile.

The point of doing this in a script rather than by reading 100 PR bodies: the
numbers are reproducible, cheap, and they distinguish "this repo always does X"
from "I saw X twice and assumed".

Usage:
    python3 mine_prs.py merged_prs.json [--out style.json] [--md]

Stdlib only.
"""
from __future__ import annotations

import argparse
import json
import os
import re
import statistics
import sys
from collections import Counter, defaultdict

CONVENTIONAL = re.compile(r"^(feat|fix|chore|docs|refactor|test|perf|build|ci|style|revert)(\([^)]*\))?!?:\s", re.I)
# Same shape but allowed to sit after a ticket-key prefix, e.g. "[MOVES-482] feat(api): …"
CONV_ANYWHERE = re.compile(
    r"(?:^|\]\s*|\s)(feat|fix|chore|docs|refactor|test|perf|build|ci|style|revert)(\([^)]*\))?!?:\s", re.I)
TICKET_KEY = re.compile(r"^\[?([A-Z][A-Z0-9]{1,9})[- ](\d+)\]?[:\s]")
HEADING = re.compile(r"^\s{0,3}(#{1,4})\s+(.+?)\s*#*\s*$", re.M)
BOLD_HEADING = re.compile(r"^\s{0,3}\*\*([^*\n]{2,60}?):?\*\*\s*$", re.M)
CHECKBOX = re.compile(r"^\s*[-*]\s*\[( |x|X)\]\s+", re.M)
HTML_COMMENT = re.compile(r"<!--.*?-->", re.S)
LINK_KEYWORD = re.compile(
    r"\b(close[sd]?|fix(?:e[sd])?|resolve[sd]?|ref(?:s|erences)?|relate[sd]?\s+to|part\s+of)\b[:\s]*#\d+", re.I)
BARE_ISSUE = re.compile(r"(?<![\w/])#\d+")
IMAGE = re.compile(r"(!\[[^\]]*\]\(|<img\s|user-images\.githubusercontent|githubusercontent\.com/assets)", re.I)
SLACK_LINK = re.compile(r"https?://[a-z0-9-]+\.slack\.com/\S+", re.I)


def pct(n: int, total: int) -> float:
    return round(100.0 * n / total, 1) if total else 0.0


def top(counter: Counter, k: int = 8) -> list[dict]:
    return [{"value": v, "count": c} for v, c in counter.most_common(k)]


def norm_heading(text: str) -> str:
    text = re.sub(r"[*_`:]", "", text).strip()
    text = re.sub(r"^[\W\s]+|[\W\s]+$", "", text)  # strip emoji/punct padding
    return re.sub(r"\s+", " ", text).lower()


def login(obj) -> str | None:
    if isinstance(obj, dict):
        return obj.get("login") or obj.get("name") or obj.get("slug")
    return None


def analyse(prs: list[dict]) -> dict:
    n = len(prs)
    prof: dict = {"sample_size": n}
    if n == 0:
        prof["warning"] = "no merged PRs available; no style profile can be derived"
        return prof

    titles = [p.get("title") or "" for p in prs]
    bodies = [p.get("body") or "" for p in prs]

    # ---- titles ----
    conv = sum(1 for t in titles if CONV_ANYWHERE.search(t))
    keys = Counter()
    for t in titles:
        m = TICKET_KEY.match(t)
        if m:
            keys[m.group(1)] += 1
    tickets = sum(keys.values())
    both = sum(1 for t in titles if TICKET_KEY.match(t) and CONV_ANYWHERE.search(t))
    lengths = [len(t) for t in titles]
    ends_period = sum(1 for t in titles if t.rstrip().endswith("."))
    lower_start = sum(1 for t in titles
                      if (re.sub(r"^(\[[^\]]+\]\s*|\w+(\([^)]*\))?!?:\s*)", "", t)[:1].islower()))

    scopes = Counter()
    for t in titles:
        m = CONV_ANYWHERE.search(t)
        if m and m.group(2):
            scopes[m.group(2).strip("()")] += 1

    prof["title"] = {
        "conventional_commit_rate": pct(conv, n),
        "ticket_key_rate": pct(tickets, n),
        "ticket_keys_seen": top(keys, 5),
        "ticket_plus_conventional_rate": pct(both, n),
        "median_length": int(statistics.median(lengths)),
        "p90_length": sorted(lengths)[int(0.9 * (n - 1))],
        "trailing_period_rate": pct(ends_period, n),
        "lowercase_after_prefix_rate": pct(lower_start, n),
        "common_scopes": top(scopes, 8),
        "examples": titles[:6],
    }

    # ---- bodies ----
    empty = sum(1 for b in bodies if not b.strip())
    stripped = [HTML_COMMENT.sub("", b) for b in bodies]
    word_counts = [len(s.split()) for s in stripped]
    nonempty_wc = [w for w in word_counts if w > 0] or [0]

    headings = Counter()
    heading_orders: list[list[str]] = []
    for s in stripped:
        found = [norm_heading(m.group(2)) for m in HEADING.finditer(s)]
        found += [norm_heading(m.group(1)) for m in BOLD_HEADING.finditer(s)]
        seen, ordered = set(), []
        for h in found:
            if h and h not in seen:
                seen.add(h)
                ordered.append(h)
        for h in seen:
            headings[h] += 1
        if ordered:
            heading_orders.append(ordered)

    # Sections that exist but are routinely left hollow (<8 words of content).
    hollow = Counter()
    present = Counter()
    for s in stripped:
        parts = re.split(r"^\s{0,3}#{1,4}\s+(.+)$", s, flags=re.M)
        for i in range(1, len(parts) - 1, 2):
            h = norm_heading(parts[i])
            if not h:
                continue
            present[h] += 1
            if len(parts[i + 1].split()) < 8:
                hollow[h] += 1
    hollow_rates = sorted(
        ({"section": h, "hollow_rate": pct(hollow[h], present[h]), "seen": present[h]}
         for h in present if present[h] >= max(3, n // 20)),
        key=lambda d: -d["hollow_rate"])[:10]

    link_kw = Counter()
    for s in stripped:
        for m in LINK_KEYWORD.finditer(s):
            link_kw[m.group(1).lower().replace("  ", " ")] += 1

    prof["body"] = {
        "empty_body_rate": pct(empty, n),
        "median_words": int(statistics.median(nonempty_wc)),
        "p10_words": sorted(nonempty_wc)[int(0.1 * (len(nonempty_wc) - 1))],
        "p90_words": sorted(nonempty_wc)[int(0.9 * (len(nonempty_wc) - 1))],
        "comments_retained_rate": pct(sum(1 for b in bodies if "<!--" in b), n),
        "checklist_rate": pct(sum(1 for s in stripped if CHECKBOX.search(s)), n),
        "checked_box_rate": pct(sum(1 for s in stripped if re.search(r"\[[xX]\]", s)), n),
        "image_rate": pct(sum(1 for s in stripped if IMAGE.search(s)), n),
        "slack_link_rate": pct(sum(1 for s in stripped if SLACK_LINK.search(s)), n),
        "issue_reference_rate": pct(sum(1 for s in stripped if BARE_ISSUE.search(s)), n),
        "link_keywords": top(link_kw, 6),
        "preferred_link_keyword": link_kw.most_common(1)[0][0] if link_kw else None,
        "section_frequency": [{"section": h, "rate": pct(c, n)} for h, c in headings.most_common(14)],
        "modal_section_order": max(heading_orders, key=lambda o: sum(headings[h] for h in o)) if heading_orders else [],
        "routinely_hollow_sections": hollow_rates,
    }

    # ---- people ----
    reviewers = Counter()
    approvers = Counter()
    requested = Counter()
    assignees = Counter()
    per_pr_reviewers = []
    by_path = defaultdict(Counter)

    for p in prs:
        rs = set()
        for r in p.get("reviews") or []:
            u = login(r.get("author"))
            if u:
                rs.add(u)
                if (r.get("state") or "").upper() == "APPROVED":
                    approvers[u] += 1
        for rr in p.get("reviewRequests") or []:
            u = login(rr) or login(rr.get("requestedReviewer") if isinstance(rr, dict) else None)
            if u:
                requested[u] += 1
                rs.add(u)
        for u in rs:
            reviewers[u] += 1
        per_pr_reviewers.append(len(rs))
        for a in p.get("assignees") or []:
            u = login(a)
            if u:
                assignees[u] += 1
        for f in (p.get("files") or [])[:200]:
            path = f.get("path") if isinstance(f, dict) else None
            if not path:
                continue
            bucket = "/".join(path.split("/")[:2]) or path
            for u in rs:
                by_path[bucket][u] += 1

    prof["people"] = {
        "top_reviewers": top(reviewers, 10),
        "top_approvers": top(approvers, 10),
        "top_requested": top(requested, 10),
        "top_assignees": top(assignees, 8),
        "median_reviewers_per_pr": int(statistics.median(per_pr_reviewers)) if per_pr_reviewers else 0,
        "mean_reviewers_per_pr": round(statistics.fmean(per_pr_reviewers), 2) if per_pr_reviewers else 0,
        "reviewer_by_path": {k: top(v, 4) for k, v in
                             sorted(by_path.items(), key=lambda kv: -sum(kv[1].values()))[:20]},
    }

    # ---- labels & size ----
    labels = Counter()
    for p in prs:
        for l in p.get("labels") or []:
            nm = l.get("name") if isinstance(l, dict) else None
            if nm:
                labels[nm] += 1
    sizes = [(p.get("additions") or 0) + (p.get("deletions") or 0) for p in prs]
    changed = [p.get("changedFiles") or 0 for p in prs]

    # A label family carried by nearly every merged PR is effectively mandatory
    # (version/*, release:*, type/*). Missing one is a finding, not an omission.
    families: dict[str, Counter] = defaultdict(Counter)
    family_prs: dict[str, set] = defaultdict(set)
    for p in prs:
        for l in p.get("labels") or []:
            nm = l.get("name") if isinstance(l, dict) else None
            if not nm:
                continue
            m = re.match(r"^([\w .-]+?)\s*[:/]\s*(.+)$", nm)
            fam = m.group(1).strip().lower() if m else None
            if fam:
                families[fam][nm] += 1
                family_prs[fam].add(p.get("number"))
    required = sorted(
        ({"family": f, "coverage": pct(len(family_prs[f]), n),
          "values": top(families[f], 6)}
         for f in families if pct(len(family_prs[f]), n) >= 80),
        key=lambda d: -d["coverage"])

    prof["meta"] = {
        "label_frequency": top(labels, 12),
        "labelled_pr_rate": pct(sum(1 for p in prs if p.get("labels")), n),
        "required_label_families": required,
        "label_families": [{"family": f, "coverage": pct(len(family_prs[f]), n)}
                           for f in sorted(families, key=lambda x: -len(family_prs[x]))[:8]],
        "median_lines_changed": int(statistics.median(sizes)) if sizes else 0,
        "median_files_changed": int(statistics.median(changed)) if changed else 0,
        "base_branches": top(Counter(p.get("baseRefName") or "?" for p in prs), 4),
    }

    if n < 10:
        prof["warning"] = (f"only {n} merged PRs — treat every rate here as anecdote, "
                           "lean on the template and conventional defaults instead")
    return prof


def render_md(p: dict) -> str:
    n = p.get("sample_size", 0)
    L: list[str] = [f"# House style profile ({n} merged PRs)"]
    if p.get("warning"):
        L += ["", f"> **{p['warning']}**"]
    if n == 0:
        return "\n".join(L) + "\n"

    t, b, ppl, m = p["title"], p["body"], p["people"], p["meta"]

    L += ["", "## Title", ""]
    L.append(f"- conventional-commit prefix: **{t['conventional_commit_rate']}%**")
    L.append(f"- ticket key prefix: **{t['ticket_key_rate']}%**"
             + (f" (keys: {', '.join(k['value'] for k in t['ticket_keys_seen'])})" if t["ticket_keys_seen"] else ""))
    L.append(f"- both together: {t['ticket_plus_conventional_rate']}%")
    L.append(f"- length: median {t['median_length']}, p90 {t['p90_length']} chars")
    L.append(f"- lowercase after prefix: {t['lowercase_after_prefix_rate']}% · trailing period: {t['trailing_period_rate']}%")
    if t["common_scopes"]:
        L.append("- scopes: " + ", ".join(f"{s['value']} ({s['count']})" for s in t["common_scopes"]))
    L += ["", "Recent titles:"] + [f"  - {x}" for x in t["examples"]]

    L += ["", "## Body", ""]
    L.append(f"- empty bodies: **{b['empty_body_rate']}%**")
    L.append(f"- length: p10 {b['p10_words']} / median **{b['median_words']}** / p90 {b['p90_words']} words")
    L.append(f"- template HTML comments left in: {b['comments_retained_rate']}%")
    L.append(f"- checklists: {b['checklist_rate']}% (boxes actually ticked: {b['checked_box_rate']}%)")
    L.append(f"- images/screenshots: {b['image_rate']}% · Slack links: {b['slack_link_rate']}%")
    L.append(f"- issue references: {b['issue_reference_rate']}% · preferred keyword: **{b['preferred_link_keyword'] or 'none'}**")
    if b["section_frequency"]:
        L += ["", "Sections by frequency:"]
        L += [f"  - {s['section']} — {s['rate']}%" for s in b["section_frequency"]]
    if b["modal_section_order"]:
        L += ["", "Typical order: " + " → ".join(b["modal_section_order"])]
    if b["routinely_hollow_sections"]:
        L += ["", "Sections usually left hollow (fill these only if you have real content):"]
        L += [f"  - {s['section']} — {s['hollow_rate']}% hollow of {s['seen']} seen"
              for s in b["routinely_hollow_sections"] if s["hollow_rate"] >= 40]

    L += ["", "## People", ""]
    L.append(f"- reviewers per PR: median **{ppl['median_reviewers_per_pr']}** (mean {ppl['mean_reviewers_per_pr']})")
    if ppl["top_reviewers"]:
        L.append("- frequent reviewers: " + ", ".join(f"@{r['value']} ({r['count']})" for r in ppl["top_reviewers"][:6]))
    if ppl["top_approvers"]:
        L.append("- frequent approvers: " + ", ".join(f"@{r['value']} ({r['count']})" for r in ppl["top_approvers"][:6]))
    if ppl["reviewer_by_path"]:
        L += ["", "Reviewers by area:"]
        for path, rs in list(ppl["reviewer_by_path"].items())[:10]:
            L.append(f"  - `{path}` → " + ", ".join(f"@{r['value']} ({r['count']})" for r in rs))

    L += ["", "## Meta", ""]
    if m.get("required_label_families"):
        for f in m["required_label_families"]:
            L.append(f"- **`{f['family']}` label appears on {f['coverage']}% of merged PRs — treat as mandatory.** "
                     "Values: " + ", ".join(f"`{v['value']}` ({v['count']})" for v in f["values"]))
    L.append(f"- labelled PRs: {m['labelled_pr_rate']}%"
             + (" · " + ", ".join(f"{l['value']} ({l['count']})" for l in m["label_frequency"][:8])
                if m["label_frequency"] else ""))
    L.append(f"- median size: {m['median_lines_changed']} lines across {m['median_files_changed']} files")
    L.append("- base branches: " + ", ".join(f"{x['value']} ({x['count']})" for x in m["base_branches"]))
    return "\n".join(L) + "\n"


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("dump", help="JSON array from `gh pr list --state merged --json ...`")
    ap.add_argument("--out", default=None, help="write profile JSON here")
    ap.add_argument("--md", action="store_true", help="also write a markdown summary next to --out")
    a = ap.parse_args()

    try:
        with open(a.dump) as fh:
            data = json.load(fh)
    except Exception as e:  # noqa: BLE001
        print(f"could not read {a.dump}: {e}", file=sys.stderr)
        return 1
    if isinstance(data, dict):
        data = data.get("data") or data.get("prs") or []
    if not isinstance(data, list):
        print("expected a JSON array of PRs", file=sys.stderr)
        return 1

    prof = analyse(data)
    out = a.out or os.path.splitext(a.dump)[0] + ".style.json"
    with open(out, "w") as fh:
        json.dump(prof, fh, indent=2)
    md_path = os.path.splitext(out)[0] + ".md"
    if a.md:
        with open(md_path, "w") as fh:
            fh.write(render_md(prof))
    print(f"profile -> {out}" + (f"\nsummary -> {md_path}" if a.md else ""))
    print(f"sample_size={prof.get('sample_size')}")
    if prof.get("warning"):
        print("WARNING: " + prof["warning"])
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
