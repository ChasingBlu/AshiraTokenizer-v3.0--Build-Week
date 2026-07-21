# AshiraTokenizer v3.0 — Build Week Demo

AshiraTokenizer v3.0 is a deterministic Rust tokenizer pipeline built around
versioned `u32` tokenizer artifacts, strict package validation, lossless
encode/decode, and reproducible same-host A/B demo runs.

This repository is the public Build Week demo package. It demonstrates the real
v3 pipeline on a compact WikiText-only corpus so judges can run it quickly from
a fresh clone.

## What This Demo Shows

- Manifest-bound corpus admission.
- Deterministic tokenizer training.
- Versioned V3 artifact publication.
- Strict package readback and validation.
- Lossless encode/decode round trip.
- Independent Run A / Run B generation.
- Byte-for-byte A/B comparison with stable hashes.

## Scope Boundary

This is not the final 128K production tokenizer package.

The demo uses:

- Corpus profile: `demo_wikitext_only`
- Vocabulary size: `512`
- Artifact format: V3 `u32`
- Demo corpus: bundled WikiText slice at `demo/corpus/wikitext.txt`

The v3 implementation is designed for the 128K production track, but this Build
Week repository ships a small, fast, judge-runnable demonstration.

## Requirements

- Rust toolchain with Cargo.
- PowerShell on Windows for the commands below.
- No Python runtime is required.

The commands below use `--locked` so Cargo uses the checked-in `Cargo.lock`.

## Quick Start

Clone the repository:

```powershell
git clone https://github.com/ChasingBlu/AshiraTokenizer-v3.0--Build-Week.git
Set-Location "AshiraTokenizer-v3.0--Build-Week"
```

Check the CLI:

```powershell
cargo run --quiet --locked --bin ashira -- --help
```

Expected output:

```text
Usage:
  ashira encode --package <run_root_or_manifest> --text-file <input.txt> --out <encoded.json>
  ashira decode --package <run_root_or_manifest> --encoded <encoded.json> --out <decoded.txt>
  ashira demo-pipeline --manifest <demo_manifest.json> --run-root <new_run_root> --vocab-size <276..4096>
  ashira demo-compare --run-a <run_a_root> --run-b <run_b_root>
```

## Run The Full Demo

Important: demo output roots must be outside the Git checkout. Ashira refuses to
write generated run folders inside the source tree.

From the repository root:

```powershell
$parent = "$PWD\..\ashira_v3_demo_runs"
$runA = "$parent\run_a"
$runB = "$parent\run_b"

if (Test-Path -LiteralPath $parent) {
  Remove-Item -LiteralPath $parent -Recurse -Force
}

New-Item -ItemType Directory -Force -Path $parent | Out-Null
```

Run A:

```powershell
cargo run --quiet --locked --bin ashira -- demo-pipeline --manifest "demo\demo_wikitext_manifest.json" --run-root $runA --vocab-size 512
```

Run B:

```powershell
cargo run --quiet --locked --bin ashira -- demo-pipeline --manifest "demo\demo_wikitext_manifest.json" --run-root $runB --vocab-size 512
```

Compare A and B:

```powershell
cargo run --quiet --locked --bin ashira -- demo-compare --run-a $runA --run-b $runB
```

Expected final line:

```text
PASS demo-compare label=demo_wikitext_only files=11 bytes=13927 vocab_size=512 merge_count=236 token_count=43 ...
```

The exact hash values may differ from internally archived runs if the public
GitHub commit differs, but Run A and Run B must match each other exactly.

## Encode And Decode Manually

After running the demo pipeline, use one generated package to encode text:

```powershell
$practice = "$parent\practice"
New-Item -ItemType Directory -Force -Path $practice | Out-Null

Set-Content -Encoding utf8 -LiteralPath "$practice\input.txt" -Value "AshiraTokenizer v3 encodes and decodes deterministic tokenizer artifacts."

cargo run --quiet --locked --bin ashira -- encode --package "$runA\package" --text-file "$practice\input.txt" --out "$practice\encoded.json"
```

Decode it:

```powershell
cargo run --quiet --locked --bin ashira -- decode --package "$runA\package" --encoded "$practice\encoded.json" --out "$practice\decoded.txt"
```

Inspect the result:

```powershell
Get-Content "$practice\input.txt"
Get-Content "$practice\decoded.txt"
```

The decoded text should match the input.

## Validation

Run the test suite:

```powershell
cargo test --locked
```

Expected result:

```text
test result: ok. 91 passed; 0 failed
```

Optional stricter local checks:

```powershell
cargo fmt --check --all
cargo clippy --locked --all-targets -- -D warnings
git diff --check
```

## What Is In The Repository

- `src/` — Rust tokenizer, artifact, manifest, codec, and demo pipeline code.
- `demo/` — bundled WikiText demo corpus and manifest.
- `runs/` — historical v2/v3 lineage and compatibility artifacts used by tests.
- `docs/` — SRS, SDD, traceability matrix, artifact format, and Build Week boundary.
- `orchestration/` — portable helper scripts and task-board material.
- `CODEX_DIRECTIVE.txt` — intentionally included base specialist directive showing
  the lab's stepwise governance pattern.

## Claim Boundary

This project is a tokenizer pipeline demonstration. It is not a language model,
not an inference system, and not a final full-corpus 128K tokenizer release.

The demonstrated claim is:

```text
AshiraTokenizer v3 can train, publish, validate, encode/decode, rerun, and
byte-compare deterministic V3 tokenizer artifacts from a fresh public clone.
```
