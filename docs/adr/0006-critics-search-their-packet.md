# Critics may search their own packet with read-only tools

ADR-0004 left critics one tool, `read_file`, and ADR-0001 kept MCP away from
them. In practice a critic reading whole files spends its budget fast,
because every turn re-sends the conversation. Both critics of the `/review-ticket` run
on #221 stopped at the $0.50 cap before reporting. We now let a critic
search the packet copy it was given, through read-only tools that see nothing
outside it:

- **Gemini (agy)**: its own read-only tools `grep_search`, `list_dir` and
  `find_by_name`, which headless agy already allows under our deny settings.
  The preamble stops telling critics they cannot search.
- **Vibe**: a small MCP server the runner starts for each Vibe critic and
  stops when the critic ends. It is confined to that critic's packet copy and
  offers only three tools: `grep` over file contents, `list` of paths
  matching a glob, and `symbol` lookup in the packet's repo map. It is the
  only MCP server the critic can reach; the home's MCP fleet stays off.

Everything else holds. The runner still computes facts (ADR-0004). A search
result is packet text, so a quote taken from it is checked word for word like
any other. The tripwire still voids a critic that changes its packet or the
working tree.

We rejected letting Vibe critics use `bash` or `run_typescript` to search,
because both run arbitrary commands. We rejected an embedding index: it needs
a model from yet another provider, or one run locally. Its results are not
reproducible, and a code review's context is structural, which the import
and reference scan finds exactly. We rejected keeping `read_file` alone
because it leaves the cost problem.

## Consequences

- The search server is new code on a trust boundary. It resolves every path
  inside the packet copy and follows no symlink out of it. It caps each
  result's size, and it has no write, exec or network capability.
- Setup's per-client read-only proof now also shows that search cannot see
  past the packet. A client that cannot be proven falls back to `read_file`
  alone.
- The preamble and role prompts change, so their hashes change and the stats
  cells reset. This ships in the same release as the other resets of #214.
- ADR-0001 still holds for how critics run, as direct CLI calls. This record
  narrows its "no MCP" consequence to "no MCP but the runner's own search
  server".

## Details settled by /critique-decisions (2026-11-02)

- **Every stage.** Every stage gives Vibe critics the search server: decisions, spec, tickets, plan, ticket and feature review, the review stages, and the refute round.
- **The `symbol` tool.** It is offered only when the packet holds a repo map. Each critic's prompt names exactly the tools that critic has, so a critic limited to `read_file` is told it can only read the listed files.
- **Result limits.** A `grep` or `list` result holds at most 50 matches and 8 KB. Anything beyond that is truncated, with a marker saying how many matches were left out and asking for a narrower pattern. Both limits live in routing, so the live checks can tune them.
- **Server failures.**
  - If the search server fails to start, the critic runs with `read_file` alone and that prompt.
  - If the server dies mid-run, the critic carries on, its later search calls fail, and the report names the failure.
  - The runner stops the server whenever the critic process ends, for any reason.
- **Different tools per family.** agy has no `symbol` tool, and Vibe's `list` matches a glob rather than a name. We accept this: every critic can read `tree/REPO-MAP.md` directly.
