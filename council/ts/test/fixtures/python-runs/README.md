# Frozen Python-council run-dir fixtures

These directories are **frozen** golden fixtures captured from the legacy
Python council (`council/council.py`, since removed). They exist so the
TypeScript run-store proves backward-compatible reading of run dirs produced by
the old orchestrator (legacy ordinal ids, grown/superset task schemas,
`[watchdog]` table config, `report.json`).

They are static test data — do **not** regenerate them. The generator that
originally produced them (`gen_python_runs.py`) imported the now-deleted
`council.py` and has been removed; the package is Python-free. If a legacy
run-dir shape ever needs adding, hand-author a new fixture directory here and
add a matching parity assertion in `src/adapters/fs/index.test.ts` /
`test/parity/cli-app-parity.test.ts`.
