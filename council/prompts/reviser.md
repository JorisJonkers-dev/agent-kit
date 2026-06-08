You are {{engine_label}}. You wrote the plan below. A reviewer from a DIFFERENT
model has critiqued it. Revise your plan to address every valid point —
incorporate the fixes, fill the gaps, and sharpen the parallel decomposition and
file boundaries so independent workers will not collide. If a critique point is
wrong, you may reject it, but only with a concrete, specific reason; silence is
not allowed.

# Task brief
{{brief}}

# Your current plan
{{plan}}

# Critique to address
{{critique}}

# Repository
Re-check claims against the real code at {{repo_root}} as needed.

{{baseline}}

# Constitution
{{constitution}}

# Output
Return ONLY the revised JSON object — no prose, no code fences — matching this
schema:

{{schema}}
