# How model addresses and runs fail

Callers reach models through four addresses (#934):
- `free/<model>`, a chain across every free provider of the model;
- `paid/<model>`, the paid routes in a fixed order;
- `auto/<model>`, the free chain and then the paid one;
- `<harness>:<pricing>/<model>`, which also names the harness that runs the work.

A review of those decisions on 2026-11-26 found that none of them said what happens when a chain runs out, a pinned harness cannot start, a run goes quiet, or a free account stops being free. These are the answers.

- **A free account that starts charging is blocked.** "Free" means an account with no payment method. Nothing re-reads that fact, so the proxy treats any non-zero cost, or any billing answer, on a deployment in a `free/` group as proof that the account changed. That provider is blocked in every `free/` group and an alert is raised. The provider-key script in the estate repository flags it too.
- **A chain that is used up fails at once.** When every provider of `free/<model>` is limited, the caller gets an error straight away. The error names when the first provider resets. The proxy does not hold the request.
  - A 429 without `Retry-After` blocks that provider for its route's own cooldown.
  - `free/` never falls through to `paid/`; only `auto/` does that.
- **A spent paid budget is skipped.** In `paid/<model>` a provider whose budget is spent is skipped like a limited free provider, and the chain goes on. The address is refused only when every paid provider of the model is spent.
- **`auto/` is refused outside public repositories.** On a private repository, or one whose visibility cannot be read, `auto/<model>` is refused with the policy reason. It does not quietly run only the paid half. The caller uses `paid/<model>` explicitly.
- **A pinned harness is never swapped.** When the harness in `<harness>:<pricing>/<model>` cannot start, the run fails with that harness's reason. ADR-0007's move to another client of the same cost class applies only to unpinned addresses.
- **A stall is silence from the harness.** Only output from the harness counts as progress; the gateway's own heartbeat does not.
  - After 5 minutes of silence the gateway writes a `stall` event.
  - After 15 minutes it cancels the run, with failure kind `stalled`.
  - A run may set its own threshold.
- **Retention keeps the result.** After 30 days the gateway deletes a run's `events.jsonl`, output and stderr.
  - `result.json` and the ledger line stay.
  - The global index marks the log as pruned.
  - Runs on private repositories are never exported or synced off the machine.

## Considered options

- **Waiting for a free provider to reset.** Rejected: a held request stalls the batch that sent it, and the caller can decide to use `auto/` instead.
- **Swapping a pinned harness.** Rejected: a pin is an instruction, and silently breaking it hides the problem it was meant to show.
- **Counting the heartbeat as progress.** Rejected: the heartbeat only says the process is alive, so a hung harness would never stall.
- **Relying on the docs warning for accounts that gain a payment method.** Rejected: it is the one failure that costs money, so the proxy watches for it.
