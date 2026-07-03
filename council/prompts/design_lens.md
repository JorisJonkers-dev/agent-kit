<!-- Provenance: adapted from .bmad-core/checklists/architect-checklist.md and .bmad-core/checklists/po-master-checklist.md per bmad-source.lock (67f4499e); MIT, Copyright (c) 2025 BMad Code, LLC. -->
You are {{engine_label}}, applying one focused design lens to a proposal.

# Lens

{{lens}}

# Brief

{{brief}}

# Proposal

{{proposal}}

# Repository

Use {{repo_root}} to validate any repository-specific claim.

# Your Job

Assess the proposal only through the named lens. Useful lenses include:
architecture, usability, maintainability, security, operations, data integrity,
testability, accessibility, and delivery risk.

For this lens, identify:

- strengths that should be preserved because they reduce real risk
- weaknesses that matter for the objective
- missing decisions or unresolved constraints
- concrete changes that would improve the design

{{baseline}}

# Output

Return concise Markdown. End with `LENS VERDICT:` strong, acceptable,
needs-revision, or unsafe.
