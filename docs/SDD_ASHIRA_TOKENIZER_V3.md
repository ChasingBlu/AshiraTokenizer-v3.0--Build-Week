# AshiraTokenizer v3 Software Design Description

**Document ID:** `SDD-ASHIRA-TOKENIZER-V3-R0`
**Stage:** controlled architecture through Stage 3B4 bounded demo A/B evidence
**Status:** `ACTIVE — STAGE 3 PARTIAL`
**Lineage:** evolutionary branch of public AshiraTokenizer v2 commit `f4ad48f...`
**Artifact contract:** `docs/ARTIFACT_FORMAT_V3_U32.md`

**Artifact-contract authority note:** The controlling directive
`GPT55-SOL-ASHIRA-V3-STAGE0-3-20260718` supersedes that file's historical
approval-pending metadata for implementation authority. The byte-verified Stage
1 contract remains unmodified for audit continuity; this note changes no binary
format field or requirement.

## 1. Design intent

Retain proven v2 semantics—byte-token layout, deterministic pre-segmentation,
lexical word ordering, left-to-right pair replacement, and highest-count then
smallest-pair-key selection—while replacing width, scale, admission,
publication, and provenance defects.

The planned module structure is an incremental extraction/refactor of the v2
codebase, not a greenfield tokenizer.

## 2. Evidence-state vocabulary

- `IMPLEMENTED`: source exists at the cited revision.
- `TESTED`: authorized test/evidence passed.
- `PENDING`: designed for Stage 2/3 but not implemented.
- `BLOCKED`: outside current authority or awaiting evidence.

## 3. System decomposition

| Planned component | Responsibility | Current state |
|---|---|---|
| `token` | `TokenId`, bounds, base/special/alias contract | `IMPLEMENTED_TESTED_STAGE_2B`; artifact and bounded codec consumers integrated |
| `pair` | `PairKey`, centralized pack/unpack, pair ordering | `IMPLEMENTED_TESTED_STAGE_2A` |
| `artifact` | explicit V2U16/V3U32 readers, V3 writer, paired validation | typed API/header and paired immutable-tokenizer load `COMMITTED_EVIDENCE_VERIFIED_STAGE_2C4`; writer/boundary vectors `COMMITTED_EVIDENCE_VERIFIED_STAGE_2C5` |
| `publication` | validated deterministic provenance, package manifest, staging/readback/hashes/rename | bounded Windows package state machine `COMMITTED_EVIDENCE_VERIFIED_STAGE_2C5`; portable no-replace rename pending |
| `manifest` | schema parsing, normalized paths, containment, file admission | `COMMITTED_EVIDENCE_VERIFIED_BOUNDED_STAGE3A`; Windows hardlink identity residual |
| `presegment` | exact logical-line and byte pre-segmentation | trainer view `COMMITTED_EVIDENCE_VERIFIED_STAGE3B2`; lossless codec view `COMMITTED_EVIDENCE_VERIFIED_STAGE3B3`; streaming/spill/calibration/probes pending |
| `calibration` | pair opportunities, integer weights, reports | `PENDING_STAGE_3` |
| `store::shard` | `StdShardStoreR1` journal/snapshots | `PENDING_STAGE_3` |
| `telemetry` | deterministic schema plus run-instance metrics | `PENDING_STAGE_3` |
| `trainer` | inherited BPE semantics over typed/bounded state | bounded admitted-input and consuming immutable freeze bridge `COMMITTED_EVIDENCE_VERIFIED_STAGE3A`; scale redesign pending |
| `codec` | canonical bounded encode/decode and schema-versioned u32 JSON | `COMMITTED_EVIDENCE_VERIFIED_STAGE3B3`; CLI/file/demo integration pending |
| `cli` | explicit formats/config/fail-closed validation | `PENDING_STAGE_2_3` |

`src/lib.rs` and `src/main.rs` retain the inherited trainer structure but their
current token-bearing paths are migrated to the checked typed core. Ambiguous
headerless artifact publication is disabled. Stage 2C.2 installs the typed
artifact/header foundation, Stage 2C.3 adds strict individual inspection, and
Stage 2C.4 adds paired immutable-tokenizer loading. Stage 2C.5 keeps publication
separate and now implements the approved context-bearing package API. The
bounded Stage 3 bridge routes admitted files through the inherited weighted
trainer and consumes validated trainer state into the immutable tokenizer; no
CLI invokes publication or training yet.

Canonical special bytes and accepted alias spellings are separate immutable
tables in `src/token.rs`. Vocabulary initialization consumes only the canonical
table. Policy lookup consumes only the alias table, maps directly to canonical
IDs, uses deterministic longest-prefix/ordinal-byte selection, and retains no
original-spelling state. This design intentionally cannot promise alias spelling
round-trip preservation. The bounded codec now consumes this table and binds
encoded-document evidence to canonical decoded bytes. Usable V2U16 package
loading is implemented in Stage 2C.4 and consumes canonical artifact bytes;
command-level V3 package selection and file I/O remain pending.

## 4. Core type design

```text
TokenId      = u32
PairKey      = u64
MAX_TOKEN_ID = 131071
MAX_VOCAB    = 131072 entries
BASE_VOCAB   = 276 entries
```

Pair packing is owned by the pair component:

```text
pack(a,b) = ((a as u64) << 32) | (b as u64)
unpack(k) = ((k >> 32) as u32, k as u32)
```

Operational ID validation is separate from collision-free full-u32 packing.
All allocations and conversions are checked.

State: `TYPED_CORE_PAIR_SPECIAL_POLICY_PAIRED_LOAD_PACKAGE_PUBLICATION_AND_BOUNDED_CODEC_COMMITTED_EVIDENCE_VERIFIED`;
command/file integration and demo acceptance remain pending.

## 5. Artifact architecture

### 5.1 Explicit selection

The caller selects `V2U16` or `V3U32`. There is no auto mode or fallback.

### 5.2 V2 compatibility

The inherited v2 grammar is read-only and widened into the u32 model. Individual
inspection and paired loading are implemented against the tracked canonical
public-v2 32,768-entry artifacts. Known ID/byte anchors and every reconstructed
merge/lookup value pass locally. Legacy aliases come from locked code/fixtures,
not artifact inference.

### 5.3 V3 native contract

The byte-identical approved contract is in
`docs/ARTIFACT_FORMAT_V3_U32.md`: 128-byte header, version/kind/endian/width,
counts, payload digest, shared sequence digest, strict record/semantic checks.

### 5.4 Stage 2C.2 implementation boundary

`src/artifact.rs` implements the explicit format/kind/limits/metadata/error
types and exact locked-header encoding/parsing. Parsing validates all fixed
fields and checked count/file-length relations before any payload allocation.
Header construction calculates SHA-256 from bounded payload bytes; merge headers
require `payload_sha256 == sequence_sha256`.

At Stage 2C.2 closure this foundation was not a file reader. Individual reading
is implemented in Stage 2C.3 and tokenizer construction in Stage 2C.4 below;
publication remains a separate validation-first slice.

### 5.5 Stage 2C.3 individual readers

`inspect_artifact` opens paths read-only and dispatches only on the caller's
explicit `ArtifactFormat` and `ArtifactKind`. V3 processing enforces fixed-header
and exact-length relations, hashes before semantics, then hashes the exact bytes
consumed during a successful semantic pass so a between-pass payload change
cannot produce trusted metadata. Vocab parsing proves declared lengths fit the
remaining payload before fallible allocation, enforces caller limits, and checks
the canonical base. Merge parsing uses `TokenId`/`PairKey` and validates bounds,
sequential results, operand ordering, uniqueness, and exact exhaustion.

V2 processing may recognize V3 magic only to reject wrong selection; it never
switches readers and exposes no writer. Its u16 merge fields widen exactly into
`TokenId`. Because V2 embeds no digests, metadata uses `None` for both digest
fields; `header_bytes = 0` and `payload_bytes = file_bytes` describe the complete
headerless legacy grammar including its count prefix.

Individual inspection deliberately does not publish token tables. Paired
identity, V3 shared-sequence equality, learned-token reconstruction, immutable
tokenizer construction, V2 decoded-byte parity, and mixed-package rejection are
the next validation boundary.

State: `INDIVIDUAL_READERS_IMPLEMENTED_AND_32_TESTS_PASS / SUPERSEDED_BY_PAIRED_LOAD_STAGE_2C4`.

### 5.6 Stage 2C.4 paired loader and immutable tokenizer

`load_tokenizer_package` opens both artifacts read-only before parsing and uses
the caller's one explicit format for both. Each file produces private validated
records plus metadata. Pair validation compares base/vocab/merge/record counts;
V3 also requires vocab and merge `sequence_sha256` equality and merge payload-
sequence equality. Exact supported V3 version equality is implicit because each
individual parser accepts only version 3.0.

Reconstruction visits merges in order and compares the learned token bytes with
the exact left/right operand slices using checked length arithmetic and no
concatenation allocation. Only then is a fallibly preallocated `PairKey ->
TokenId` lookup built. `Tokenizer` owns private vocab/merge/lookup containers and
exposes read-only accessors, so validation failure cannot publish partial state.

V2 has no embedded sequence identity. Its package binding is therefore strict
count agreement plus complete reconstruction. The canonical public-v2 32,768
package passes known ID/byte anchors and exhaustive merge-lookup equality under
explicit V2U16 selection. This is compatibility evidence, not alias-spelling
round-trip preservation.

State: `PAIRED_LOAD_COMMITTED_AND_IMMUTABLE_EVIDENCE_VERIFIED` at commit
`a96a8826d0b6273ee938e3d787a5359f2c7e63c5`; evidence SHA-256
`AB21F89A9B939A52ED5D8AD3ADD5055019BBFAE26053B77CFB93921EB17AA5D3`.

### 5.7 Publication

Native package publication uses create-new same-filesystem staging,
flush/sync, strict readback, dual external hashes, immutable rename, and no
overwrite. Windows failure-injection evidence passes; directory sync remains
unsupported, so crash/power-loss durability is not claimed.

The approved `PublicationContext` has private validated state and admits only
bounded deterministic IDs/labels, nonzero fixed digests, valid Git object IDs,
and explicit backend/evidence identity. It cannot source authority from paths,
environment, timestamps, machine/process identity, or zero/sentinel values.

`ArtifactPackageManifestV1` owns a private Serde wire representation. Schema,
format/version/header, provenance, counts, two fixed-order artifact records,
and successful readback assertions are validated before publication-facing use.
Canonical JSON is compact UTF-8 plus one LF with uppercase hashes and a 32 KiB
pre-parse cap. Parse reserializes and byte-compares, so unknown fields, alternate
whitespace/order/case, and semantically inconsistent records fail closed.
An independent ordered PowerShell generator matches the Rust serializer's
2,245-byte bounded vector at SHA-256
`1C0B117DA9B0BB1EAD64392F807B0FDBEAD11002B2628BB80AD83BBFA9C95E99`.

Exactly pinned `serde`/`serde_json` and nine new transitive packages are recorded
in the dependency matrix. No map-order feature is enabled. Release attribution
remains pending, including Unicode-3.0 notice handling.

The package state machine validates destination ancestry, rejects Windows
reparse points, creates a bounded unique same-parent staging directory, streams
create-new artifacts, performs strict paired readback and complete-file rehash,
writes and rereads the canonical manifest last, requests supported directory
sync, and renames without replacement on Windows. Eight pre-rename phase hooks,
staged corruption, existing-final, rename-race, and two-path deterministic
publication are bounded-test-only controls. Failed staging remains discoverable;
the implementation contains no cleanup path.

`PublishedPackage` returns fixed relative paths, counts, artifact evidence,
manifest SHA-256/SHA-512, and explicit sync capability. Windows directory sync
is reported unsupported; crash/power-loss safety is not claimed. Non-Windows
publication fails closed before rename because `std::fs::rename` cannot prove
no-replace directory semantics on those platforms.

Independent test code constructs the complete V3 wire from literal header fields
and canonical base bytes without calling production header/writer code. Empty,
locked one-merge, and 65,537-vocab/cross-u16 inputs are strictly loaded and then
republished byte-for-byte. The final cross-u16 merge result is ID 65,536. A live
Windows directory symlink carrying the reparse attribute is rejected before any
staging directory appears.

The formal visibility claim is limited to: every pre-rename failure leaves no
final directory; after successful rename the package is visible, and no rollback
or renewed invisibility is claimed without prohibited deletion.

Exact commit `0bd850f87e08ccce3b3b7840dd1e8d86098ff7d9` and immutable
evidence `governance/SOL_STAGE2C5_V3_PACKAGE_PUBLICATION_20260719.json` at
SHA-256 `B36125D2F8CED19CCA50BD9B24A1653440846993812E14757126A8EBB6B14F65`
bind the 48-test, exact-file, traceability, and independent-vector closure.

State: `PACKAGE_PUBLICATION_AND_BOUNDARY_FIXTURES_COMMITTED_EVIDENCE_VERIFIED_WINDOWS_STAGE_2C5`;
portable no-replace rename, Windows crash/power-loss durability, CLI, and codec
remain pending. Bounded trainer freeze and manifest admission are separately
committed/evidenced in the next section and do not alter the Stage 2C.5 claim.

## 6. Manifest/admission architecture

Input authority is an explicit manifest file, never filename patterns. Two
non-interchangeable profiles share the same filesystem and hash validation
core: `CompositeFourFamily` for production-directed composite work and
`DemoWikitextOnly` for the bounded public-pipeline demonstration.

`src/manifest.rs` implements this bounded validation order:

1. read a bounded regular manifest file and require canonical compact JSON plus
   one terminal LF under `ashira_v3_composite_corpus_manifest_v1`;
2. validate sequential ordinals, exact root-set authority, explicit entry
   fields, and nonzero enabled-byte coverage of all four families;
3. validate UTF-8 forward-slash relative paths and deterministic ordinal/path
   ordering;
4. reject NUL, empty/dot/dot-dot, absolute, drive, UNC, symlink, junction,
   and Windows reparse paths;
5. inventory every supplied root, require exact declared/inventoried file-set
   equality, and prove canonical containment;
6. verify size, SHA-256, SHA-512, and optional UTF-8 policy while rechecking
   metadata before/opened/after reads;
7. expose only ordered enabled `AdmittedFile` handles, then repeat verification
   immediately before trainer ingestion.

`admit_corpus_manifest` accepts only
`ashira_v3_composite_corpus_manifest_v1`, forbids a demo label, and retains the
nonzero four-family gate. `admit_demo_wikitext_manifest` accepts only
`ashira_v3_demo_wikitext_manifest_v1`, exact label `demo_wikitext_only`, and
exactly one enabled, nonzero WikiText entry. `AdmittedCorpus::profile` carries
the typed admission result downstream. Cross-profile calls fail on schema
before corpus authority can escape; no boolean bypass weakens composite
validation.

`AdmittedCorpus::training_files` binds explicit positive family weights to
those handles. `TokenizerTrainer::train_weighted` rejects retained legacy
scanner output before file reads, leaving the predecessor scanner available for
audit but unable to authorize v3 training. `TokenizerTrainer::freeze` is
consuming and all-or-nothing: it first proves trainer lookup/merge agreement,
then the shared artifact constructor validates the canonical base vocabulary,
sequential results, forward references, duplicate pairs, and learned-token
reconstruction before private immutable state is returned.

Bounded tests cover admit/train/freeze, deterministic repeat, rejection of
legacy scanner authority, canonical/path/root/nonzero-family/file-set/hash/
encoding failures, post-admission content mutation, live Windows reparse input,
and malformed freeze state. The committed suite is 55/55 with denied-warning
Clippy.

On Unix, object identity uses device/inode. Stable Rust 1.94 does not expose
Windows volume serial/file index; the safe Windows implementation therefore
uses canonical path plus stable metadata/content rechecks. This rejects
reparse traversal and detected mutation but does not prove hardlink alias
uniqueness. Unsafe Win32 FFI and a new dependency were rejected for this
credit-emergency unit. Mount-point portability, true Windows object identity,
full composite admission, and corpus-scale behavior remain open.

Exact commit `8322363b865155183116fa5440fe29a028de91d8` and immutable
evidence `governance/SOL_STAGE3A_BOUNDED_MANIFEST_FREEZE_20260719.json` at
SHA-256 `B5F36F260DE91CC19BC19B16767CFAAFE2D68D6C45CB45FAF828F9C3EF8A6D53`
bind the six source paths, 55/55 tests, 38/38 traceability, and explicit
platform/scale residuals.

The local single-family addition and bundled manifest pass three focused tests,
the complete 85/85 suite, formatting, and denied-warning Clippy. They are not
yet committed or evidenced. The original Stage 3A composite implementation and
its immutable evidence remain unchanged.

State: `COMPOSITE_STAGE3A_COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED /
DEMO_WIKITEXT_ONLY_PROFILE_LOCAL_85_OF_85 / WINDOWS_HARDLINK_IDENTITY_AND_FULL_SCALE_OPEN`.

## 7. Pre-segmentation and calibration

`src/presegment.rs` owns the versioned byte-exact semantic core. The trainer now
calls this module; its inherited private trim/split implementation is removed.
Future calibration and probes must call the same module rather than
implement equivalent-looking logic.

The bounded visitor consumes an admitted byte slice without UTF-8 decoding,
splits LF logical lines, removes exactly one terminal CR, applies the locked
alias sequence predicate to whole-line skipping, and emits maximal leading
ASCII-whitespace-plus-word segments or trailing standalone whitespace. The
whitespace set is an explicit six-byte match. Checked statistics record lines,
skip classes, bytes, segments, and initial pair opportunities. The trainer also
uses checked aggregation for those statistics and weighted word frequencies.

Seven exact fixture groups cover the required line endings, whitespace bytes,
special-only distinctions, non-UTF-8 bytes, long bounded segment, and callback
failure. The committed suite passes 62/62 with Clippy warnings denied.

This module does not yet implement bounded chunk streaming or spill. The
trainer still receives a fully verified bounded byte vector from manifest
admission and retains corpus-dependent maps. Therefore this is the shared
bounded-demo semantic path, not corpus-scale memory evidence.

Calibration computes initial byte-pair opportunities:

```text
P_f = sum_t max(len_bytes(t) - 1, 0)
```

Adaptive integer weights target basis points `3125/1875/2500/2500` within
`0.01` percentage point. No floating-point value may decide pair priority.

Exact commit `f70464f075bd847cb511430971251832612c3020` and immutable
evidence `governance/SOL_STAGE3B2_BOUNDED_PRESEGMENTER_20260719.json` at
SHA-256 `F6D68AB1782F5BA5E0026CC998D1089CC6CE32BA7049146BF9D01D8523C3A447`
bind the exact five-path, 62-test, fixture, toolchain, and residual record.

State: `BOUNDED_PRESEGMENT_CORE_TRAINER_INTEGRATION_COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED_STAGE3B2_WITH_LOCAL_LOSSLESS_CODEC_VIEW / CALIBRATION_PROBES_STREAMING_SPILL_PENDING`;
production calibration execution cannot begin until its checked integer layer
uses this module and the streaming/store prerequisites pass.

### 7.1 Bounded codec core

`src/codec.rs` operates only on the private, validated immutable `Tokenizer`.
Ordinary segment bytes start as locked base-byte IDs. Available adjacent pairs
enter a binary heap ordered by lowest learned merge ID (the immutable merge
rank), then lowest left position. A linked adjacency vector invalidates stale
candidates without interior vector removal. Special aliases split ordinary
merge spans and become locked shared IDs; no merge crosses an alias or
structural CR/LF boundary.

The codec calls `visit_lossless_presegments`, which shares the production
line-segment engine while emitting training-excluded terminal CR and LF as
literal bytes. This distinction is required: applying training's empty/special-
only/line-ending deletion to user text would contradict D4 lossless demo
requirements. Alias canonicalization remains the sole explicit non-reversible
case.

`EncodedTokensV1` has canonical ordered wire fields:

```text
schema, token_id_width, tokenizer{vocab_size,merge_count,sequence_sha256},
decoded_bytes, decoded_sha256, token_count, token_ids
```

The tokenizer binding uses the exact little-endian 12-byte-per-merge sequence
digest already shared by V3 artifacts. Validated base bytes plus ordered merge
records determine reconstructed vocabulary bytes, so no unordered lookup state
enters the binding. The decoded length/hash attest canonical decoder output;
they deliberately do not misrepresent discarded non-canonical alias spelling.
Serialization is compact plus one LF, uppercase-hash, non-cloning, and stopped
by a caller-supplied byte limit. Parsing uses Serde with unknown fields denied,
checks schema/width/counts/uppercase digests/token ranges and caller limits,
then decode additionally requires exact tokenizer binding and output evidence.

The committed Stage 3B3 codec exposes no path-taking API and performs no
filesystem write. The current D4 integration candidate adds one constrained
path seam in `publication`: `load_v3_tokenizer_package` resolves only an
existing package directory or its exact `package_manifest.json`. It rejects
direct artifact/headerless selection and link-like ancestry, strictly parses
the canonical manifest, bounds file size before complete-file hashing, verifies
manifest SHA-256/SHA-512 before and after an explicit `V3U32` paired load,
compares loaded counts, and rereads the unchanged manifest before returning the
immutable tokenizer. This deliberately reuses the manifest's private wire
rather than exposing mutable/unvalidated artifact evidence to CLI code.

The D4 command/file candidate now reads input as bounded bytes and uses a
create-new/atomic Windows output path; it does not introduce a headerless binary
encoded-corpus format. The combined resolver/command candidate passes 82/82 but
is not yet committed or evidenced.

Exact commit `70949771f356c8ef9a622fb7f0da12f95415f605` and immutable
evidence `governance/SOL_STAGE3B3_BOUNDED_CODEC_20260719.json` at SHA-256
`BB96E00A5392B26B83637C7FD949C5D2C96BF2DE4AF082DEE4B3EA61720E82EF`
bind the six-path implementation, 71/71 postcommit gate, 40/40 traceability,
and exact JSON/sequence/cross-u16 vectors.

State: `COMMITTED_IMMUTABLE_EVIDENCE_VERIFIED_71_OF_71_STAGE3B3 / D4_PACKAGE_CLI_FILE_LOCAL_82_OF_82 / DEMO_ACCEPTANCE_AND_EVIDENCE_PENDING`.

### 7.2 D4 explicit CLI and bounded file publication

`Cargo.toml` names one `ashira` binary. `main` is a thin dispatcher into
`cli::run_cli`; the inherited scanner/trainer entrypoint remains textually
available behind `#[cfg(any())]` item guards but is unbuildable. The parser operates on
`OsString` path values and accepts only these exact command option sets:

```text
encode: --package, --text-file, --out
decode: --package, --encoded, --out
```

All three options are required exactly once. Parsing and output-destination
preflight precede package/input work. `decode` bounds and structurally parses
JSON before loading the package; `encode` loads the manifest-bound V3 package,
then performs a bounded byte read. Both converge on the committed/evidenced
`EncodedTokensV1` codec rather than a second encoding implementation.

The CLI's fixed Build Week policy is:

```text
artifact file=512 MiB, total vocab bytes=256 MiB, token bytes=16 MiB,
input=8 MiB, token count=8 Mi IDs, decoded=32 MiB, encoded JSON=128 MiB
```

`read_bounded_regular_file` checks every existing ancestor for link/reparse
state, canonicalizes and rechecks, requires a regular file, enforces metadata
size before reservation, reads through `limit + 1`, compares open-handle
before/after length with actual bytes, and re-resolves the path. This catches
ordinary mutation but does not prove Windows file identity against adversarial
same-length swap-and-restore; that limitation is explicit.

Output uses a validated existing same-parent directory and rejects every
preexisting final object before package/input work. `publish_output_atomic`
tries at most 1,024 process/ordinal create-new staging names, writes through a
buffer, flushes and `sync_all`s, exact-rereads staging, rechecks final absence,
performs Windows non-replacing rename, and exact-rereads final. A late final
preserves both its owner bytes and staging candidate bytes. Non-Windows fails
closed because standard-library rename cannot prove no replacement. Pre-rename
failure exposes no final; successful rename makes final visible, with no
rollback claim. Windows directory durability across crash/power loss is not
claimed.

Success emits one deterministic-content summary with action, byte/token counts,
and uppercase output SHA-256. Paths and timestamps do not enter it. Exit classes
are usage 2, input/package 3, codec 4, and output/publication 5.

### 7.3 Deterministic WikiText demo member

The first demo corpus member is `demo/corpus/wikitext.txt`, created from only
the two operator-supplied WikiText-labelled train Parquet shards. The governed
extractor hashes all four supplied files before and after work, validates their
single UTF-8 `text` column and Snappy structure, scans all 1,801,350 train rows,
and rejects null, non-string, or nonempty non-LF-terminated values.

Eligible nonempty rows are ranked by a domain-separated SHA-256 over shard
filename, little-endian row index, and exact UTF-8 bytes. Selection takes the
hash-order prefix that fits within 1,048,576 bytes without gap filling; emission
then returns those rows to `(shard ordinal, row index)` order. The result is
2,187 rows / 1,047,897 bytes, SHA-256
`71A024D25F902E2F5911E475A68DB9749D58AA560A3A37A5BAE1A0FE20867454`.
An independently implemented verifier rehashes all inputs, rescans both train
shards, reconstructs the selection and output, and matches both the output and
selection-sequence digests recorded in
`governance/SOL_DEMO_WIKITEXT_EXTRACTION_20260719.json`.

This artifact does not establish external dataset revision or license because
those identities are absent from the embedded Parquet metadata. Validation and
test shards are reserved for later probes/evaluation. Under the operator-relayed
GPT-55 ruling, `demo/demo_wikitext_manifest.json` is admitted only through the
typed `DemoWikitextOnly` profile for the local Build Week pipeline. Its compact
canonical 453 bytes have SHA-256
`6879671A146F8553A9A27DD242046A00020D655DC087D3066AE075A9225A5E7A`.
It does not close production composite, balanced coverage, final-v3 authority,
or the other three family gates. Attribution/license remains a public
submission-packaging prerequisite, not a local execution prerequisite.

### 7.4 Single-run demo orchestration candidate

`src/demo.rs` implements the exact command path:

```text
demo-pipeline --manifest <demo_manifest.json> --run-root <new_run_root>
              --vocab-size <276..4096>
```

The bounded maximum 4096 prevents this Build Week command from silently
becoming a full 131,072-entry training surface. The final run root must be absent,
its parent must already exist, and it must be outside the Git source tree. This
allows clean A/B roots without making the second run invalidate source
authority.

Before corpus work, the runtime requires a clean Git worktree/index and derives
the exact commit, tree, Rust toolchain string, and an
`ashira_v3_source_tracked_manifest_v1` digest. The tracked digest sorts normalized
UTF-8 Git paths ordinally and hashes records of lowercase file SHA-256, decimal
byte count, and path. File/count/aggregate bounds and link/reparse rejection are
enforced. The same authority is recomputed after training and must be equal
before publication. This identifies the clean source state; it does not prove a
reproducible binary-to-source build by itself, so the runbook requires `cargo
run --locked --offline` from that clean commit.

The execution sequence is:

1. explicit `DemoWikitextOnly` admission and post-admission manifest hash check;
2. canonical deterministic configuration plus honest calibration/probe
   `not_applicable` assertion documents;
3. fixed uniform weight 1000, minimum frequency 2, deterministic CPU training;
4. consuming trainer freeze into the immutable tokenizer;
5. V3 package publication inside an invisible run staging directory;
6. manifest-bound package load and equality with the frozen tokenizer;
7. fixed 66-byte demo input encode, canonical JSON reparse, decode, exact-byte
   comparison, and synced readback;
8. canonical `demo_run_manifest.json` written last within staging with status
   `PREPUBLICATION_VALIDATED`;
9. Windows same-parent non-replacing run-directory rename;
10. final report and every nonpackage file reread plus final package reload;
11. create-new synced `demo_final_validation.json` with `PASS` only after step 10.

The deterministic-core digest domain-binds the run manifest and final validation
marker. It excludes staging names, paths, PID, timestamps, durations, and
throughput. Trainer console progress remains run-instance telemetry and is not
part of deterministic-core bytes. Two clean test roots match every deterministic
file and hash. An existing operator root is preserved, non-Windows run rename
fails closed, directory sync is unclaimed, and post-rename visibility is never
represented as rollback-capable.

The pipeline source is committed and immutably evidenced. Two real WikiText-only
vocabulary-512 runs from that clean source commit match all 11 governed files,
the 13,927-byte total, run-tree SHA-256 `A15E6B2B...E40A2C`, and
deterministic-core SHA-256 `9EB041C6...39C0B`.

### 7.5 Read-only judge-facing A/B comparator

`src/demo.rs::compare_demo_runs` and the exact CLI form below implement the
committed comparator:

```text
demo-compare --run-a <run_a_root> --run-b <run_b_root>
```

The two inputs must resolve to distinct non-link directories. Root inventory is
fixed to eight governed files plus one `package` directory containing exactly
three governed files; missing, extra, non-UTF-8, nonregular, or link/reparse
entries fail closed. Each file has a type-specific read bound and the complete
run has a checked 2 GiB ceiling.

For each run the comparator performs these read-only checks:

1. read and hash the exact ordinal 11-file set;
2. strictly parse and canonical-reserialize the run manifest, deterministic
   config, two not-applicable gate assertions, and final validation marker;
3. manifest-bound load the V3 package and independently inspect both V3U32
   artifact headers, payload digests, sequence digests, counts, and complete-file
   SHA-256/SHA-512 evidence;
4. verify package provenance binds the run configuration, corpus, assertions,
   source commit/tree/tracked aggregate, writer, toolchain, and backend;
5. canonical-reparse the u32 encoded document, decode with the validated
   tokenizer, and require the fixed input and decoded file to match byte-for-byte;
6. recompute final-attestation references, deterministic-core framing, and the
   ordinal run-tree SHA-256 framing used by the independent A/B receipt;
7. compare every corresponding complete file and the semantic/aggregate result.

Only after all checks pass does `src/cli.rs` emit one `PASS demo-compare ...`
line. The comparator creates no file and does not train, freeze, publish, delete,
or reuse a run root. It requires equal embedded run source authority but does
not compare that older authority with the comparator binary's current HEAD;
otherwise the later comparator commit would make valid earlier runs impossible
to inspect. Comparator source authority is separately bound by commit
`80055ecbbb8e46d51edd9d9a54098041778f2f9f`, tree
`1b26e94229964edf702a7bf72b90f41f2c60da23`, and immutable evidence
`governance/SOL_STAGE3B5_D4_WIKITEXT_AB_COMPARATOR_20260720.json` (15,239 bytes;
SHA-256
`4463C88BE5622E7A52F048AEB025626A567632D83241F22E8252C84A69ED99CA`).
Independent readback revalidated all seven committed file/blob identities,
seven immutable inputs, 91/91 tests, the live 11-file A/B trees, and the exact
judge-facing PASS line.

GPT-55's independent rerun found a test-harness caveat: a repeated full suite in
the default temporary directory can collide with fixture directories retained
by an earlier process, while an isolated fresh temporary root passes 91/91.
Inspection confirms test-only helpers in `cli`, `demo`, and `publication` use
process ID plus process-local ordinal names followed by `create_dir`, so PID
reuse can meet retained paths. The production comparator does not use these
helpers. A later harness change must provide collision-resistant bounded
create-new roots or explicit per-run temp isolation without deleting retained
evidence; that fix is not part of this documentation reconciliation.

## 8. `StdShardStoreR1`

Stage 3 provides a deterministic in-repo skeleton using standard-library
filesystem and buffered I/O only:

- deterministic shard keys/names;
- append journal with generation/transaction identity;
- committed shard snapshots;
- temp-write, flush, supported sync, and rename;
- single deterministic ordering path;
- no DB, mmap, network, or ordering-changing background thread;
- no timestamp in deterministic-core bytes.

Only bounded fixtures are required. Full BookCorpus throughput is not claimed.

State: `PENDING_STAGE_3`; scale readiness `BLOCKED_BY_PROBES`.

## 9. Determinism and failure design

- Inputs and reports use ordinal normalized relative-path ordering.
- Pair priority is highest checked count then smallest packed key.
- Hash/random iteration order never enters deterministic serialization.
- Wrong format, malformed bytes, path escape, count overflow, stale state, and
  publication failure return typed errors; no warning-and-continue behavior.
- Timestamps/machine/absolute paths remain outside deterministic-core bytes.

State: `DESIGNED / TESTS_PENDING`.

## 10. Security and resource design

- Validate sizes/counts before allocation.
- Enforce explicit artifact/resource limits.
- Reject reparse/link traversal before content admission.
- Stream/spill corpus-dependent state; do not retain unbounded global maps.
- Use a separate Cargo target directory so inherited tracked `target/` evidence
  is not overwritten.

State: `PARTIAL_ARTIFACT_PUBLICATION_AND_BOUNDED_MANIFEST_ADMISSION_IMPLEMENTED_TESTED`;
production Windows object identity, bounded corpus-dependent trainer state, and
scale evidence remain pending.

## 11. Traceability and change control

Requirement mappings reside in `docs/REQUIREMENTS_TRACEABILITY_MATRIX.csv`.
Standards mappings reside in `docs/ISO_IEEE_EVIDENCE_MATRIX.csv`.
Dependency policy resides in `docs/DEPENDENCY_LICENSE_MATRIX.csv`.

Any required V3U32 header change stops implementation and creates
`V3U32_HEADER_CHANGE_REQUEST_<timestamp>.md`. No silent binary change is
permitted.

## 12. Current design status

| Area | Implemented | Tested | Pending | Blocked |
|---|---|---|---|---|
| Source lineage | yes | yes | no | no |
| Design/traceability baseline | yes | bounded readback yes | continuing reconciliation | no |
| Typed core/special policy/artifacts/codec | typed core, pair, special/alias policy, artifact types/header, strict readers, paired validation, immutable tokenizer construction, publication context, canonical manifest, Windows package writer, consuming trainer freeze, bounded codec/JSON, manifest-bound CLI, atomic command output, demo orchestration, and read-only A/B comparator | core, demo pipeline, and comparator committed/evidenced; real WikiText v512 A/B 11/11 parity; comparator fresh-root 91/91 and live acceptance pass | retained-default-temp test-fixture rerunability fix and video execution | binary-to-source reproducibility, handle-bound identity, non-Windows no-replace, and crash/power-loss durability unresolved |
| Manifest/pre-segment/calibration/store | bounded composite admission plus separately typed single-WikiText demo admission and shared bounded trainer/lossless-codec pre-segment core | composite committed/evidenced Stage 3A; demo profile/bundled manifest local 3 focused and 85/85 complete; WikiText bytes independently reconstructed | calibration, probes, streaming/spill, store, full production composite manifest | external WikiText attribution/license before public packaging, Windows hardlink identity, and full corpus/probes |
| Trainer/checkpoints/full training | inherited weighted BPE plus admitted-input/freeze bridge | bounded deterministic one-merge same-path and malformed-freeze fixtures committed/evidenced Stage 3A | bounded-state trainer redesign, checkpoints, full training | scale readiness |
| Release/submission claims | no | no | official-source review | yes |
