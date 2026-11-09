# Gold Band roles from the critique pipeline

Gold Band calls a critic a **profile**: a Markdown file with YAML front
matter that it reads from `~/.gold-band/context/profiles/<profile name>.md`.
The `gold-band-roles` crate (`scripts/crates/gold-band-roles`) renders the
six critique roles into that directory (#345): the decision-gap critic, the
spec red team, the ticket linter, the diff reviewer, the feature reviewer
and the adversarial verifier. Each generated profile matches its source
files rule for rule — the body is the role file verbatim, then the shared
preamble and output contract, then the finding schema the contract points
at, then the lens texts the role uses — with templating off
(`dynamicTemplate: false`) and a fixed stamp, so a second run changes
nothing. Gold Band's own built-in profiles are reused, never generated.
Each profile closes with a section "What this role cannot enforce", which
states the critique runner's guarantees that a Gold Band path loses —
read-only proof, packet confinement, the quote check, schema enforcement,
the refute round, the family allowlist and the nedap refusal, plan-limit
routing — and says the role cannot enforce them itself.

`render` writes the six profiles; `converge` keeps the profiles directory
at them: it removes a retired role and a generated one that is no longer
one of the six, and never touches a hand-made profile (no front matter
`id`) or a Gold Band built-in (`pf-builtin-*`). The setup wiring lands in
#345's follow-up ticket.

## Role to agent

The orchestrator that drafts the decisions, the spec, the tickets and the
implementation of every ticket is the anthropic family, so each reviewing
role runs on another family — one critic per family, the way the critique
pipeline's stages are routed. Every family named below is one the model
gateway's catalogue offers (`skills/workstation/model-gateway/defaults.json`),
the same four the critique clients come from, and never any other.

| Role | Families that may run it | Reason |
| --- | --- | --- |
| Decision-gap critic | mistral, google, zhipu | The decisions were drafted by anthropic, so each gap critic runs on a family other than the author's, one critic per family. |
| Spec red team | mistral, google, zhipu, deepseek | The spec was written by anthropic, and each attacking family differs from the author's and from its fellow critics'. |
| Ticket linter | deepseek, mistral | The tickets were cut by anthropic, so the linters run on other families. |
| Diff reviewer | google, mistral | The ticket was implemented by anthropic, so the second reviewer's family differs from the implementer's. |
| Feature reviewer | mistral, google, zhipu, deepseek | The feature's author and per-ticket implementers are anthropic, so every reviewer differs from the author's family. |
| Adversarial verifier | mistral, google, zhipu, deepseek, whichever did not raise the finding | The verifier checks one accepted critical finding, so its family must differ from the family that raised it and from the orchestrator's (anthropic). |

The family allowlist is not enforced on this Gold Band path: a Gold Band
workflow that runs one of these roles sends the artifact to whatever
family its agent declarations name, and nothing here consults the critique
pipeline's per-repository allowlist entry or its nedap refusal. The
profiles say so themselves in their closing section, and the owner who
wants the pipeline's guarantees runs the roles only from a workflow that
follows this table.
