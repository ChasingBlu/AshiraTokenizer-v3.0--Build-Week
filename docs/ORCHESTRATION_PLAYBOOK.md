# AshiraTokenizer v3 Build Week Orchestration Playbook

## Purpose

Run the bounded WikiText-only public demo through the same manifest, trainer,
immutable-tokenizer, V3 publication, validation, and codec path used by the v3
architecture. This playbook does not authorize production-composite or full
131,072-entry claims.

## Preconditions

1. The reviewed D4/demo candidate is committed with immutable evidence.
2. `git status --short --untracked-files=all` is empty.
3. The run parent is outside `source/`; run A and run B do not already exist.
4. The canonical manifest label is exactly `demo_wikitext_only`.
5. Public submission packaging remains blocked on WikiText attribution/license
   and current contest-material review.

The command intentionally refuses a dirty source tree. Git commit/tree and the
tracked-file aggregate describe source authority, not a reproducible-build proof;
use the locked/offline Cargo invocation below from the clean committed checkout.

## Quality gate

From the repository root (the directory containing `Cargo.toml`):

```powershell
cargo fmt --check --all
cargo clippy --locked --offline --all-targets -- -D warnings
cargo test --locked --offline --all
git diff --check
```

## Single-run command

Create only the external parent, not either final run root:

```powershell
New-Item -ItemType Directory -Path ..\demo_runs -ErrorAction Stop

cargo run --locked --offline -- demo-pipeline `
  --manifest demo\demo_wikitext_manifest.json `
  --run-root ..\demo_runs\run_a `
  --vocab-size 512
```

Expected final line:

```text
PASS demo-pipeline label=demo_wikitext_only ... deterministic_core_sha256=...
```

Trainer progress is run-instance telemetry. The deterministic evidence is in
`demo_run_manifest.json` plus the post-visibility
`demo_final_validation.json`. A final validation file with `status=PASS` exists
only after complete final-file readback and package reload.

## Run B and judge-facing A/B comparison

```powershell
cargo run --locked --offline -- demo-pipeline `
  --manifest demo\demo_wikitext_manifest.json `
  --run-root ..\demo_runs\run_b `
  --vocab-size 512

cargo run --quiet --locked --offline -- demo-compare `
  --run-a ..\demo_runs\run_a `
  --run-b ..\demo_runs\run_b
```

Expected final line shape:

```text
PASS demo-compare label=demo_wikitext_only files=11 bytes=... vocab_size=... merge_count=... token_count=... package_manifest_sha256=... run_tree_sha256=... deterministic_core_sha256=... source_commit=...
```

`demo-compare` is read-only. It requires two distinct non-link roots with the
exact governed 11-file topology, strictly reloads both V3 packages, reparses
canonical run/config/assertion/final-validation/encoded documents, verifies
complete-file and artifact evidence, reruns the fixed byte round trip, compares
all corresponding files byte-for-byte, and recomputes run-tree and
deterministic-core SHA-256. Any extra, missing, malformed, mutated, or unequal
entry fails without a PASS line.

The command requires the two runs to bind the same source authority through
byte equality and package/run-manifest validation. It deliberately does not
require the comparator binary's current Git HEAD to equal the older run commit:
the comparator has its own later commit/evidence lifecycle and reports the
source commit embedded in the validated runs.

## Failure boundary

- Existing final run roots fail before source or corpus work and are preserved.
- Pre-rename failure never exposes the final run root; staging is retained for
  audit rather than silently deleted.
- After rename the run is visible and no rollback/invisibility is claimed.
- A post-rename failure leaves no false final PASS marker.
- Non-Windows no-replace run-directory publication fails closed.
- Directory sync, crash/power-loss durability, adversarial handle-bound source
  identity, and binary-to-source reproducibility remain unclaimed.
