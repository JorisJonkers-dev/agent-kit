# Tasks: 001-watchdog-table-config

<!-- council-tasks-format: v1 -->

## ck-a100: Touch watchdog table config
<!-- council-task-id: ck-a100 -->
```json
{
  "acceptance_criteria": [
    "The [watchdog] table remains present."
  ],
  "boundaries": "Keep edits inside config/service.toml.",
  "depends_on": [],
  "dev_notes": "[watchdog]\ninterval = \"30s\"",
  "difficulty": "moderate",
  "id": "ck-a100",
  "model": "haiku",
  "objective": "Exercise a task that mentions a [watchdog] TOML table.",
  "output_format": "A deterministic result for a table-bearing config.",
  "paths": [
    "config/service.toml"
  ],
  "title": "Touch watchdog table config",
  "verify": "rg '\\[watchdog\\]' config/service.toml"
}
```
