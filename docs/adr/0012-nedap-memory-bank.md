# Employer facts live in their own memory bank

Shared Memory holds facts from every surface, including facts about employer
(`nedap`) repositories. Those facts belong to the employer. They must not mix
with personal memory, and personal-only agents must not recall them. So a
separate bank, `nedap`, holds every employer fact. Only work surfaces reach
it: the work Gold Band instance (ADR-0011), and direct-lane sessions on
`nedap` repositories (the work Claude profile, and Codex). Shared Memory and
Private Memory never hold employer facts.

The existing employer facts move out of Shared Memory. The work Claude
subscription does the move, so employer content is processed only under the
employer's account. The recall from the `nedap` bank is checked before the
copies are deleted from Shared Memory.

In both Gold Band instances, only judgment nodes may write memory: nodes whose
id ends in `-acceptance` or `-escalation`. Every other node reads only. The
workflow lint enforces this, in the same way as the Opus rule. Private Memory
stays reachable only from `vibe-private`.

## Consequences

- A work session that recalls from Shared Memory will no longer find employer
  facts there; it has to be pointed at the `nedap` bank.
- The move is one-way and ends in deletions, so the recall check is a gate,
  not a formality.
