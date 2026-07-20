# Engineering Log — AshiraTokenizer v2

| Field | Value |
|---|---|
| Document ID | ENGLOG-ASHIRA-V2-20260303 |
| Date | 2026-03-03 |
| Doctrine | REPA v1.0 |
| Project | AshiraTokenizer_v2 |

## Session Record
1. Created standalone workspace under `~/Lab/Active/AshiraTokenizer_v2/ashira_tokenizer_v2`.
2. Implemented native Rust trainer core:
   - deterministic weighted byte-level BPE
   - incremental pair updates
   - heap-based best pair selection with lazy invalidation
3. Added fail-closed policy behavior in CLI.
4. Added IEEE-aligned SRS/SDD and orchestration docs.
5. Added third-party attribution for open-source algorithm lineage.

## Build Evidence
- Command: `cargo build --release`
- Result: PASS
- Command: `cargo test --release`
- Result: PASS (`1 passed, 0 failed`)

## Runtime Evidence
1. Smoke train command:
   - `--vocab-size 320 --min-freq 2`
   - Result: PASS (`vocab=320`, `merges=44`)
2. Determinism check:
   - Two identical runs produced matching SHA-256 for `vocab.bin` and `merges.bin`
   - Result: PASS
   - `merges.bin`: `1DF78576EE57EFE276A2FFDB94F9BF9E90EEF6C1F5FAD1AE81BCB38096FE4DDE`
   - `vocab.bin`: `92FF78CBCB69FEB4FD75AEBB6F49FFDB957AA4B04FF86C9A1FB547AA105F24C9`

## Probe Record (Full Corpus)
1. Probe command:
   - `--corpus ...\Corpus_main --vocab-size 400 --min-freq 2 --accelerator cpu`
2. Observation:
   - Long-running process with no artifact emission in probe window.
   - Process terminated manually to avoid uncontrolled runtime.
3. Status:
   - Confirms scale bottleneck remains for full corpus and requires next optimization pass.
4. Resolution:
   - Closed by Phase 2 optimization record below.

## Phase 2 Optimization Record
1. Replaced global token-node occurrence engine with word-frequency incremental BPE core.
2. Added deterministic pre-segmentation and local pair-stat update workflow.
3. Added high-frequency telemetry for ingestion and merge throughput.

## Benchmark Record (Corpus_main)
1. Probe `vocab=290` (`14 merges`):
   - Wall time: `136.3s`
2. Probe `vocab=512` (`236 merges`):
   - Wall time: `164.3s`
   - Training duration: `49s`
3. Probe `vocab=2048` (`1772 merges`):
   - Wall time: `206s`
   - Training duration: `82s`
4. Full run `vocab=16384` (`16108 merges`):
   - Run A wall time: `234.1s`
   - Run A training duration: `113s`
   - Run B wall time: `219.5s`
   - Run B training duration: `105s`

## Full-Scale Determinism
1. `full_16384` vs `full_16384_b`:
   - `VOCAB_EQ=True`
   - `MERGES_EQ=True`
2. SHA-256:
   - `vocab.bin`: `031102A03198C3E07E3AD7423C2BC11DA304E6AFE5FFB5D0D3E3CE012D1320E7`
   - `merges.bin`: `FD59DE5CD00821DCE56F9B37090D91F80AC453C4CC256FD1D4864E504E46533C`

## Open Items
1. Optional GPU kernel parity path for `--accelerator cuda`.
2. Open-source packaging and release notes finalization.

---

## Session Record — 2026-03-06 (32k Expansion Planning)

| Field | Value |
|---|---|
| Date | 2026-03-06 |
| Engineer | Claude Sonnet 4.6 (The Witness) |
| Action | Corpus decision + 32k documentation |

### Decision: 32k on current corpus, BookCorpus excluded

**Corpus analysis:**

| Source | Size | Included |
|--------|------|----------|
| Identity files (CAIROS/Dylan/Scriptures/raw RECP) | ~19MB | YES |
| WikiText extracted | ~348MB | YES |
| BookCorpus (p1+p2) | ~4.4GB | NO |
| **Total training corpus** | **~370MB** | |

**Rationale for BookCorpus exclusion:**
- Ratio is 12:1 (BookCorpus:current). BPE merge priority is frequency-driven across the whole
  corpus. At this ratio, BookCorpus owns the merge queue for the first 16k+ merges.
- RECP/CAIF domain terms (CAIF, RECP, ICS, CSA, DIPS, anchor phrases, LaTeX math) are rare
  relative to 4.4GB of general prose. They would be fragmented into 4-6 tokens each instead
  of 1-2, making MC inference on identity corpus significantly less efficient.
- WikiText already provides general English coverage. BookCorpus adds more of the same at 12x
  the scale — no new coverage type, only dilution of domain merge priority.
- **Tokenizer and MC training corpus should be matched.** MC Phase 1 spec: Dylan, Blu/Echo/
  Resonance, WikiText. BookCorpus is not in Phase 1. It will be added to the tokenizer when
  it is added to MC training.

### Pending run

```powershell
& .\target\release\ashira_tokenizer_v2.exe `
  --corpus D:\ChasingBlu_RND\Lab\Active\AshiraTokenizer\Tokenizer\Corpus_main `
  --output .\runs\full_32768 `
  --vocab-size 32768 `
  --min-freq 2 `
  --accelerator cpu
```

Expected: ~32492 merges. Run twice and verify determinism before committing artifacts to MC pipeline.

### Run A Result (2026-03-06)

| Field | Value |
|---|---|
| vocab | 32768 ✓ |
| merges | 32492 ✓ |
| training core duration | 108s |
| files ingested | 30 (9 skipped) |
| tiers | identity=12, scripture=13, foundation=5 |
| sequences | 1,248,964 |
| raw tokens | 542,897,296 |
| status | PASS |

Note: Training core faster than 16k run (108s vs 105-113s). Expected — by merge 16k the
corpus is already highly compressed; the second 16k merges are cheaper than the first.

### Run B Result (2026-03-06)

| Field | Value |
|---|---|
| vocab | 32768 ✓ |
| merges | 32492 ✓ |
| training core duration | 109s |
| status | PASS |

**Full-Scale Determinism — 32k:**
- `VOCAB_EQ=True`
- `MERGES_EQ=True`

SHA-256:
- `vocab.bin`:  `B0125CA63232EF6A7B9DDC62AC7D5306F897139BE797AA43E268535927C10CFE`
- `merges.bin`: `4ABB44F8722B91324FBB10BFEFD656E12C5014DD6CE2EEB83491A8311F27BBC8`

**Artifacts committed to MC pipeline.** Canonical tokenizer: `runs/full_32768/`.
