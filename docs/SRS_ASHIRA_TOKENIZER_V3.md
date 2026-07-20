# AshiraTokenizer v3 Software Requirements Specification

**Document ID:** `SRS-ASHIRA-TOKENIZER-V3-R0`
**Stage:** controlled baseline through Stage 3B Step 3 bounded codec evidence
**Status:** `ACTIVE — STAGE 3 PARTIAL`
**Source baseline:** public AshiraTokenizer v2 commit
`f4ad48ff57db2b080d55e8b3a6fb6c71bca0d5c3`
**Controlling directive:** `GPT55-SOL-ASHIRA-V3-STAGE0-3-20260718`
**Claim language:** traceability artifact; no certification or quality claim

**Artifact-contract authority note:** The controlling directive supersedes the
historical approval-pending metadata inside `docs/ARTIFACT_FORMAT_V3_U32.md` for
implementation authority and locks its contract without changing any format
byte. That Stage 1 file remains byte-identical to its verified evidence copy for
audit continuity.

## 1. Purpose

Define auditable requirements for the evolutionary v3 line: a deterministic,
native Rust, weighted byte-level BPE tokenizer with end-to-end `u32` IDs,
explicit V2U16 compatibility, versioned V3U32 artifacts, manifest-only corpus
admission, and bounded-state scaffolding.

This document originated as the Stage 1 requirement baseline and now records
bounded implementation evidence. Each requirement row states its own evidence
boundary; no row implies full-system, corpus-scale, or release readiness.

## 2. Evidence-state vocabulary

| State | Meaning |
|---|---|
| `IMPLEMENTED` | Corresponding code/document exists at the cited revision. |
| `TESTED` | Authorized verification passed and immutable evidence is cited. |
| `PENDING` | Authorized for a later stage but not yet implemented/tested. |
| `BLOCKED` | Prohibited or awaiting a stated authority/evidence gate. |

No requirement is considered closed merely because this SRS contains it.

## 3. Scope

### 3.1 Included through Stage 3

- public-v2 Git lineage and v3 branch;
- typed IDs, pair helpers, base/special/alias policy;
- explicit V2U16 reader and V3U32 reader/writer;
- bounded artifact fixtures and corruption tests;
- final composite-manifest parser/admission validator on bounded fixtures;
- exact pre-segmentation and pair-opportunity calibration scaffolding;
- deterministic `StdShardStoreR1` skeleton and telemetry schema.

### 3.2 Explicitly excluded or blocked

- full corpus hashing;
- 1%/5%/10% probes;
- production checkpoint generation;
- full 131,072-entry training;
- Build Week submission packaging;
- public quality, scalability, compliance, certification, or readiness claims.

## 4. Locked system constants

| Constant | Value | Current evidence state |
|---|---:|---|
| `TokenId` | `u32` | `TESTED_STAGE_2A` |
| `PairKey` | `u64` | `TESTED_STAGE_2A` |
| `BASE_VOCAB_COUNT` | `276` | `TESTED_STAGE_2B` |
| `MAX_VOCAB_SIZE` | `131072` | `TESTED_STAGE_2A` |
| `MAX_TOKEN_ID` | `131071` | `TESTED_STAGE_2A` |
| V3U32 header | `128` bytes | `FOUNDATION_IMPLEMENTED_TESTED_STAGE_2C2` |

## 5. Functional requirements

| Requirement ID | Requirement | Planned stage | Implementation | Verification |
|---|---|---:|---|---|
| `ASH-FR-LIN-001` | V3 shall preserve ancestry from public v2 commit `f4ad48f...` and tree `f6f310c...`. | 0 | `IMPLEMENTED` | `TESTED_STAGE_0` |
| `ASH-FR-ID-001` | Every token-bearing internal/API field shall use the canonical `TokenId = u32`. | 2 | `IMPLEMENTED_CURRENT_PATHS_COMMITTED_EVIDENCE_VERIFIED_STAGE3B3` | typed core, artifacts, merge lookup, codec, JSON cross-u16 vector, and 131072 rejection committed/evidenced |
| `ASH-FR-ID-002` | Target 131072 entries shall be accepted; target 131073 shall be rejected before corpus work. | 2 | `IMPLEMENTED_STAGE_2A` | `TESTED_STAGE_2A` |
| `ASH-FR-ID-003` | ID 131071 shall be valid and ID 131072 shall be rejected. | 2 | `IMPLEMENTED_STAGE_2A` | `TESTED_STAGE_2A` |
| `ASH-FR-ID-004` | ID allocation and conversions shall use checked arithmetic without wrapping, saturation, or silent truncation. | 2 | `IMPLEMENTED_STAGE_2A` | `TESTED_STAGE_2A` |
| `ASH-FR-PAIR-001` | Ordered pair `(a,b)` shall pack only as `((a as u64) << 32) | b as u64`. | 2 | `IMPLEMENTED_STAGE_2A` | `TESTED_STAGE_2A` |
| `ASH-FR-PAIR-002` | Direct pair shifts outside the pair module shall be forbidden. | 2 | `IMPLEMENTED_STAGE_2A` | `STATIC_AUDIT_STAGE_2A` |
| `ASH-FR-BASE-001` | IDs 0-19 shall retain the locked canonical special/reserved bytes. | 2 | `IMPLEMENTED_STAGE_2B` | `UNIT_STATIC_AND_EVIDENCE_VERIFIED_STAGE_2B` |
| `ASH-FR-BASE-002` | IDs 20-275 shall map byte `b` to ID `20+b`; learned IDs shall begin at 276. | 2 | `IMPLEMENTED_STAGE_2A` | `TESTED_STAGE_2A` |
| `ASH-FR-ALIAS-001` | Encoder shall accept the locked canonical/alias table; decoder shall emit canonical bytes only. | 2 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3B3`; local CLI path uses the same codec | locked-alias collapse and canonical decoded-byte/document-hash fixtures committed/evidenced; bundled command fixture pending |
| `ASH-FR-ALIAS-002` | Alias spelling round-trip preservation shall not be claimed. | 1 | `IMPLEMENTED_DOC_POLICY_AND_CODEC_COMMITTED_EVIDENCE_VERIFIED_STAGE3B3` | non-canonical opening/closing aliases collapse to shared IDs and canonical bytes; raw-byte equality deliberately unclaimed |
| `ASH-FR-CODEC-001` | Immutable tokenizers shall encode byte slices by deterministic merge rank using `TokenId = u32` and a bounded adjacency work structure; decoding shall concatenate validated canonical vocabulary bytes and reject invalid/out-of-vocabulary IDs. | 3 | library `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3B3`; command/file path `LOCAL_CANDIDATE_82_OF_82` | ranked/leftmost, structural-byte, non-UTF-8, cross-u16, invalid-ID, resource-limit, and command round-trip fixtures pass; CLI evidence/demo fixture pending |
| `ASH-FR-CODEC-002` | Demo encoded-token JSON shall use schema `ashira_v3_encoded_tokens_v1`, declare `u32`, bind vocabulary/merge sequence, attest canonical decoded length/SHA-256, serialize deterministically, and reject malformed, unknown, inconsistent, oversized, or foreign input. | 3 | library `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3B3`; atomic JSON command output `LOCAL_CANDIDATE_82_OF_82` | canonical reserialization and A/B command bytes plus malformed/invalid/above-131071 failures pass; no binary encoded-corpus format |
| `ASH-FR-ART-001` | Callers shall select exactly `ArtifactFormat::V2U16` or `ArtifactFormat::V3U32`; auto-detection/fallback is forbidden. | 2 | `MANIFEST_BOUND_V3_PACKAGE_AND_CLI_LOCAL_CANDIDATE_82_OF_82`; evidence pending | `EXPLICIT_SELECTION_READER_STAGE_2C4_AND_WRITER_STAGE_2C5_COMMITTED_EVIDENCE_VERIFIED`; command-level headerless rejection passes |
| `ASH-FR-ART-002` | V2U16 loading shall be read-only, strict, and widen IDs without value change. | 2 | `IMPLEMENTED_STAGE_2C4` | `CANONICAL_PUBLIC_32K_PAIRED_RECONSTRUCTION_ID_BYTE_AND_LOOKUP_PARITY_COMMITTED_EVIDENCE_VERIFIED_STAGE_2C4` |
| `ASH-FR-ART-003` | V3U32 shall implement the locked 128-byte header without silent field changes. | 2 | `WRITER_AND_BOUNDARY_VECTORS_COMMITTED_EVIDENCE_VERIFIED_STAGE_2C5`; locked contract unchanged | `HEADER_EVIDENCE_VERIFIED_STAGE_2C2`; paired readers Stage 2C.4; empty/one/cross-u16 complete-file vectors Stage 2C.5 |
| `ASH-FR-ART-004` | V3 paired artifacts shall validate payload and shared merge-sequence SHA-256 values. | 2 | `IMPLEMENTED_STAGE_2C4` | `SEQUENCE_MISMATCH_RECONSTRUCTION_AND_PUBLIC_V2_PARITY_COMMITTED_EVIDENCE_VERIFIED_STAGE_2C4` |
| `ASH-FR-ART-005` | Native artifact publication shall use create-new staging, flush/sync, strict readback, and non-overwriting rename. | 2 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_WINDOWS_STAGE_2C5`; non-Windows no-replace rename fails closed | `48_TESTS_COMMITTED_EVIDENCE_VERIFIED`; package/failure/boundary/runtime-reparse coverage; crash/power-loss durability unclaimed |
| `ASH-FR-MAN-001` | Production composite corpus admission shall use schema `ashira_v3_composite_corpus_manifest_v1`; pattern-only inclusion is forbidden. | 3 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3A` | canonical admit/train/freeze and legacy-scanner-rejection fixtures; full production manifest CLI pending |
| `ASH-FR-MAN-002` | Every enabled entry shall declare family, root ID, normalized path, bytes, SHA-256, SHA-512, and encoding policy. | 3 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3A` | canonical schema, dual-hash, size, encoding, root-set, and four-family fixtures committed/evidenced |
| `ASH-FR-MAN-003` | Admission shall reject absolute/drive/UNC/empty/dot/dot-dot/NUL paths and containment escape. | 3 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3A` | normalized-path, inventory, and containment fixtures committed/evidenced; exhaustive path matrix pending |
| `ASH-FR-MAN-004` | Admission shall reject symlinks, junctions, mount points, and Windows reparse points. | 3 | `PARTIAL_IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_WINDOWS_BOUNDED_STAGE3A` | live Windows reparse fixture committed/evidenced; true Windows file-object/hardlink identity and mount-point portability remain open |
| `ASH-FR-MAN-005` | No undeclared file shall be ingested; all four families shall be nonzero before balanced work. | 3 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3A` | undeclared-file, exact-root-set, and nonzero enabled-family fixtures committed/evidenced; full composite manifest pending |
| `ASH-FR-MAN-006` | The Build Week public-pipeline demo may admit exactly one enabled nonzero WikiText entry only through schema `ashira_v3_demo_wikitext_manifest_v1`, exact label `demo_wikitext_only`, and an explicit demo-only API. This profile shall not satisfy or weaken composite/final-v3 authority. | 3 | `COMMITTED_IMMUTABLE_EVIDENCE_REAL_RUNS_VERIFIED` | exact profile/schema/label/family, cross-profile rejection, bundled-manifest admission, and two real demo runs pass; attribution/license and public packaging remain pending |
| `ASH-FR-PRE-001` | Calibration, training, and encoding shall use one byte-exact production pre-segmenter. | 3 | trainer path `COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3B2`; lossless codec view `COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3B3`; calibration/probes pending | shared line/segment engine, lossless structural-byte reconstruction, and codec integration committed/evidenced; chunked streaming/spill pending |
| `ASH-FR-PRE-002` | Logical-line, one-CR trim, ASCII-whitespace, trailing-whitespace, and special-only rules shall match the approved design. | 3 | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_BOUNDED_BYTE_SLICE_STAGE3B2` | seven exact fixture groups committed/evidenced; chunked streaming/spill parity pending |
| `ASH-FR-CAL-001` | Family opportunity shall be `P_f = sum_t max(len_bytes(t)-1,0)`, never trained-token count. | 3 | `PENDING` | `PENDING` |
| `ASH-FR-CAL-002` | Family weights and tolerance decisions shall use checked integer/rational arithmetic without floating-point priority decisions. | 3 | `PENDING` | `PENDING` |
| `ASH-FR-CAL-003` | Initial opportunity balance shall not be represented as final-vocabulary balance. | 1 | `IMPLEMENTED_DOC` | `DOCUMENT_REVIEW_PENDING` |
| `ASH-FR-STORE-001` | Stage 3 storage shall use in-repo `StdShardStoreR1` with deterministic files/order and no external DB, mmap, network, or ordering-changing background thread. | 3 | `PENDING` | `PENDING` |
| `ASH-FR-STORE-002` | Store writes shall use append journal, committed snapshots, temp-write, flush/sync where supported, and atomic rename. | 3 | `PENDING` | `PENDING` |
| `ASH-FR-OBS-001` | Telemetry schema shall distinguish deterministic-core data from timestamps/machine/run-instance data. | 3 | `PENDING` | `PENDING` |
| `ASH-FR-CLI-001` | The executable shall expose only explicit authorized subcommands and exact options; legacy pattern-training, unsupported accelerator/fallback, unknown, missing, and duplicate forms shall fail closed before work. Writing commands shall use bounded non-overwriting publication; `demo-compare` shall write only its stdout/stderr result. | 2-3 | demo pipeline and comparator `COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED`; comparator commit `80055ec...f2f9f` | exact encode/decode/pipeline/compare parsers, live comparison PASS, A/B JSON, byte round trip, limits, existing-output, rename-race, and reparse fixtures pass; portable durability and retained-default-temp test-fixture rerunability remain open |
| `ASH-FR-DEMO-001` | `ashira demo-pipeline --manifest --run-root --vocab-size` shall accept only the demo-only manifest profile, require clean committed source authority, bound the vocabulary to 276..4096, train/freeze/publish/load/encode/decode/read back on one path, and publish PASS only after final validation. `ashira demo-compare --run-a --run-b` shall read two distinct governed run roots without writing, strictly validate each run, require exact 11-file byte parity, recompute aggregate hashes, and emit one concise PASS summary. | 3 | pipeline and comparator `COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED`; comparator evidence `SOL_STAGE3B5_D4_WIKITEXT_AB_COMPARATOR_20260720.json` SHA-256 `4463C88B...D99CA` | real A/B 11/11 parity, fresh-root 91/91 regression, and committed live comparator aggregate reproduction pass; default-temp retained-fixture rerunability, cross-host/rebuild, full-128K/composite, and public packaging gates remain open |

## 6. Quality and assurance requirements

| Requirement ID | Requirement | Status |
|---|---|---|
| `ASH-NFR-DET-001` | Identical bounded input/configuration shall produce byte-identical deterministic-core artifacts. | commit `80055ec...f2f9f` and evidence `SOL_STAGE3B5_D4_WIKITEXT_AB_COMPARATOR_20260720.json` SHA-256 `4463C88B...D99CA` verify real same-host WikiText A/B equality for all 11 files, run-tree `A15E6B2B...E40A2C`, and deterministic-core `9EB041C6...39C0B`; cross-host/rebuild proof pending |
| `ASH-NFR-SAF-001` | Malformed artifacts/manifests/encoded-token documents shall fail before unsafe allocation or state publication. | committed artifact/admission/codec gates plus local bounded package/CLI reads and pre-output validation pass 82/82; handle-bound path identity, Windows hardlink identity, portable rename, and scale residuals open |
| `ASH-NFR-RES-001` | Corpus-dependent state shall not grow resident memory without a configured bound. | `PENDING_STAGE_3`; scale evidence blocked |
| `ASH-NFR-AUD-001` | Commands, exit codes, hashes, source commits, changed files, and residual blockers shall be handed off. | Stage 3B4 pipeline/run/parity receipts plus Stage 3B5 comparator commit `80055ec...f2f9f` and 15,239-byte immutable evidence SHA-256 `4463C88B...D99CA` verified; retained-default-temp test-harness rerunability recorded for later correction |
| `ASH-NFR-LIC-001` | V3 package metadata shall be Apache-2.0; every dependency/version/license/reason/scope shall be recorded. | `PARTIAL_STAGE_2C5`; project plus 20 locked external packages reconciled, release attribution/toolchain evidence pending |
| `ASH-NFR-STD-001` | Standards mapping shall be clause/practice-to-evidence traceability and shall not claim certification or generic compliance. | `SKELETON_IMPLEMENTED`; primary-source verification pending |
| `ASH-NFR-PORT-001` | Binary contract shall be implementation-language independent and explicitly little-endian. | `INDEPENDENT_COMPLETE_FILE_VECTORS_COMMITTED_EVIDENCE_VERIFIED_STAGE_2C5`; locked bytes unchanged; full cross-platform publication readiness unclaimed |
| `ASH-NFR-REG-001` | Public-v2 V2U16 artifacts shall remain readable only through explicit compatibility mode. | `IMPLEMENTED_COMMITTED_EVIDENCE_VERIFIED_STAGE_2C4` |

## 7. Verification boundary

Stage 2/3 acceptance requires recorded `cargo fmt`, `cargo clippy -D warnings`,
`cargo test`, and `git diff --check` results using a target directory that does
not overwrite tracked public-v2 `target/` evidence.

Full-corpus hashing, deterministic probes, full training, and local submission
packaging are operator-authorized but unexecuted. Production checkpoint
generation, separate Run A/B replications, external upload, quality, and
performance claims remain outside current evidence and cannot close any
requirement in this SRS.

## 8. Stage 1 status

- Requirements identified: `IMPLEMENTED_DOC`.
- Production implementation: `PENDING_STAGE_2_OR_3`.
- Code tests: `NOT_RUN`.
- Stage 0 lineage: `TESTED_STAGE_0`.
- Full-scale claims: `BLOCKED`.

## 9. Stage 2A typed-core status

- Existing token-bearing trainer, merge, lookup, and special-ID paths:
  `TokenId = u32`.
- Pair packing and unpacking: centralized and boundary-tested in `src/pair.rs`.
- Target/ID/allocation bounds: checked and boundary-tested in `src/token.rs`.
- Base byte IDs `20..=275`: implemented and tested in Stage 2A. Canonical
  special/alias policy is recorded separately in Stage 2B below.
- Headerless inherited artifact publication: disabled and tested fail-closed;
  Stage 2C.2 adds the explicit format type and V3 header foundation. Stage 2C.3
  adds strict individual V2U16/V3U32 inspection, and Stage 2C.4 adds paired
  tokenizer loading. Stage 2C.5 now adds validated publication context,
  canonical package-manifest types, and bounded Windows package publication;
  independent boundary vectors are committed/evidenced and CLI selection remains
  pending.
- Bounded tests: passed under compiler warnings-as-errors in an external target
  directory. The operator-installed Clippy component is available and the
  denied-warning gate passes without lint suppression.
- Clippy closure evidence:
  `governance/SOL_STAGE2A_CLIPPY_CLOSURE_20260718.json`.

State: `STAGE_2A_IMPLEMENTED_AND_TESTED / STAGE_2_REMAINS_PARTIAL`.

## 10. Stage 2B canonical special/alias status

- `src/token.rs` contains the authority-locked 20-entry canonical table for IDs
  `0..=19`; ID 19 has empty canonical bytes and no accepted alias.
- The 44 byte-exact public-v2 code/fixture spellings are accepted aliases for
  IDs `0..=18`. No new alias was introduced, and aliases are not independent
  vocabulary records.
- `canonical_special_bytes` exposes canonical bytes by ID;
  `special_token_id` and `match_special_alias_prefix` map accepted byte strings
  to canonical IDs. Prefix selection is longest-match first with an ordinal-byte
  tie break, and sequence advancement uses checked arithmetic.
- Trainer base-vocabulary initialization emits canonical bytes only. The
  inherited special-only pre-segmentation predicate now uses the locked alias
  policy.
- Six Stage 2B unit tests cover exact canonical bytes, alias uniqueness and
  canonical mapping, reserved ID 19, byte-exact concatenated matching, canonical
  trainer vocabulary, and trainer alias recognition. The complete bounded suite
  passes 16/16 tests with compiler warnings denied.
- Format, Git whitespace, all-target Clippy with warnings denied, and all 17
  retained static authority/policy gates pass.
- Full encoder/decoder integration, explicit V2U16 alias loading, and artifact
  readers/writers remain pending. The Stage 2B immutable evidence record and
  source commit are verified.
  Therefore `ASH-FR-ALIAS-001` is not closed by Stage 2B policy helpers alone.

State: `STAGE_2B_COMMITTED_AND_EVIDENCE_VERIFIED / CODEC_INTEGRATION_PENDING`.

## 11. Stage 2C.2 typed artifact/header foundation status

- Added exactly two artifact-format variants (`V2U16`, `V3U32`), two artifact
  kinds, caller-supplied resource-limit fields, metadata, and the locked error
  classes. There is no auto, unknown, filename, extension, or fallback mode.
- Implemented an exact 128-byte V3U32 header encoder/parser with locked magic,
  version, kind, little-endian marker, u32 ID width, kind-specific record width,
  flags/reserved rejection, count relations, checked file length, and merge
  payload/sequence-digest equality.
- Pinned approved `sha2` 0.11.0 with default features disabled. All nine locked
  direct/transitive packages resolve to `MIT OR Apache-2.0` and are recorded in
  the dependency matrix; release attribution assembly remains pending.
- Tests cover the locked empty-merge SHA-256, locked one-record merge payload
  SHA-256, a real canonical base-vocabulary header, fixed-field mutation order,
  count/vocabulary boundaries, exact header length, file truncation/trailing
  data, and constructor relation failures.
- Bounded gate: 22 tests pass; format, Git whitespace, and all-target Clippy with
  warnings denied pass using the external Cargo target.
- This slice does not implement `inspect_artifact`, either file reader, payload
  digest verification against file bytes, V2 widening, paired tokenizer
  construction, semantic reconstruction, writer/publication, manifest, or CLI
  integration. Those requirements remain open.

State: `STAGE_2C2_COMMITTED_AND_EVIDENCE_VERIFIED` at commit
`3c1437b45a8f70ee0c27087e9aed48168f0794cf`; immutable evidence SHA-256
`3CD15AE1BF2350F67BF9EC7C705F97FC9632D2A373F8BA67BB3372A810EB5F7F`.

## 12. Stage 2C.3 strict individual-reader status

- `inspect_artifact` opens only read-only, requires an explicit format and kind,
  obtains exact file size, and enforces `max_file_bytes` before format parsing.
- V3U32 inspection validates the locked header, stream-hashes the declared
  payload before semantic parsing, parses with remaining-byte accounting, and
  hashes the exact bytes consumed by a successful semantic pass. A same-length
  between-pass payload change is rejected instead of returning trusted metadata.
- Vocab inspection proves each declared length fits the remaining payload before
  fallible allocation, applies per-token and cumulative byte limits, validates
  all 276 canonical base records, and forbids empty learned records.
- Merge inspection uses canonical `TokenId` and `PairKey` types and rejects
  out-of-range IDs, non-sequential results, forward/self references, duplicate
  ordered pairs, and non-exhausted payloads.
- V2U16 inspection never writes or falls back. It widens each u16 merge field
  without value change, validates the inherited grammar and canonical base, and
  makes digest absence explicit with `None` rather than fabricated zero hashes.
  For headerless V2 metadata, `header_bytes = 0` and `payload_bytes = file_bytes`
  describe the complete legacy grammar, including its leading count.
- The tracked canonical public-v2 32,768-entry artifacts pass strict individual
  inspection. This does not yet prove a usable tokenizer: paired count/sequence
  identity, learned-token reconstruction, immutable tokenizer construction, and
  decoded-byte parity remain Stage 2C.4 work.
- Bounded gate: 32 tests pass; format, Git whitespace, and all-target Clippy with
  warnings denied pass using the controlled external Cargo target.

State: `STAGE_2C3_INDIVIDUAL_READERS_COMMITTED_AND_EVIDENCED_WITH_STAGE_2C4`.

## 13. Stage 2C.4 paired package and immutable-tokenizer status

- Added public `load_tokenizer_package` with the exact contract signature. Both
  paths are opened read-only and parsed under one caller-selected format; no
  auto-detection, fallback, mixed-format retry, or V2 writer exists.
- Parsing now retains validated vocab and merge records privately. The loader
  compares base/vocab/merge/record counts, applies V3 shared-sequence and merge-
  payload digest equality, and then reconstructs every learned token from its
  exact operands without allocating a concatenation buffer.
- `Tokenizer` has private vocab, merge, and lookup state. Only read-only size,
  token-byte, merge-order, and pair-lookup accessors are public. Construction
  occurs only after both files, paired metadata, reconstruction, and fallible
  lookup allocation pass; failure returns no partially usable object.
- V2 lacks an embedded sequence digest, so its pair binding is exact count plus
  full learned-token reconstruction. The tracked canonical public-v2 32,768
  package loads under explicit V2U16 mode and matches known ID/byte anchors at
  IDs 0, 20, 275, 276, and 32,767. All 32,492 merge-lookup values equal their
  widened merge records.
- Bounded V3 fixtures prove successful construction, shared-sequence mismatch
  rejection, reconstructed-byte mismatch rejection, exact ordered lookup, and
  mixed-format rejection. No test fixture writes or deletions were introduced.
- Commit `a96a8826d0b6273ee938e3d787a5359f2c7e63c5` and immutable evidence
  `governance/SOL_STAGE2C4_PAIRED_ARTIFACT_LOADER_20260719.json` at SHA-256
  `AB21F89A9B939A52ED5D8AD3ADD5055019BBFAE26053B77CFB93921EB17AA5D3`
  bind the 36-test, format, Clippy, traceability, and exact-file closure.

State: `STAGE_2C4_PAIRED_LOAD_COMMITTED_AND_IMMUTABLE_EVIDENCE_VERIFIED / WRITER_PUBLICATION_PENDING`.

## 14. Stage 2C.5 publication implementation status

- GPT-55 ruling
  `GPT55_ASHIRA_V3_STAGE2C5_PUBLICATION_API_RULING_20260719.md` authorizes the
  explicit context-bearing writer API without changing V3U32 binary bytes.
- Added private validated `PublicationContext` state constructed only from a
  named input object. IDs and labels are bounded; absolute-path characters,
  zero digests, zero/uppercase/malformed Git object IDs, self-parent lineage,
  and fabricated/hidden authority inputs fail closed.
- Added deterministic schema `ashira_v3_artifact_package_manifest_v1` using a
  fixed private wire struct, fixed vocab-before-merges artifact array, uppercase
  digest text, compact UTF-8 JSON, and exactly one terminal LF. Map iteration,
  timestamps, paths, durations, and machine/process identity cannot enter the
  manifest model.
- Public parse returns a validated wrapper rather than exposing Serde
  `Deserialize`; unknown fields, inconsistent counts/lengths/sequence digests,
  noncanonical whitespace/case, missing readback assertions, and input above
  the 32 KiB manifest cap fail before a manifest object is returned.
- Exactly pinned `serde` 1.0.229 and `serde_json` 1.0.150 resolve with nine new
  transitive packages. The complete project-plus-20-external-package closure is
  recorded; release attribution remains pending, including Unicode-3.0 notice
  handling for `unicode-ident` and the MIT option for `memchr`.
- Four new bounded tests bring the local suite to 40 and cover accepted context,
  hidden/fabricated authority rejection, canonical deterministic round-trip and
  artifact ordering, manifest size/case/unknown-field/count/sequence rejection.
  An independent ordered PowerShell generator agrees on the 2,245-byte sample
  manifest SHA-256
  `1C0B117DA9B0BB1EAD64392F807B0FDBEAD11002B2628BB80AD83BBFA9C95E99`,
  which is locked in the Rust test.
- `write_v3_package(tokenizer, destination, context)` now measures and streams
  locked V3 headers/records without complete payload buffers, uses same-parent
  create-new staging, flushes/syncs both files, reloads strict paired V3U32 with
  exact tokenizer equality, independently recomputes complete-file SHA-256 and
  SHA-512, writes and strictly rereads the canonical manifest last, then uses a
  non-overwriting final-directory rename on Windows.
- Published results expose fixed relative filenames, counts, artifact and
  manifest hashes, and explicit file/directory sync capability. Windows reports
  directory sync as unsupported and makes no crash/power-loss durability claim.
  Non-Windows builds fail closed before rename because the Rust standard library
  cannot prove directory no-replace semantics there.
- Eight integration/core/boundary tests bring the suite to 48. They cover two byte-identical
  path-distinct public-v2-32K conversions, strict source equality, existing-final
  preflight, all eight pre-rename interruption phases, corruption before
  manifest publication, manifest-last visibility, rename-race preservation,
  independent empty/one/cross-u16 complete-file bytes, and runtime Windows
  directory-symlink rejection before staging.
- The formal visibility claim is: every pre-rename failure leaves no final
  directory; after successful rename the package is visible, and no rollback or
  renewed invisibility is claimed without prohibited deletion.
- Exact commit `0bd850f87e08ccce3b3b7840dd1e8d86098ff7d9` and immutable
  evidence `governance/SOL_STAGE2C5_V3_PACKAGE_PUBLICATION_20260719.json` at
  SHA-256
  `B36125D2F8CED19CCA50BD9B24A1653440846993812E14757126A8EBB6B14F65`
  bind the 48-test, format, Clippy, 38/38 traceability, exact 10-file, and
  independent boundary-vector closure.
- At Stage 2C.5 evidence time, non-Windows no-replace support, Windows
  crash/power-loss durability, CLI integration, codec, trainer-freeze bridge,
  manifest/admission, training, and packaging were pending. Section 15 records
  the later bounded local freeze/admission work without rewriting that evidence.

State: `STAGE_2C5_WINDOWS_BOUNDED_PUBLICATION_COMMITTED_AND_IMMUTABLE_EVIDENCE_VERIFIED / FULL_CROSS_PLATFORM_READINESS_NOT_CLAIMED`.

## 15. Bounded Stage 3 manifest admission and trainer-freeze status

- `src/manifest.rs` accepts only canonical compact JSON plus one LF under
  schema `ashira_v3_composite_corpus_manifest_v1`. It requires sequential
  ordinals, normalized forward-slash relative paths, exact supplied/declaration
  root-set equality, explicit family/enabled/bytes/SHA-256/SHA-512/encoding
  fields, and nonzero enabled-byte coverage of identity, scripture, wikitext, and
  bookcorpus.
- Admission inventories every supplied root and requires the declared file set
  to equal that inventory. It rejects link-like ancestors and entries, proves
  canonical containment, checks size and dual hashes, applies UTF-8 policy when
  selected, and rechecks identity/size/content immediately before training
  reads. Disabled declarations are verified but never returned for ingestion.
- `AdmittedCorpus::training_files` is the same production path consumed by the
  inherited weighted trainer. The trainer rejects every unadmitted file before
  corpus reads, so the retained predecessor directory scanner cannot authorize
  v3 training. `TokenizerTrainer::freeze` consumes the trainer, verifies
  lookup/merge agreement, and constructs the existing private immutable
  `Tokenizer` only after base, merge-sequence, pair-uniqueness, and
  reconstruction validation succeeds.
- The bounded same-path fixture admits all four families, trains one merge, and
  freezes a tokenizer with the expected `aa` token. Negative fixtures cover
  canonical form, traversal, root-set mismatch, nonzero family coverage, undeclared
  files, dual-hash/UTF-8 mismatch, post-admission mutation, live Windows
  reparse entries, corrupted base vocabulary, and inconsistent trainer lookup.
  The committed suite passes 55/55 and all-target Clippy with warnings denied.
- This closes the demo-critical bounded bridge only. Stable Rust 1.94 does not
  expose Windows volume/file-index metadata; the current safe Windows identity
  stamp cannot prove hardlink alias uniqueness. No unsafe Win32 FFI or new
  dependency was introduced. Full composite-manifest admission, production
  hardlink/object identity, pre-segmentation/calibration/store wiring,
  full-corpus hashing/training, codec, and CLI remain pending.

- Exact commit `8322363b865155183116fa5440fe29a028de91d8` and immutable
  evidence `governance/SOL_STAGE3A_BOUNDED_MANIFEST_FREEZE_20260719.json` at
  SHA-256
  `B5F36F260DE91CC19BC19B16767CFAAFE2D68D6C45CB45FAF828F9C3EF8A6D53`
  bind the six-path, 55-test, 38/38 traceability, toolchain, rejected-diagnostic,
  and residual-claim record.

State: `BOUNDED_MANIFEST_TO_TRAINER_TO_IMMUTABLE_TOKENIZER_COMMITTED_AND_IMMUTABLE_EVIDENCE_VERIFIED_STAGE3A / PRODUCTION_WINDOWS_OBJECT_IDENTITY_OPEN`.

## 16. Bounded shared pre-segmentation status

- `src/presegment.rs` owns version `ashira_v3_presegment_v1` and is the only
  successful trainer path for logical-line and pre-segment construction. The
  inherited private CR-trim and line-split implementations were removed.
- The allocation-free visitor splits exact bytes at LF, excludes LF, removes
  exactly one terminal CR, preserves and skips the final empty segment after a
  terminal LF, rejects empty and locked-alias-only lines, and never emits an
  empty segment.
- ASCII whitespace is explicitly limited to bytes `09`, `0A`, `0B`, `0C`,
  `0D`, and `20`. A maximal whitespace run attaches to the following maximal
  non-whitespace run; trailing whitespace is emitted alone. No pair opportunity
  crosses a line or pre-segment boundary.
- Checked counters expose accepted/skipped lines, bytes, segments, and initial
  pair opportunities. Trainer counter aggregation and weighted word-frequency
  addition now fail on overflow instead of wrapping.
- Seven module tests cover LF/CRLF/doubled CR/unterminated input, terminal-LF
  empty preservation, every possible byte's whitespace classification,
  whitespace runs and trailing whitespace, empty/special-only/concatenated
  alias/alias-plus-space behavior, non-UTF-8 data, a 16,385-byte segment, and
  consumer failure. The committed suite passes 62/62 with denied-warning
  Clippy. With the separately typed demo-manifest admission fixtures, the
  complete local suite now passes 85/85.
- This is bounded byte-slice evidence, not the Stage 3 streaming/spill layer.
  Calibration, probes, codec, chunk-boundary/spill parity, bounded-state store,
  and corpus-scale behavior remain pending and must reuse this module rather
  than reimplement its semantics.

- Exact commit `f70464f075bd847cb511430971251832612c3020` and immutable
  evidence `governance/SOL_STAGE3B2_BOUNDED_PRESEGMENTER_20260719.json` at
  SHA-256
  `F6D68AB1782F5BA5E0026CC998D1089CC6CE32BA7049146BF9D01D8523C3A447`
  bind the five committed paths, 62/62 gates, exact fixtures, rejected
  diagnostics, tool-artifact correction, and residual boundaries.

State: `BOUNDED_SHARED_PRESEGMENT_CORE_AND_TRAINER_INTEGRATION_COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED_STAGE3B2 / STREAMING_SPILL_AND_OTHER_CONSUMERS_OPEN`.

## 17. Bounded codec status

- `src/codec.rs` adds caller-bounded in-memory encode/decode over the immutable
  validated `Tokenizer`. Encoding uses a linked adjacency vector plus a
  rank/left-position priority heap; it never performs predecessor-style
  interior vector removal or scans the full merge list per replacement.
- The encoder reuses `src/presegment.rs` through a lossless view of the shared
  line/segment engine. Training-only omission is not applied to user text:
  terminal CR/LF bytes and empty-line delimiters are emitted as literal base
  tokens, while merges remain unable to cross logical-line boundaries.
- Locked aliases are recognized by the existing deterministic longest-prefix
  table and become their shared special IDs. JSON evidence binds canonical
  decoded bytes, not the discarded alias spelling. Therefore non-canonical
  aliases are accepted and decode successfully without creating a false raw
  byte-round-trip claim.
- Schema `ashira_v3_encoded_tokens_v1` uses ordered compact JSON plus one LF,
  explicit `u32`, vocabulary size, merge count, exact V3 merge-sequence
  SHA-256, canonical decoded byte count/SHA-256, token count, and a `u32`
  token array. Serialization is bounded without cloning the token array;
  parsing denies unknown fields and validates caller limits and binding/count/
  digest representations before decode.
- The committed suite passes 71/71 tests, including deterministic rank behavior,
  exact CR/LF/non-UTF-8 reconstruction, canonical alias collapse, stable JSON,
  foreign-tokenizer/tamper rejection, malformed/schema/width/count/range/size
  failures, and a real token ID `65536` through encode, JSON, and decode.
- Exact commit `70949771f356c8ef9a622fb7f0da12f95415f605` and immutable
  evidence `governance/SOL_STAGE3B3_BOUNDED_CODEC_20260719.json` at SHA-256
  `BB96E00A5392B26B83637C7FD949C5D2C96BF2DE4AF082DEE4B3EA61720E82EF`
  bind the six committed paths, 71/71 postcommit gates, 40/40 traceability,
  independent JSON/sequence/cross-u16 vectors, and residual claim boundary.
- The committed Stage 3B3 unit did not itself close D4: package-loading CLI,
  atomic file output, a bundled canonical demo fixture, and command-level byte
  comparison were still pending at that commit. The in-memory implementation
  is bounded by caller-supplied limits; streaming encoded-document I/O is not
  claimed.
- The current D4 file-integration candidate adds
  `load_v3_tokenizer_package(package, limits)`. It accepts only an existing
  package directory or its exact `package_manifest.json`, rejects direct
  vocab/merge and headerless paths, parses the canonical manifest under its
  32 KiB cap, enforces the caller's artifact-file limit before hashing, checks
  declared complete-file SHA-256/SHA-512 both before and after explicit V3U32
  paired loading, compares loaded counts, and rereads the manifest before the
  immutable tokenizer escapes. Directory/manifest selection, corrupted
  complete-file bytes, and live Windows directory-reparse fixtures passed in
  the initial 74/74 resolver micro-step.
- The resolver and command/file additions are not yet committed or evidenced.
  The bundled canonical demo fixture, judge orchestration, and command-level
  immutable evidence remain pending.

### 17.1 D4 explicit command/file candidate

- Cargo now names the executable `ashira`. Its active entrypoint accepts only
  the exact directive forms `encode --package --text-file --out` and
  `decode --package --encoded --out`; option order is flexible, but missing,
  duplicate, unknown, non-UTF-8 option names, extra operands, and inherited
  `--corpus` training forms fail with usage exit 2. The inherited pattern CLI is
  physically retained under always-disabled item guards for auditability and
  cannot execute.
- The command policy fixes explicit caps: 512 MiB per artifact, 256 MiB total
  vocabulary bytes, 16 MiB per token, 8 MiB input, 8,388,608 token IDs, 32 MiB
  decoded bytes, and 128 MiB encoded JSON. Reads reject link/reparse ancestry,
  non-regular files, pre-read oversize, detected length/path mutation, and
  limit-plus-one growth before bytes reach the codec.
- Output parents must already exist and be non-link directories; an existing
  final path fails before package/input work. The Windows writer creates one of
  at most 1,024 unique same-parent files with `create_new`, writes, flushes,
  `sync_all`s, performs exact bounded readback, rechecks final absence, renames
  without replacement, and rereads the visible final bytes. Pre-rename failure
  never exposes final and retained staging is not silently deleted. After a
  successful rename, final is visible and no rollback/invisibility is claimed.
- Eight focused command fixtures cover deterministic A/B JSON, exact canonical
  byte round-trip including CR/LF and non-UTF-8 bytes, malformed JSON, invalid
  ID 276 for a 276-entry package, ID 131072, headerless package selection,
  bounded input, existing-output preservation before reads, legacy/unknown/
  duplicate/missing arguments, output reparse rejection, and late-output
  rename-race preservation. The live `ashira --help` path succeeds and inherited
  `--corpus` exits 2; the complete local suite passes 82/82 and denied-warning
  Clippy.
- This remains a local candidate. Encoded JSON is bounded but materialized in
  memory; no streaming encoded-corpus format is authorized. Non-Windows
  no-replace output fails closed. Windows directory sync, crash/power-loss
  durability, adversarial handle-bound path identity, commit/evidence, and
  judge orchestration remain open.
- One create-new WikiText demo member now exists at
  `demo/corpus/wikitext.txt`. It is 1,047,897 bytes from 2,187 nonempty rows of
  only the two operator-supplied train Parquet shards. The deterministic
  extraction receipt is
  `governance/SOL_DEMO_WIKITEXT_EXTRACTION_20260719.json`; an independent
  full-source rescan reconstructed the exact output SHA-256
  `71A024D25F902E2F5911E475A68DB9749D58AA560A3A37A5BAE1A0FE20867454`.
  The source files do not self-bind dataset revision or license, so this is
  described only as operator-supplied WikiText-labelled data.
- The operator-relayed GPT-55 ruling authorizes this slice as the sole D4 demo
  corpus member. `demo/demo_wikitext_manifest.json` uses separate schema
  `ashira_v3_demo_wikitext_manifest_v1` and exact label
  `demo_wikitext_only`. `admit_demo_wikitext_manifest` accepts it while
  `admit_corpus_manifest` rejects it; the reverse cross-profile rejection also
  passes. This local demo authority does not claim balanced-composite coverage,
  final-v3 training authority, or closure of identity/scripture/BookCorpus
  gates. External attribution/license remains required before public submission
  packaging, but not before local demo execution.
- The local `demo-pipeline` candidate accepts the exact directive form
  `--manifest <demo_manifest.json> --run-root <new_run_root> --vocab-size
  <276..4096>`. It requires the run-root parent to exist outside the Git source
  tree and rejects an existing final before source or corpus work. It refuses a
  dirty source, derives commit/tree plus a normalized tracked-file aggregate
  digest from a clean checkout, and repeats that authority check after training
  before publication. This prevents an uncommitted candidate from being
  mislabeled with the prior HEAD.
- One execution admits `DemoWikitextOnly`, builds deterministic configuration
  plus explicit calibration/probe not-applicable assertions, trains with fixed
  minimum frequency/weight/backend, consumes the trainer into an immutable
  tokenizer, publishes V3 artifacts, loads the manifest-bound package, encodes
  and canonically reparses the fixed 66-byte demo input, decodes exact bytes,
  and writes a deterministic run manifest. The entire run is assembled under a
  same-parent create-new staging directory and renamed without replacement on
  Windows.
- The staged run manifest is deliberately
  `PREPUBLICATION_VALIDATED`, not `PASS`. After rename, every visible nonpackage
  file is reread, the package is reloaded, and only then is create-new
  `demo_final_validation.json` written with `PASS`. Post-rename failure leaves a
  visible incomplete run without a false PASS marker; no rollback/invisibility
  is claimed. Two clean fixture roots produce identical artifacts, package
  manifest, configuration, encoded/decoded bytes, run manifest, final marker,
  and deterministic-core digest. The committed pipeline passed 88/88 before
  immutable evidence and real WikiText execution.
- Real WikiText-only Runs A/B from the same clean evidenced source commit match
  all 11 governed files. The committed `demo-compare` implementation accepts only
  `--run-a` and `--run-b`, rejects the same root and non-exact topology,
  validates canonical run/config/assertion/final/encoded documents, strictly
  reloads and inspects both V3 packages, repeats the byte round trip, compares
  every complete file, and recomputes the run-tree and deterministic-core
  SHA-256 values. It performs no file write, retraining, publication, or current
  HEAD substitution. Commit `80055ecbbb8e46d51edd9d9a54098041778f2f9f`
  and immutable evidence
  `governance/SOL_STAGE3B5_D4_WIKITEXT_AB_COMPARATOR_20260720.json` SHA-256
  `4463C88BE5622E7A52F048AEB025626A567632D83241F22E8252C84A69ED99CA`
  bind the seven-path implementation and its 91/91 fresh-root acceptance gate.
- GPT-55 reported that a later repeated `cargo test` in the default temporary
  directory collided with a retained fixture root, while the same 91 tests
  passed with a fresh temporary root. Inspection shows several test-only helpers
  derive directory names from process ID plus a process-local ordinal and retain
  them for audit, so PID reuse can make `create_dir` encounter an earlier root.
  This is an open test-harness rerunability defect, not evidence of comparator or
  tokenizer output failure; its code fix is outside this citation-only step.

State: `D4_PIPELINE_COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED /
REAL_WIKITEXT_V512_A_B_11_OF_11_PARITY /
DEMO_COMPARE_COMMIT_80055EC_IMMUTABLE_EVIDENCE_4463C88B_VERIFIED /
DEFAULT_TEMP_FIXTURE_RERUNABILITY_FIX_PENDING`.
