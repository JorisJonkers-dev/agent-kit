# Critics cite facts the runner computed; they never compute them

A review over a whole repository needs its import graph, cycles, layer
violations, churn and the files that change together. We have the runner's
`prepare` compute these as **facts** into the packet, at a pinned commit and
with the time window counted from that commit's date, so the same commit
always gives the same facts. Critics keep `read_file` as their only tool and
cite facts the way they quote files; a quote from a fact file carries its own
provenance, so a churn number never counts as a recorded decision. Layer
violations come only from the repository's own architecture-test
configuration (dependency-cruiser, import-linter, ArchUnit), run in report mode
when its binary is already installed; the runner never installs a tool.

We rejected giving critics search or execution tools so they could measure the
repository themselves. That would break the read-only guarantee the tripwire
and ADR-0001 rest on, and models count and walk large graphs unreliably, so
their numbers could not be checked. We also rejected inferring layers from
directory names, which would make the runner's guess a rule the code is
judged against.

## Consequences

- Facts are only as good as the runner's scans: a repository in a language the
  import scan does not know gets no import facts, and one with no
  architecture-test configuration gets no layer violations, so design findings
  there can only be questions for the owner.
- Facts count as code under the family allowlist; any stage that sends them
  needs the code permission.
- Packets for repository reviews are sampled and capped (initially 150 KB, files
  cut into 400-line windows that keep their line numbers), and every file left
  out is recorded so a critic knows what it cannot see.
