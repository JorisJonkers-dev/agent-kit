#!/usr/bin/env python3
"""Check a drafted PR body against the repo's template before showing it to anyone.

The failure this prevents: silently inventing a section, dropping checklist items,
reordering the template, or flattening heading levels. All four look fine in a
paraphrased preview and only become visible after the PR is created.

Usage:
    python3 check_body.py --template /tmp/pr-ctx/template.md --body /tmp/pr-ctx/body.md
    python3 check_body.py -t tpl.md -b body.md --skipped "Screenshots,Migration"

Exit 0 = clean. Exit 1 = violations (printed). Exit 2 = bad invocation.
Stdlib only.
"""
from __future__ import annotations

import argparse
import re
import sys

HEADING = re.compile(r"^(\s{0,3})(#{1,6})\s+(.+?)\s*#*\s*$")
CHECKBOX = re.compile(r"^\s*[-*]\s*\[( |x|X)\]\s*(.+?)\s*$")
PLACEHOLDER = re.compile(r"«([^»]*)»")
FENCE = re.compile(r"^\s*(```|~~~)")


def parse(text: str) -> tuple[list[dict], list[str]]:
    """Return (headings, checklist item texts), ignoring fenced code blocks."""
    headings: list[dict] = []
    boxes: list[str] = []
    in_fence = False
    for i, line in enumerate(text.splitlines(), 1):
        if FENCE.match(line):
            in_fence = not in_fence
            continue
        if in_fence:
            continue
        m = HEADING.match(line)
        if m:
            headings.append({"level": len(m.group(2)), "text": m.group(3).strip(), "line": i})
            continue
        b = CHECKBOX.match(line)
        if b:
            boxes.append(b.group(2).strip())
    return headings, boxes


def norm(s: str) -> str:
    """Compare heading text loosely: case, punctuation, emoji padding, bold markers."""
    s = re.sub(r"[*_`]", "", s)
    s = re.sub(r"[^\w\s]+", " ", s)
    return re.sub(r"\s+", " ", s).strip().lower()


def main() -> int:
    ap = argparse.ArgumentParser()
    ap.add_argument("-t", "--template", required=True)
    ap.add_argument("-b", "--body", required=True)
    ap.add_argument("--skipped", default="",
                    help="comma-separated headings the user chose to omit (not reported as missing)")
    a = ap.parse_args()

    try:
        tpl_raw = open(a.template, encoding="utf-8").read()
        body_raw = open(a.body, encoding="utf-8").read()
    except OSError as e:
        print(f"cannot read input: {e}", file=sys.stderr)
        return 2

    t_head, t_boxes = parse(tpl_raw)
    b_head, b_boxes = parse(body_raw)
    skipped = {norm(s) for s in a.skipped.split(",") if s.strip()}

    t_by_norm = {norm(h["text"]): h for h in t_head}
    b_by_norm = {norm(h["text"]): h for h in b_head}

    problems: list[str] = []
    warnings: list[str] = []

    # 1. Invented sections — the worst failure, because they look authoritative.
    for h in b_head:
        if norm(h["text"]) not in t_by_norm:
            problems.append(
                f"INVENTED SECTION: '{h['text']}' (body line {h['line']}) is not in the template. "
                "Remove it, or ask the user before adding a section the repo doesn't use.")

    # 2. Dropped sections.
    for h in t_head:
        n = norm(h["text"])
        if n not in b_by_norm and n not in skipped:
            problems.append(
                f"MISSING SECTION: template has '{h['text']}' (template line {h['line']}) "
                "and the body doesn't. Include it, or pass it in --skipped if the user skipped it.")

    # 3. Heading level changes — '##' silently becoming '#' reflows the whole PR.
    for n, bh in b_by_norm.items():
        th = t_by_norm.get(n)
        if th and th["level"] != bh["level"]:
            problems.append(
                f"HEADING LEVEL CHANGED: '{bh['text']}' is {'#' * bh['level']} in the body "
                f"but {'#' * th['level']} in the template. Match the template exactly.")

    # 4. Order. Compare only sections present in both, so a skip doesn't read as a reorder.
    t_order = [n for n in (norm(h["text"]) for h in t_head) if n in b_by_norm]
    b_order = [n for n in (norm(h["text"]) for h in b_head) if n in t_by_norm]
    if t_order != b_order:
        problems.append(
            "SECTION ORDER CHANGED. Template order: "
            + " → ".join(t_by_norm[n]["text"] for n in t_order)
            + "  |  body order: " + " → ".join(b_by_norm[n]["text"] for n in b_order)
            + ". Restore the template's order — link blocks in particular belong where "
              "the template puts them, not wherever they read best.")

    # 5. Checklist completeness. Templates use checklists as review gates; dropping
    #    an item removes a gate, and a summarized preview hides that.
    b_norm_boxes = {norm(x) for x in b_boxes}
    for item in t_boxes:
        if norm(item) not in b_norm_boxes:
            problems.append(f"DROPPED CHECKLIST ITEM: '{item}'. Reproduce every item verbatim.")
    if t_boxes and len(b_boxes) < len(t_boxes):
        problems.append(f"CHECKLIST TRUNCATED: template has {len(t_boxes)} items, body has {len(b_boxes)}.")

    # 6. Placeholders — reported, never auto-removed.
    ph = PLACEHOLDER.findall(body_raw)
    if ph:
        warnings.append(f"{len(ph)} unfilled placeholder(s): "
                        + "; ".join(f"«{p}»" for p in ph[:6])
                        + (" …" if len(ph) > 6 else ""))

    # 7. Leftover template instructions.
    leftover = re.findall(r"<!--(.*?)-->", body_raw, re.S)
    if leftover:
        warnings.append(f"{len(leftover)} HTML comment(s) from the template still present — "
                        "keep them only if the repo's merged PRs also keep them "
                        "(check comments_retained_rate in the style profile).")

    print(f"template sections: {len(t_head)} · body sections: {len(b_head)} · "
          f"template checkboxes: {len(t_boxes)} · body checkboxes: {len(b_boxes)}")
    if skipped:
        print("user-skipped: " + ", ".join(sorted(skipped)))

    for w in warnings:
        print(f"  warn  {w}")
    for p in problems:
        print(f"  FAIL  {p}")

    if problems:
        print(f"\n{len(problems)} violation(s). Fix the body and re-run before rendering the preview.")
        return 1
    print("\nbody matches template" + (" (with warnings)" if warnings else ""))
    return 0


if __name__ == "__main__":
    raise SystemExit(main())
