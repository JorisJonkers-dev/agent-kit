# Tasks: 001-legacy-ordinal-ids

<!-- council-tasks-format: v1 -->

## T1: Preserve first ordinal task
<!-- council-task-id: T1 -->
```json
{
  "boundaries": "Only touch legacy/t1.txt.",
  "depends_on": [],
  "difficulty": "trivial",
  "id": "T1",
  "model": "haiku",
  "objective": "Exercise a legacy T1 task id.",
  "output_format": "A committed deterministic worker result.",
  "paths": [
    "legacy/t1.txt"
  ],
  "title": "Preserve first ordinal task",
  "verify": "test -f legacy/t1.txt"
}
```

## T2: Preserve dependent ordinal task
<!-- council-task-id: T2 -->
```json
{
  "boundaries": "Only touch legacy/t2.txt.",
  "depends_on": [
    "T1"
  ],
  "difficulty": "moderate",
  "id": "T2",
  "model": "sonnet",
  "objective": "Exercise a legacy T2 task id that depends on T1.",
  "output_format": "A committed deterministic worker result.",
  "paths": [
    "legacy/t2.txt"
  ],
  "title": "Preserve dependent ordinal task",
  "verify": "test -f legacy/t2.txt"
}
```
