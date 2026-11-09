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

Private Memory gets the same pass. It sits behind the passphrase gate, so no
agent can search it on its own. The owner starts the pass through
`vibe-private` and types the passphrase. The agent then finds the employer
facts and moves them to the `nedap` bank, using the same recall check before
anything is deleted.

In both Gold Band instances, only judgment nodes may write memory: nodes whose
id ends in `-acceptance` or `-escalation`. Every other node reads only. The
workflow lint enforces this, in the same way as the Opus rule. The
judgment-node rule applies only inside Gold Band. Direct-lane sessions on
`nedap` repositories read and write the `nedap` bank, as they write Shared
Memory today. A session's bank is chosen by the repository's remote owner.
Private Memory stays reachable only from `vibe-private`.

## Consequences

- A work session that recalls from Shared Memory will no longer find employer
  facts there; it has to be pointed at the `nedap` bank.
- The move is one-way and ends in deletions, so the recall check is a gate,
  not a formality.
