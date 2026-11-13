# Code-navigation measurements

The measurement record for B7 (spec #392, ticket B7-11). It holds the question-set
runner's per-layer results: tokens and accuracy per question and family, before
and after each layer. The raw rows are in
`evals/code-nav/results/2026-11-13-layers.jsonl`; the tables below are
`code-nav-eval table` over that file. Add a dated file under
`evals/code-nav/results/` and a section here for every later run.

## What was run

The runner is `scripts/crates/code-nav-eval` (B7-02) with the frozen set
`evals/code-nav/questions.json`. Layers are cumulative, and each is a context
or tool the agent is given on top of the one before it:

| Layer | Adds |
|---|---|
| L0 | Baseline: file reading and a shell over the repository snapshot. |
| L1 | The repository's `AGENTS.md` (B7-03) put in front of the question. |
| L2 | code-nav `search`, `outline` and `expand` (B7-06), through `cn.py`. |
| L3 | code-nav `map` (B7-07). |
| L4 | code-nav `symbols` and `related_repos` (B7-08). |

The transcript analyser (B7-01, #516) reads finished transcripts and gives the
tool mix; it is not a layer an agent is given, so it has no row.

The layers are set by `evals/code-nav/adapters/vibe_layer.py`, the runner's
generic adapter, which puts the layer after `@` in `--model` so one results
file holds every layer. `cn.py` is the shell front for the code-nav MCP server.
Vibe starts every layer with the same shell and file-reading tools (its file
tools come as one group), so a layer differs by what the agent is told and
given, not by which tools are switched off.

Models, the cheapest the gateway lists for the Vibe lanes: `mistral-small-latest`
(trivial tier) and `zai-glm-5-3-flash` (routine tier). Two runs of every
question per model and layer, at most 15 turns a run.

### What was not run

- **Only one question was asked: FR-02.** The run was kept to this public
  repository, and FR-02 is the only question in the set that is about
  agent-kit. The other 19 questions are about other personal repositories and
  were not sent to a model. So there are **no results for the concept, name-known,
  refs or cross-repo families**, and FR-02 is a freshness question.
- The pin for FR-02 (`8e6eeb8`) predates `AGENTS.md`, so L1 gives the agent the
  current `AGENTS.md`. It names no path for FR-02's answer; it is a pointer to
  `CLAUDE.md`.
- Two runs of one question cannot rank layers. Read the figures as the proof
  that the runner, the layers and the table work end to end, not as a result.

## Results

Cost: both models are in the subsidised class and Vibe reported 0.00 USD for
every run. Total tokens for the 20 runs: 2,935,745 (input and output; about 2.1 million of the input were cache reads).

Per question and per family are the same here, since each family holds one
question. Each cell is `correct/runs, median tokens (input+output), median tool
calls, mean gold-file recall`.

| Layer | mistral-small-latest | zai-glm-5-3-flash |
|---|---|---|
| L0 | 0/2, 154,234 tok, 12 calls, recall 0.00 | 0/2, 168,396 tok, 13 calls, recall 0.00 |
| L1 | 1/2, 86,165 tok, 5 calls, recall 0.50 | 1/2, 93,734 tok, 8.5 calls, recall 0.25 |
| L2 | 1/2, 226,732 tok, 12.5 calls, recall 0.50 | 1/2, 81,432 tok, 8 calls, recall 0.50 |
| L3 | 1/2, 159,824 tok, 11 calls, recall 0.25 | 1/2, 121,650 tok, 87 calls, recall 0.25 |
| L4 | 0/2, 242,599 tok, 15 calls, recall 0.25 | 0/2, 133,109 tok, 12 calls, recall 0.00 |

Per run:

| Model@layer | Run | Correct | Gold recall | Input tok | Output tok | Tool calls |
|---|---|---|---|---|---|---|
| mistral-small-latest@L0 | 1 | no | 0.00 | 123,349 | 1,002 | 9 |
| mistral-small-latest@L0 | 2 | no | 0.00 | 183,087 | 1,030 | 15 |
| mistral-small-latest@L1 | 1 | no | 0.50 | 70,411 | 796 | 5 |
| mistral-small-latest@L1 | 2 | yes | 0.50 | 100,038 | 1,084 | 5 |
| mistral-small-latest@L2 | 1 | yes | 1.00 | 295,113 | 1,312 | 12 |
| mistral-small-latest@L2 | 2 | no | 0.00 | 155,541 | 1,497 | 13 |
| mistral-small-latest@L3 | 1 | no | 0.00 | 212,406 | 1,362 | 14 |
| mistral-small-latest@L3 | 2 | yes | 0.50 | 104,897 | 983 | 8 |
| mistral-small-latest@L4 | 1 | no | 0.50 | 290,722 | 1,315 | 15 |
| mistral-small-latest@L4 | 2 | no | 0.00 | 192,167 | 993 | 15 |
| zai-glm-5-3-flash@L0 | 1 | no | 0.00 | 141,636 | 1,320 | 12 |
| zai-glm-5-3-flash@L0 | 2 | no | 0.00 | 192,397 | 1,439 | 14 |
| zai-glm-5-3-flash@L1 | 1 | no | 0.50 | 91,682 | 964 | 8 |
| zai-glm-5-3-flash@L1 | 2 | yes | 0.00 | 93,658 | 1,164 | 9 |
| zai-glm-5-3-flash@L2 | 1 | yes | 1.00 | 70,859 | 772 | 7 |
| zai-glm-5-3-flash@L2 | 2 | no | 0.00 | 89,938 | 1,294 | 9 |
| zai-glm-5-3-flash@L3 | 1 | no | 0.00 | 93,522 | 803 | 9 |
| zai-glm-5-3-flash@L3 | 2 | yes | 0.50 | 146,783 | 2,192 | 165 |
| zai-glm-5-3-flash@L4 | 1 | no | 0.00 | 131,445 | 1,458 | 12 |
| zai-glm-5-3-flash@L4 | 2 | no | 0.00 | 131,926 | 1,388 | 12 |

### What the figures do show

- Every L0 run missed (0 of 4); the two models both stayed on whole-tree
  `find`, `ls` and `grep` and ran to 12 to 15 calls.
- Runs that gave no answer within 15 turns count as wrong: three of
  mistral-small-latest's (L0 run 2, L4 runs 1 and 2) and one of
  zai-glm-5-3-flash's (L3 run 1, an empty answer). On mistral-small-latest the
  L4 runs used more tokens than L0 (median 242,599 against 154,234) and did
  not answer more questions right.
- Input tokens are dominated by re-sent context: about 73 percent of input
  tokens were cache reads, and a run's token count follows its turn count.
  Vibe's fixed tool and context preamble is in every run.
- No layer is shown to help or to hurt. The spread inside a cell (one run
  right, one wrong) is as large as the spread between layers.

### To make the record decisive

Run the whole set (`code-nav-eval run`, all 20 questions, the layers above, at
least 3 runs) on the owner's workstation where the other repositories may be
read, then add the results here and read the two triggers in
[ADR-0015](adr/0015-no-vector-index-for-code.md).
