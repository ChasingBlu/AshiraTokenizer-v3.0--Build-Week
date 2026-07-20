# Task Board

## Program: AshiraTokenizer v2
Date: 2026-03-03

## Completed
1. Standalone workspace creation.
2. Native Rust trainer implementation.
3. Build and smoke validation.
4. Determinism parity check.
5. IEEE SRS/SDD documentation.
6. Third-party attribution record.
7. Determinism unit test added (`cargo test --release`).

## In Progress
1. Open-source packaging checklist.

## Completed (continued)
8. **32k vocab expansion — VALIDATED 2026-03-06.**
   - Run A: vocab=32768, merges=32492, 108s training core. PASS.
   - Run B: vocab=32768, merges=32492, 109s training core. PASS.
   - Determinism: SHA-256 identical across both runs. PASS.
   - `vocab.bin`:  `B0125CA63232EF6A7B9DDC62AC7D5306F897139BE797AA43E268535927C10CFE`
   - `merges.bin`: `4ABB44F8722B91324FBB10BFEFD656E12C5014DD6CE2EEB83491A8311F27BBC8`
   - Canonical artifacts: `runs/full_32768/`. Committed to MC pipeline.

## Pending
2. Optional CUDA/GPU implementation path with parity checks.
3. Release packaging checklist for public open-source publication.
