# AshiraTokenizer V3U32 Binary Artifact Format

**Document status:** `DESIGN DRAFT — NOT IMPLEMENTED — GPT-55 APPROVAL REQUIRED`  
**Format name:** `V3U32`  
**Proposed format version:** `3.0`  
**Header size:** 128 bytes  
**Byte order:** little-endian for every multi-byte integer  
**Native files:** `vocab.bin`, `merges.bin`  
**Compatibility mode:** explicit `V2U16` only; never auto-detected  
**Construction date:** 2026-07-18  
**Authority:** `SPIKE-ASHIRA-V3-PREIMPLEMENTATION-DESIGN-20260624`  

## 1. Purpose and lineage

This is the proposed native binary contract for AshiraTokenizer v3. It is an
evolution of the public AshiraTokenizer v2 implementation at commit
`f4ad48ff57db2b080d55e8b3a6fb6c71bca0d5c3`; it is not a greenfield tokenizer
or an unrelated artifact family.

V3 retains the v2 byte-vocabulary and BPE merge semantics while correcting the
legacy artifact limitations:

- headerless, unversioned files;
- `u16` merge IDs;
- no explicit endianness or ID width;
- no embedded payload integrity value;
- no binding between the vocabulary and its merge sequence;
- no strict format-selection API.

The existing v2 bytes remain readable only through the explicit V2U16
compatibility path specified in section 11. Native v3 code writes V3U32 only.

## 2. Normative language and claim boundary

`MUST`, `MUST NOT`, `SHOULD`, and `MAY` are design requirements inside this
draft. They do not claim implementation or test completion.

This draft does not authorize source import, implementation, building, testing,
corpus hashing, probes, checkpoint generation, or training. It becomes a
normative implementation contract only after GPT-55 approval.

## 3. Design correction from the first report

The preimplementation report initially proposed a 96-byte header containing a
payload digest. That layout authenticated each payload only through external
package hashes and did not directly bind `vocab.bin` to the merge sequence it
represents.

This draft supersedes that proposal with a 128-byte header. Bytes 96-127 contain
`sequence_sha256`, defined as SHA-256 of the canonical V3U32 merge payload. Both
files carry the same value. In `merges.bin`, `sequence_sha256` therefore equals
`payload_sha256`; in `vocab.bin`, it binds the vocabulary to its companion merge
sequence.

The refinement does not replace external complete-file SHA-256/SHA-512 or
semantic paired-file validation.

## 4. Canonical primitive types

| Name | Width | Meaning |
|---|---:|---|
| `u8` | 1 byte | unsigned integer |
| `u16le` | 2 bytes | unsigned little-endian integer |
| `u32le` | 4 bytes | unsigned little-endian integer |
| `u64le` | 8 bytes | unsigned little-endian integer |
| `TokenId` | 4 bytes | `u32le`; operationally `0..=131071` |
| SHA-256 | 32 bytes | raw digest bytes, not hexadecimal text |

All offsets and sizes in this document are decimal unless prefixed by `0x`.
There is no implicit alignment or padding in either payload.

## 5. File grammar

```text
v3_file       = common_header payload EOF
common_header = exactly 128 bytes

vocab_payload = vocab_record repeated record_count times
vocab_record  = token_byte_length:u32le token_bytes:[u8; token_byte_length]

merge_payload = merge_record repeated record_count times
merge_record  = a:TokenId b:TokenId merged:TokenId
```

The header is the only V3U32 count prefix. V3U32 payloads MUST NOT repeat the
legacy leading `u32` count.

## 6. Common 128-byte header

### 6.1 Byte layout

| Offset | End | Size | Field | Required value or meaning |
|---:|---:|---:|---|---|
| 0 | 7 | 8 | `magic` | raw bytes `41 53 48 49 52 41 33 00`, ASCII `ASHIRA3` plus NUL |
| 8 | 9 | 2 | `format_major` | `3` |
| 10 | 11 | 2 | `format_minor` | `0` |
| 12 | 12 | 1 | `artifact_kind` | `1` vocab; `2` merges |
| 13 | 13 | 1 | `endianness` | `1` means little-endian |
| 14 | 14 | 1 | `token_id_bytes` | `4` |
| 15 | 15 | 1 | `fixed_record_bytes` | `0` vocab; `12` merges |
| 16 | 17 | 2 | `header_bytes` | `128` (`80 00`) |
| 18 | 19 | 2 | `flags` | `0`; every unknown bit is rejected |
| 20 | 23 | 4 | `reserved_0` | all zero |
| 24 | 31 | 8 | `record_count` | vocab or merge record count |
| 32 | 39 | 8 | `payload_bytes` | exact number of bytes after the header |
| 40 | 43 | 4 | `base_vocab_count` | `276` |
| 44 | 47 | 4 | `vocab_size` | represented entries, `276..=131072` |
| 48 | 55 | 8 | `merge_count` | represented learned merges |
| 56 | 63 | 8 | `reserved_1` | all zero |
| 64 | 95 | 32 | `payload_sha256` | SHA-256 of bytes 128 through EOF |
| 96 | 127 | 32 | `sequence_sha256` | SHA-256 of the companion merge payload |

The field sizes sum to exactly 128 bytes.

### 6.2 Header invariants common to both kinds

A reader MUST validate these conditions before payload-driven allocation:

1. caller selected `ArtifactFormat::V3U32`;
2. the file contains at least 128 bytes;
3. magic, supported major/minor, kind, endianness, ID width, record width, and
   header size match exactly;
4. flags and reserved bytes are zero;
5. `base_vocab_count == 276`;
6. `276 <= vocab_size <= 131072`;
7. `merge_count == vocab_size - 276` using checked arithmetic;
8. `header_bytes + payload_bytes` is checked and equals exact file length;
9. SHA-256 of the payload equals `payload_sha256`;
10. no bytes exist before the header or after the declared payload.

Unknown flags, header extensions, minor versions, and reserved values are
rejected in version 3.0. Silent forward compatibility is forbidden.

## 7. `vocab.bin`

### 7.1 Kind-specific header values

```text
artifact_kind     = 1
fixed_record_bytes = 0
record_count      = vocab_size
merge_count       = vocab_size - 276
payload_sha256    = SHA256(vocab_payload)
sequence_sha256   = SHA256(companion_merge_payload)
```

### 7.2 Record parsing

For every token ID from zero through `record_count - 1`, in ascending order:

1. read `token_byte_length:u32le`;
2. prove the declared length fits inside the remaining payload;
3. consume exactly that many raw bytes as the canonical decoded value for the
   token ID;
4. perform checked cumulative offset arithmetic;
5. after the final record, require exact payload exhaustion.

Raw token bytes are not required to be UTF-8 and are not NUL-terminated.
Zero-length token bytes are structurally permitted only where the approved base
vocabulary contract permits them. Learned IDs `>=276` MUST have nonempty bytes.

### 7.3 Base vocabulary contract

V3 format major 3 retains v2's ID assignments:

- IDs `0..=19`: special/reserved slots;
- IDs `20..=275`: one-byte tokens where token `20 + b` decodes to byte `b`;
- learned IDs begin at `276`.

The proposed canonical special bytes, derived from the verified public-v2 32K
artifact, are:

| ID | Canonical bytes interpreted as UTF-8 | Hex |
|---:|---|---|
| 0 | `<PAD>` | `3C5041443E` |
| 1 | `<UNK>` | `3C554E4B3E` |
| 2 | `<BOS>` | `3C424F533E` |
| 3 | `<EOS>` | `3C454F533E` |
| 4 | `<kareem_narration>` | `3C6B617265656D5F6E6172726174696F6E3E` |
| 5 | `<dylan_thinking>` | `3C64796C616E5F7468696E6B696E673E` |
| 6 | `<DYLAN>` | `3C44594C414E3E` |
| 7 | `<DYLAN_ADVERSARIAL>` | `3C44594C414E5F414456455253415249414C3E` |
| 8 | `<BLU>` | `3C424C553E` |
| 9 | `<ECHO>` | `3C4543484F3E` |
| 10 | `<RESONANCE>` | `3C5245534F4E414E43453E` |
| 11 | `<AI>` | `3C41493E` |
| 12 | `<PHIL>` | `3C5048494C3E` |
| 13 | `<SYM>` | `3C53594D3E` |
| 14 | `<REFLECTION>` | `3C5245464C454354494F4E3E` |
| 15 | `<CAIROS>` | `3C434149524F533E` |
| 16 | `[[/ANCHOR]]` | `5B5B2F414E43484F525D5D` |
| 17 | `[[/CSA]]` | `5B5B2F4353415D5D` |
| 18 | `<science_doc>` | `3C736369656E63655F646F633E` |
| 19 | empty reserved slot | empty |

These canonical bytes are a proposal pending GPT-55 approval. The many accepted
v2 spellings that map to shared IDs are an encoder/API alias contract and are
not recoverable from, or duplicated inside, `vocab.bin`.

## 8. `merges.bin`

### 8.1 Kind-specific header values

```text
artifact_kind      = 2
fixed_record_bytes = 12
record_count       = merge_count
payload_bytes      = checked(record_count * 12)
payload_sha256     = SHA256(merge_payload)
sequence_sha256    = payload_sha256
```

### 8.2 Record invariants

For zero-based merge ordinal `i`:

```text
expected_merged = 276 + i
```

The reader MUST prove:

- `expected_merged <= 131071`;
- `merged == expected_merged`;
- `a < merged` and `b < merged`;
- `a`, `b`, and `merged` are at most `131071`;
- the ordered pair `(a,b)` has not appeared in an earlier record;
- exact payload exhaustion follows the last record.

The pair key used for duplicate detection is exactly:

```text
((a as u64) << 32) | (b as u64)
```

No other pair packing is permitted.

## 9. Paired-package validation

Loading a usable tokenizer requires both artifacts. An individual file can be
structurally inspected, but it is not a complete tokenizer package.

The paired validator MUST prove:

1. both files passed strict individual validation;
2. `format_major`, `format_minor`, `base_vocab_count`, `vocab_size`, and
   `merge_count` agree;
3. both `sequence_sha256` values agree;
4. merge `payload_sha256 == sequence_sha256`;
5. vocabulary `record_count == vocab_size`;
6. merge `record_count == merge_count`;
7. base vocabulary bytes match the approved v3 base contract;
8. for each learned merge in order, vocabulary bytes at `merged` equal the
   exact concatenation of vocabulary bytes at `a` and `b`;
9. all checked resource limits are satisfied.

A count-compatible vocabulary and merge file from different sequences is
therefore rejected by sequence-digest or reconstruction validation.

## 10. Typed API and explicit selection

The semantic API is:

```rust
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactFormat {
    V2U16,
    V3U32,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ArtifactKind {
    Vocab,
    Merges,
}

pub struct ArtifactLimits {
    pub max_file_bytes: u64,
    pub max_total_vocab_bytes: u64,
    pub max_token_bytes: u32,
}

pub fn inspect_artifact(
    path: &Path,
    expected_format: ArtifactFormat,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
) -> Result<ArtifactMetadata, ArtifactError>;

pub fn load_tokenizer_package(
    vocab_path: &Path,
    merges_path: &Path,
    expected_format: ArtifactFormat,
    limits: &ArtifactLimits,
) -> Result<Tokenizer, ArtifactError>;

pub fn write_v3_package(
    tokenizer: &Tokenizer,
    destination: &Path,
) -> Result<PublishedPackage, ArtifactError>;
```

There is no `Auto`, `Unknown`, best-effort, extension-based, or fallback format
mode. CLI callers MUST supply an equivalent of:

```text
--artifact-format v2-u16
--artifact-format v3-u32
```

The V2U16 reader MAY check for V3 magic solely to produce a precise
wrong-selection error. It MUST NOT switch readers after that check.

## 11. Explicit V2U16 compatibility

### 11.1 Legacy vocab grammar

```text
legacy_vocab = count:u32le
               (token_byte_length:u32le token_bytes)*count
               EOF
```

### 11.2 Legacy merge grammar

```text
legacy_merges = count:u32le
                (a:u16le b:u16le merged:u16le)*count
                EOF
```

The compatibility reader MUST:

- require explicit `ArtifactFormat::V2U16`;
- widen each legacy ID to `TokenId` without value change;
- reject V3 magic rather than switching formats;
- use checked count/length arithmetic and caller resource limits;
- reject truncation, trailing bytes, duplicate pairs, non-sequential results,
  forward/self references, count/vocab inconsistency, and reconstructed-byte
  mismatch;
- validate canonical public-v2 32K artifacts and their known prefix packages;
- seed the locked public-v2 alias table from code/fixtures rather than infer it
  from canonical vocab slots.

V3 MUST NOT expose a V2U16 writer. Compatibility is read-only.

## 12. Reader processing order

To prevent malformed input from driving unsafe allocation, readers process in
this order:

1. open read-only and obtain exact file size;
2. enforce caller `max_file_bytes`;
3. read the fixed header required by the explicitly selected format;
4. validate all fixed header fields and checked length relations;
5. stream-hash the declared payload and verify its digest;
6. parse records with remaining-byte accounting and per-record limits;
7. validate semantic invariants and cumulative resource limits;
8. for package loading, validate companion identity and reconstruction;
9. publish a tokenizer object only after every check passes.

Failure MUST leave no partially usable tokenizer. Hashing before full semantic
parsing does not make untrusted data safe; all structural and semantic checks
remain mandatory.

## 13. Error taxonomy and failure modes

Implementations MAY use richer internal causes, but must preserve these stable
classes in evidence:

| Error class | Trigger | Required behavior |
|---|---|---|
| `Io` | open/read/metadata failure | fail; preserve OS cause |
| `WrongFormatSelection` | selected reader conflicts with observed contract | fail; never retry another reader |
| `BadMagic` | V3 magic mismatch | fail |
| `UnsupportedVersion` | major/minor not exactly supported | fail |
| `WrongArtifactKind` | vocab/merge kind differs from caller expectation | fail |
| `BadEndianness` | endian marker is not `1` | fail |
| `BadIdWidth` | token width is not `4` | fail |
| `BadHeaderSize` | header size is not `128` | fail |
| `UnsupportedFlags` | flags nonzero | fail |
| `NonZeroReserved` | either reserved region nonzero | fail |
| `CountOutOfRange` | vocab/count/merge relation invalid | fail before allocation |
| `ArithmeticOverflow` | any size, offset, count, or ID calculation overflows | fail |
| `Truncated` | required header/record/token bytes absent | fail |
| `TrailingData` | bytes remain beyond exact grammar | fail |
| `PayloadDigestMismatch` | payload SHA-256 differs | fail |
| `SequenceDigestMismatch` | paired sequence identities differ | fail |
| `DuplicatePair` | ordered pair repeated | fail |
| `NonSequentialResult` | merged ID differs from `276 + ordinal` | fail |
| `ForwardReference` | operand is not earlier than result | fail |
| `InvalidTokenId` | ID exceeds `131071` | fail |
| `BaseContractMismatch` | IDs 0-275 do not match approved base contract | fail |
| `ReconstructedTokenMismatch` | learned bytes differ from operand concatenation | fail |
| `ResourceLimitExceeded` | explicit caller limit would be exceeded | fail without partial object |
| `ExistingDestination` | publication target exists | fail; do not overwrite |
| `DurabilityFailure` | flush/sync/rename/readback fails | fail; final package not accepted |

CLI failure returns nonzero. Library failure returns a typed error. Neither path
logs a warning and continues.

## 14. Native writer and atomic package publication

The writer operates on the pair as one package:

1. validate the complete in-memory vocabulary/merge sequence;
2. serialize the canonical merge payload and calculate `sequence_sha256`;
3. serialize vocabulary and calculate its payload SHA-256;
4. construct both 128-byte headers using the same sequence digest;
5. create unique same-filesystem staging files with create-new semantics;
6. write header and payload, flush, and request durable file sync;
7. strict-read both staged files through `ArtifactFormat::V3U32`;
8. compute complete-file SHA-256 and SHA-512;
9. write the external package manifest last;
10. sync staging directory where supported;
11. rename to a non-existing immutable final directory;
12. sync the parent directory where supported.

Direct overwrite of an existing artifact or checkpoint is forbidden. A failed
publication leaves only a non-authoritative staging path that startup reports or
quarantines. Atomic visibility and durable persistence are separate properties
and require platform failure-injection evidence.

## 15. External package manifest

The external manifest is mandatory even though each artifact embeds SHA-256.
It records at least:

- format name/version and header size;
- relative filename, artifact kind, exact bytes, SHA-256, and SHA-512;
- payload SHA-256 and sequence SHA-256;
- vocab and merge counts;
- checkpoint/run/parent IDs;
- deterministic configuration hash;
- corpus composite-manifest and calibration/probe hashes;
- source commit, tree, and tracked-file manifest digest;
- writer version/toolchain identity;
- readback and prefix-proof evidence IDs.

Absolute paths, timestamps, durations, and machine identity belong to a
separate run-instance provenance object and do not enter deterministic core
artifacts.

## 16. Exact test vectors

### 16.1 Empty merge sequence digest

For `vocab_size=276` and `merge_count=0`, the merge payload is empty and:

```text
SHA256(empty) =
E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855
```

Both files' `sequence_sha256` MUST contain those 32 raw digest bytes.

### 16.2 One-record merge payload

The verified v2 first learned token is byte concatenation `20 74` (`" t"`).
With byte-token IDs `a=52`, `b=136`, and `merged=276`, its V3U32 merge record is:

```text
a       = 52  = 34 00 00 00
b       = 136 = 88 00 00 00
merged  = 276 = 14 01 00 00
payload = 34 00 00 00 88 00 00 00 14 01 00 00
```

Payload bytes: `12`

```text
SHA256(payload) =
53887809DDF78304283754676329289EC235EADA6AE7F5BFB02C97A2FB276FA9
```

For the corresponding one-merge `merges.bin` header:

```text
artifact_kind      = 2
fixed_record_bytes = 12
header_bytes       = 128
record_count       = 1
payload_bytes      = 12
base_vocab_count   = 276
vocab_size         = 277
merge_count        = 1
payload_sha256     = digest above
sequence_sha256    = digest above
```

The complete header/file digest is intentionally not asserted in this draft
until an independent golden-vector generator and parser cross-check are
authorized and recorded.

## 17. Mandatory test matrix

### 17.1 Boundary IDs

- accept and round-trip `65_534`, `65_535`, `65_536`, `65_537`, `131_070`, and
  `131_071`;
- reject token ID `131_072`;
- accept vocabulary size `131_072`;
- reject vocabulary size `131_073`;
- serialize merges crossing the former u16 boundary.

### 17.2 Header and payload corruption

Mutate each header field independently and require the corresponding error.
Test every truncation position around header end, length prefixes, token bytes,
and 12-byte merge records. Append one and multiple trailing bytes. Alter one
payload byte without updating digest. Alter digest only. Swap count-compatible
vocab and merge files. Test duplicate, forward, self, out-of-range, and
non-sequential records.

### 17.3 Format selection

- V2U16 selected for V3U32 input fails without fallback;
- V3U32 selected for canonical v2 input fails without fallback;
- missing format selection is a CLI/API validation error;
- file extension and filename never choose the reader.

### 17.4 Compatibility and determinism

- canonical public-v2 32K artifacts load under V2U16 with unchanged IDs and
  decoded bytes;
- probe290/512/2048 and 16K remain exact logical prefixes of canonical 32K;
- V3 save/load preserves all IDs and bytes;
- repeated bounded runs produce byte-identical deterministic core artifacts;
- every checkpoint's vocab/merges are exact prefixes of later checkpoints in
  the same sequence;
- interrupted writer tests never expose a partial final package.

## 18. Open approval items

Before implementation, GPT-55 must rule on:

1. the 128-byte header and sequence-digest refinement;
2. the proposed canonical bytes for special IDs 0-19;
3. the complete alias table outside the binary artifact;
4. caller/runtime resource-limit defaults;
5. whether complete header/file golden vectors are required in this document or
   may reside in the test-evidence package;
6. the encoded-corpus format, which is outside this tokenizer-artifact contract.

## 19. Stop boundary

Stop and escalate if implementation would:

- auto-detect or fall back between artifact formats;
- emit native v2 artifacts;
- use `u16` for an ID-bearing v3 field;
- accept ID `131072` or vocabulary size `131073`;
- alter the base ID layout without approved migration authority;
- omit sequence binding, external dual hashes, or paired semantic validation;
- allocate from untrusted counts before checked length/resource validation;
- overwrite a published artifact;
- claim atomic durability without failure-injection evidence;
- weaken canonical v2 compatibility fixtures.

No implementation may begin under this draft's authority alone.
