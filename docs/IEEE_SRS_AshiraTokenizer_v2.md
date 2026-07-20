# IEEE SRS — AshiraTokenizer v2

| Field | Value |
|---|---|
| Document ID | SRS-ASHIRA-V2-20260303 |
| Status | Active |
| Doctrine | REPA v1.0 |
| Owner | ChasingBlu R&D |
| Date | 2026-03-03 |

## 1. Purpose
Define functional and non-functional requirements for a standalone, open-source-capable, deterministic tokenizer trainer that does not depend on Python runtime handoffs.

## 2. Scope
- Byte-level BPE training for weighted corpus tiers.
- Native executable training pipeline.
- Binary artifact output compatible with existing consumers.

## 3. Functional Requirements
1. Load corpus files recursively from operator-provided directory.
2. Classify files into tiers (`foundation`, `scripture`, `identity`) and apply fixed weights.
3. Train weighted byte-level BPE to target vocabulary size.
4. Persist artifacts:
   - `vocab.bin`
   - `merges.bin`
   - `tokenizer_config.json`
5. Enforce deterministic merge selection.
6. Support fail-closed mode for non-active accelerators.

## 4. Non-Functional Requirements
1. Determinism: same corpus/config => identical artifacts.
2. Traceability: config and run log written for each run.
3. Independence: no Python runtime dependency in training path.
4. Portability: output artifacts independent of trainer implementation language.

## 5. Interface Requirements
- CLI flags:
  - `--corpus`
  - `--output`
  - `--vocab-size`
  - `--min-freq`
  - `--accelerator`
  - `--allow-cpu-fallback`
  - `--vram-budget` (compatibility arg)

## 6. Verification Criteria
1. Build success in release mode.
2. Smoke train success on small corpus.
3. Determinism hash equality across repeat runs.
4. Fail-closed behavior when unsupported accelerator is requested without fallback authorization.

## 7. Validated Configurations

| vocab-size | corpus | merges | status |
|------------|--------|--------|--------|
| 16384 | identity + WikiText (~370MB) | 16108 | VALIDATED 2026-03-03 |
| 32768 | identity + WikiText (~370MB) | 32492 | VALIDATED 2026-03-06 |

**Corpus inclusion policy:** BookCorpus (4.4GB) is excluded from tokenizer training corpus until
Morphine Child Phase 1 training data explicitly includes it. Inclusion at 12:1 ratio would
dominate merge priority and fragment RECP/CAIF domain vocabulary.

