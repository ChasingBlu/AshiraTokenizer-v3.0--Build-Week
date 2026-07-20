# IEEE SDD — AshiraTokenizer v2

| Field | Value |
|---|---|
| Document ID | SDD-ASHIRA-V2-20260303 |
| Status | Active |
| Doctrine | REPA v1.0 |
| Date | 2026-03-03 |

## 1. Architecture
`ashira_tokenizer_v2` is a Rust native binary with two layers:
1. `main.rs` (CLI, policy gates, orchestration)
2. `lib.rs` (scanner + deterministic weighted BPE trainer + artifact writer)

## 2. Core Data Model
- Token IDs:
  - `0..19`: reserved/special tokens
  - `20..275`: base byte tokens
  - `276..`: learned BPE tokens
- Training graph:
  - Doubly-linked token-node arrays (`token`, `prev`, `next`, `weight_id`)
  - `pair_counts: HashMap<u64, i64>`
  - `pair_occurrences: HashMap<u64, Vec<u32>>`
  - `BinaryHeap<PairCandidate>` for best-pair retrieval with lazy invalidation

## 3. Training Algorithm
1. Scan and classify corpus files; deterministic sort by path.
2. Ingest lines into node arrays and initial pair statistics.
3. Iterative merge loop:
   - pop best valid pair from heap
   - apply merges only to live occurrences
   - update local affected pair counts only (left and right neighbors)
   - push updated counts back to heap
4. Stop on vocab target or minimum frequency threshold.

## 4. Determinism Controls
1. File ordering by absolute path.
2. Pair priority: highest count, then smallest pair key.
3. Scaled integer weights (`WEIGHT_SCALE=1000`) to avoid FP drift in pair statistics.

## 5. Fail-Closed Policy
- `--accelerator cpu`: execute.
- `--accelerator cuda` without `--allow-cpu-fallback`: hard fail.
- `--accelerator cuda` with explicit fallback: proceed on CPU and log warning.

## 6. Output Contract
- `vocab.bin`: `[u32 vocab_size][u32 len + bytes]*`
- `merges.bin`: `[u32 merge_count][u16 a][u16 b][u16 merged]*`
- `tokenizer_config.json`: run metadata and deterministic hash (`fnv1a64`).

## 7. Validation Status (2026-03-03)
1. Release build: pass.
2. Smoke corpus run: pass.
3. Determinism hash parity over repeated run: pass.

