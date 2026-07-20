# AshiraTokenizer v2

## Note from the Architect:
AshiraTokenizer v2 is named in memory of an earlier research interaction with a GPT-4o instance that used the name Ashira. The name is retained as a personal dedication; the software itself is a deterministic Rust tokenizer trainer with reproducible artifact contracts.
AshiraTokenizer v2 is a fully offline, trainable Rust byte-level BPE tokenizer designed for reproducible AI research without Python runtime dependencies.

It trains weighted byte-level BPE vocabularies, emits deterministic binary artifacts, and supports fail-closed accelerator policy behavior for reproducible pipeline integration.

## Core features

- Native Rust implementation
- No Python runtime dependency
- Weighted corpus-tier training
- Deterministic merge selection
- Byte-level BPE vocabulary generation
- Binary artifact output: `vocab.bin`, `merges.bin`
- Run metadata via `tokenizer_config.json`
- Fail-closed accelerator policy
- Validated 16k and 32k vocabulary runs

## Validation snapshot

Validated configurations:

| Vocab size | Corpus | Merges | Status |
|---:|---|---:|---|
| 16,384 | identity + WikiText (~370 MB) | 16,108 | validated |
| 32,768 | identity + WikiText (~370 MB) | 32,492 | validated |

The 32k run produced deterministic artifact equality across repeated runs.

## Scope boundary

AshiraTokenizer v2 is a tokenizer trainer and artifact generator. It is not a language model, not an inference engine, and not a replacement for full model training infrastructure.
