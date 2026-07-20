use crate::BpeMerge;
use crate::pair::{PairKey, pack_pair};
use crate::token::{
    BPE_TOKEN_START, MAX_TOKEN_ID, MAX_VOCAB_SIZE, SPECIAL_TOKEN_COUNT, TokenId, base_byte_token,
    canonical_special_bytes,
};
use sha2::{Digest, Sha256};
use std::collections::{HashMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::File;
use std::io::{self, BufReader, Read, Seek, SeekFrom};
use std::path::Path;

pub const V3_MAGIC: [u8; 8] = *b"ASHIRA3\0";
pub const V3_FORMAT_MAJOR: u16 = 3;
pub const V3_FORMAT_MINOR: u16 = 0;
pub const V3_HEADER_BYTES: usize = 128;

const V3_HEADER_BYTES_U16: u16 = 128;
const V3_HEADER_BYTES_U64: u64 = 128;
const V3_ENDIAN_LITTLE: u8 = 1;
const V3_TOKEN_ID_BYTES: u8 = 4;
const V3_VOCAB_RECORD_BYTES: u8 = 0;
const V3_MERGE_RECORD_BYTES: u8 = 12;
const V3_BASE_VOCAB_COUNT: u32 = BPE_TOKEN_START;
const V2_COUNT_PREFIX_BYTES: u64 = 4;
const V2_MERGE_RECORD_BYTES: u64 = 6;
const V2_MAX_VOCAB_SIZE: u32 = 65_536;
const V2_MAX_MERGE_COUNT: u32 = V2_MAX_VOCAB_SIZE - V3_BASE_VOCAB_COUNT;
const HASH_BUFFER_BYTES: usize = 8192;
const HASH_BUFFER_BYTES_U64: u64 = 8192;
const V3_MAGIC_BYTES_U64: u64 = 8;

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

impl ArtifactKind {
    const fn encoded(self) -> u8 {
        match self {
            Self::Vocab => 1,
            Self::Merges => 2,
        }
    }

    const fn fixed_record_bytes(self) -> u8 {
        match self {
            Self::Vocab => V3_VOCAB_RECORD_BYTES,
            Self::Merges => V3_MERGE_RECORD_BYTES,
        }
    }

    const fn from_encoded(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Vocab),
            2 => Some(Self::Merges),
            _ => None,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactLimits {
    pub max_file_bytes: u64,
    pub max_total_vocab_bytes: u64,
    pub max_token_bytes: u32,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactMetadata {
    pub format: ArtifactFormat,
    pub kind: ArtifactKind,
    pub file_bytes: u64,
    pub header_bytes: u16,
    pub record_count: u64,
    pub payload_bytes: u64,
    pub base_vocab_count: u32,
    pub vocab_size: u32,
    pub merge_count: u64,
    pub payload_sha256: Option<[u8; 32]>,
    pub sequence_sha256: Option<[u8; 32]>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct Tokenizer {
    vocab: Vec<Vec<u8>>,
    merges: Vec<BpeMerge>,
    merge_lookup: HashMap<PairKey, TokenId>,
}

impl Tokenizer {
    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    pub fn merge_count(&self) -> usize {
        self.merges.len()
    }

    pub fn token_bytes(&self, token_id: TokenId) -> Option<&[u8]> {
        let index = usize::try_from(token_id).ok()?;
        self.vocab.get(index).map(Vec::as_slice)
    }

    pub fn merge_at(&self, ordinal: usize) -> Option<&BpeMerge> {
        self.merges.get(ordinal)
    }

    pub fn merged_token(&self, a: TokenId, b: TokenId) -> Option<TokenId> {
        self.merge_lookup.get(&pack_pair(a, b)).copied()
    }

    pub(crate) fn try_from_parts(
        vocab: Vec<Vec<u8>>,
        merges: Vec<BpeMerge>,
    ) -> Result<Self, ArtifactError> {
        validate_tokenizer_parts(&vocab, &merges)?;

        let merge_count_u64 =
            u64::try_from(merges.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "merge lookup length conversion",
            })?;
        let mut merge_lookup = HashMap::new();
        merge_lookup.try_reserve(merges.len()).map_err(|_| {
            ArtifactError::ResourceLimitExceeded {
                resource: "merge_lookup_allocation",
                limit: u64::from(MAX_VOCAB_SIZE),
                actual: merge_count_u64,
            }
        })?;
        for merge in &merges {
            if merge_lookup
                .insert(pack_pair(merge.a, merge.b), merge.merged)
                .is_some()
            {
                return Err(ArtifactError::DuplicatePair);
            }
        }

        Ok(Self {
            vocab,
            merges,
            merge_lookup,
        })
    }
}

enum ArtifactRecords {
    Vocab(Vec<Vec<u8>>),
    Merges(Vec<BpeMerge>),
}

struct ValidatedArtifact {
    metadata: ArtifactMetadata,
    records: ArtifactRecords,
}

#[derive(Clone, Copy)]
enum ReadPurpose {
    Inspect,
    Load,
}

impl ReadPurpose {
    const fn retains_records(self) -> bool {
        matches!(self, Self::Load)
    }
}

#[derive(Debug)]
pub enum ArtifactError {
    Io(io::Error),
    WrongFormatSelection,
    BadMagic,
    UnsupportedVersion {
        major: u16,
        minor: u16,
    },
    WrongArtifactKind {
        expected: ArtifactKind,
        actual: u8,
    },
    BadEndianness {
        actual: u8,
    },
    BadIdWidth {
        actual: u8,
    },
    BadRecordWidth {
        kind: ArtifactKind,
        actual: u8,
    },
    BadHeaderSize {
        actual: u16,
    },
    UnsupportedFlags {
        actual: u16,
    },
    NonZeroReserved,
    CountOutOfRange {
        field: &'static str,
    },
    ArithmeticOverflow {
        operation: &'static str,
    },
    Truncated {
        expected_bytes: u64,
        actual_bytes: u64,
    },
    TrailingData {
        expected_bytes: u64,
        actual_bytes: u64,
    },
    PayloadDigestMismatch,
    SequenceDigestMismatch,
    DuplicatePair,
    NonSequentialResult,
    ForwardReference,
    InvalidTokenId,
    BaseContractMismatch,
    ReconstructedTokenMismatch,
    ResourceLimitExceeded {
        resource: &'static str,
        limit: u64,
        actual: u64,
    },
    InvalidPublicationContext {
        field: &'static str,
    },
    InvalidPackageManifest {
        field: &'static str,
    },
    InvalidPublicationPath {
        field: &'static str,
    },
    ExistingDestination,
    DurabilityFailure {
        operation: &'static str,
        source: io::Error,
    },
}

impl ArtifactError {
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Io(_) => "Io",
            Self::WrongFormatSelection => "WrongFormatSelection",
            Self::BadMagic => "BadMagic",
            Self::UnsupportedVersion { .. } => "UnsupportedVersion",
            Self::WrongArtifactKind { .. } => "WrongArtifactKind",
            Self::BadEndianness { .. } => "BadEndianness",
            Self::BadIdWidth { .. } => "BadIdWidth",
            Self::BadRecordWidth { .. } => "BadRecordWidth",
            Self::BadHeaderSize { .. } => "BadHeaderSize",
            Self::UnsupportedFlags { .. } => "UnsupportedFlags",
            Self::NonZeroReserved => "NonZeroReserved",
            Self::CountOutOfRange { .. } => "CountOutOfRange",
            Self::ArithmeticOverflow { .. } => "ArithmeticOverflow",
            Self::Truncated { .. } => "Truncated",
            Self::TrailingData { .. } => "TrailingData",
            Self::PayloadDigestMismatch => "PayloadDigestMismatch",
            Self::SequenceDigestMismatch => "SequenceDigestMismatch",
            Self::DuplicatePair => "DuplicatePair",
            Self::NonSequentialResult => "NonSequentialResult",
            Self::ForwardReference => "ForwardReference",
            Self::InvalidTokenId => "InvalidTokenId",
            Self::BaseContractMismatch => "BaseContractMismatch",
            Self::ReconstructedTokenMismatch => "ReconstructedTokenMismatch",
            Self::ResourceLimitExceeded { .. } => "ResourceLimitExceeded",
            Self::InvalidPublicationContext { .. } => "InvalidPublicationContext",
            Self::InvalidPackageManifest { .. } => "InvalidPackageManifest",
            Self::InvalidPublicationPath { .. } => "InvalidPublicationPath",
            Self::ExistingDestination => "ExistingDestination",
            Self::DurabilityFailure { .. } => "DurabilityFailure",
        }
    }
}

impl fmt::Display for ArtifactError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(formatter, "artifact I/O failed: {error}"),
            Self::UnsupportedVersion { major, minor } => {
                write!(formatter, "unsupported artifact version {major}.{minor}")
            }
            Self::WrongArtifactKind { expected, actual } => {
                write!(
                    formatter,
                    "wrong artifact kind {actual}; expected {expected:?}"
                )
            }
            Self::BadEndianness { actual } => {
                write!(formatter, "bad artifact endianness marker {actual}")
            }
            Self::BadIdWidth { actual } => write!(formatter, "bad token ID width {actual}"),
            Self::BadRecordWidth { kind, actual } => {
                write!(formatter, "bad {kind:?} record width {actual}")
            }
            Self::BadHeaderSize { actual } => write!(formatter, "bad header size {actual}"),
            Self::UnsupportedFlags { actual } => {
                write!(formatter, "unsupported artifact flags {actual:#06x}")
            }
            Self::CountOutOfRange { field } => {
                write!(formatter, "artifact count relation failed for {field}")
            }
            Self::ArithmeticOverflow { operation } => {
                write!(formatter, "artifact arithmetic overflow during {operation}")
            }
            Self::Truncated {
                expected_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "truncated artifact: expected {expected_bytes} bytes, got {actual_bytes}"
            ),
            Self::TrailingData {
                expected_bytes,
                actual_bytes,
            } => write!(
                formatter,
                "trailing artifact data: expected {expected_bytes} bytes, got {actual_bytes}"
            ),
            Self::ResourceLimitExceeded {
                resource,
                limit,
                actual,
            } => write!(
                formatter,
                "artifact resource limit exceeded for {resource}: {actual} > {limit}"
            ),
            Self::InvalidPublicationContext { field } => {
                write!(formatter, "invalid publication context field {field}")
            }
            Self::InvalidPackageManifest { field } => {
                write!(formatter, "invalid package manifest field {field}")
            }
            Self::InvalidPublicationPath { field } => {
                write!(formatter, "invalid publication path field {field}")
            }
            Self::DurabilityFailure { operation, source } => {
                write!(
                    formatter,
                    "artifact durability failure during {operation}: {source}"
                )
            }
            other => formatter.write_str(other.class()),
        }
    }
}

impl Error for ArtifactError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io(error) | Self::DurabilityFailure { source: error, .. } => Some(error),
            _ => None,
        }
    }
}

impl From<io::Error> for ArtifactError {
    fn from(error: io::Error) -> Self {
        Self::Io(error)
    }
}

pub fn inspect_artifact(
    path: &Path,
    expected_format: ArtifactFormat,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
) -> Result<ArtifactMetadata, ArtifactError> {
    Ok(read_artifact(path, expected_format, expected_kind, limits)?.metadata)
}

pub fn load_tokenizer_package(
    vocab_path: &Path,
    merges_path: &Path,
    expected_format: ArtifactFormat,
    limits: &ArtifactLimits,
) -> Result<Tokenizer, ArtifactError> {
    let (mut vocab_reader, vocab_file_bytes) = open_artifact(vocab_path, limits)?;
    let (mut merge_reader, merge_file_bytes) = open_artifact(merges_path, limits)?;
    load_tokenizer_seekable(
        &mut vocab_reader,
        vocab_file_bytes,
        &mut merge_reader,
        merge_file_bytes,
        expected_format,
        limits,
    )
}

fn open_artifact(
    path: &Path,
    limits: &ArtifactLimits,
) -> Result<(BufReader<File>, u64), ArtifactError> {
    let file = File::open(path)?;
    let file_bytes = file.metadata()?.len();
    enforce_file_limit(file_bytes, limits)?;
    Ok((BufReader::new(file), file_bytes))
}

fn read_artifact(
    path: &Path,
    expected_format: ArtifactFormat,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
) -> Result<ValidatedArtifact, ArtifactError> {
    let (mut reader, file_bytes) = open_artifact(path, limits)?;
    read_seekable(
        &mut reader,
        file_bytes,
        expected_format,
        expected_kind,
        limits,
        ReadPurpose::Inspect,
    )
}

#[cfg(test)]
fn inspect_seekable<R: Read + Seek>(
    reader: &mut R,
    file_bytes: u64,
    expected_format: ArtifactFormat,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
) -> Result<ArtifactMetadata, ArtifactError> {
    Ok(read_seekable(
        reader,
        file_bytes,
        expected_format,
        expected_kind,
        limits,
        ReadPurpose::Inspect,
    )?
    .metadata)
}

fn read_seekable<R: Read + Seek>(
    reader: &mut R,
    file_bytes: u64,
    expected_format: ArtifactFormat,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
    purpose: ReadPurpose,
) -> Result<ValidatedArtifact, ArtifactError> {
    enforce_file_limit(file_bytes, limits)?;
    match expected_format {
        ArtifactFormat::V2U16 => read_v2(reader, file_bytes, expected_kind, limits, purpose),
        ArtifactFormat::V3U32 => read_v3(reader, file_bytes, expected_kind, limits, purpose),
    }
}

fn load_tokenizer_seekable<RV: Read + Seek, RM: Read + Seek>(
    vocab_reader: &mut RV,
    vocab_file_bytes: u64,
    merge_reader: &mut RM,
    merge_file_bytes: u64,
    expected_format: ArtifactFormat,
    limits: &ArtifactLimits,
) -> Result<Tokenizer, ArtifactError> {
    let vocab_artifact = read_seekable(
        vocab_reader,
        vocab_file_bytes,
        expected_format,
        ArtifactKind::Vocab,
        limits,
        ReadPurpose::Load,
    )?;
    let merge_artifact = read_seekable(
        merge_reader,
        merge_file_bytes,
        expected_format,
        ArtifactKind::Merges,
        limits,
        ReadPurpose::Load,
    )?;
    construct_tokenizer(vocab_artifact, merge_artifact)
}

fn construct_tokenizer(
    vocab_artifact: ValidatedArtifact,
    merge_artifact: ValidatedArtifact,
) -> Result<Tokenizer, ArtifactError> {
    validate_paired_metadata(&vocab_artifact.metadata, &merge_artifact.metadata)?;

    let vocab = match vocab_artifact.records {
        ArtifactRecords::Vocab(vocab) => vocab,
        ArtifactRecords::Merges(_) => {
            return Err(ArtifactError::WrongArtifactKind {
                expected: ArtifactKind::Vocab,
                actual: ArtifactKind::Merges.encoded(),
            });
        }
    };
    let merges = match merge_artifact.records {
        ArtifactRecords::Merges(merges) => merges,
        ArtifactRecords::Vocab(_) => {
            return Err(ArtifactError::WrongArtifactKind {
                expected: ArtifactKind::Merges,
                actual: ArtifactKind::Vocab.encoded(),
            });
        }
    };

    Tokenizer::try_from_parts(vocab, merges)
}

fn validate_paired_metadata(
    vocab: &ArtifactMetadata,
    merges: &ArtifactMetadata,
) -> Result<(), ArtifactError> {
    if vocab.format != merges.format {
        return Err(ArtifactError::WrongFormatSelection);
    }
    if vocab.base_vocab_count != merges.base_vocab_count {
        return Err(ArtifactError::CountOutOfRange {
            field: "paired base_vocab_count",
        });
    }
    if vocab.vocab_size != merges.vocab_size {
        return Err(ArtifactError::CountOutOfRange {
            field: "paired vocab_size",
        });
    }
    if vocab.merge_count != merges.merge_count {
        return Err(ArtifactError::CountOutOfRange {
            field: "paired merge_count",
        });
    }
    if vocab.record_count != u64::from(vocab.vocab_size) {
        return Err(ArtifactError::CountOutOfRange {
            field: "paired vocab record_count",
        });
    }
    if merges.record_count != merges.merge_count {
        return Err(ArtifactError::CountOutOfRange {
            field: "paired merge record_count",
        });
    }

    match vocab.format {
        ArtifactFormat::V2U16 => {
            if vocab.payload_sha256.is_some()
                || vocab.sequence_sha256.is_some()
                || merges.payload_sha256.is_some()
                || merges.sequence_sha256.is_some()
            {
                return Err(ArtifactError::SequenceDigestMismatch);
            }
        }
        ArtifactFormat::V3U32 => {
            let vocab_sequence = vocab
                .sequence_sha256
                .ok_or(ArtifactError::SequenceDigestMismatch)?;
            let merge_sequence = merges
                .sequence_sha256
                .ok_or(ArtifactError::SequenceDigestMismatch)?;
            let merge_payload = merges
                .payload_sha256
                .ok_or(ArtifactError::PayloadDigestMismatch)?;
            if vocab_sequence != merge_sequence || merge_payload != merge_sequence {
                return Err(ArtifactError::SequenceDigestMismatch);
            }
        }
    }
    Ok(())
}

fn validate_reconstruction(vocab: &[Vec<u8>], merges: &[BpeMerge]) -> Result<(), ArtifactError> {
    for merge in merges {
        let a_index = usize::try_from(merge.a).map_err(|_| ArtifactError::InvalidTokenId)?;
        let b_index = usize::try_from(merge.b).map_err(|_| ArtifactError::InvalidTokenId)?;
        let merged_index =
            usize::try_from(merge.merged).map_err(|_| ArtifactError::InvalidTokenId)?;
        let a_bytes = vocab
            .get(a_index)
            .ok_or(ArtifactError::ReconstructedTokenMismatch)?;
        let b_bytes = vocab
            .get(b_index)
            .ok_or(ArtifactError::ReconstructedTokenMismatch)?;
        let merged_bytes = vocab
            .get(merged_index)
            .ok_or(ArtifactError::ReconstructedTokenMismatch)?;
        let expected_length =
            a_bytes
                .len()
                .checked_add(b_bytes.len())
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "reconstructed token length",
                })?;
        if merged_bytes.len() != expected_length
            || !merged_bytes.starts_with(a_bytes)
            || merged_bytes[a_bytes.len()..] != b_bytes[..]
        {
            return Err(ArtifactError::ReconstructedTokenMismatch);
        }
    }
    Ok(())
}

fn validate_tokenizer_parts(vocab: &[Vec<u8>], merges: &[BpeMerge]) -> Result<(), ArtifactError> {
    let vocab_size = u32::try_from(vocab.len()).map_err(|_| ArtifactError::CountOutOfRange {
        field: "tokenizer vocab_size",
    })?;
    if !(BPE_TOKEN_START..=MAX_VOCAB_SIZE).contains(&vocab_size) {
        return Err(ArtifactError::CountOutOfRange {
            field: "tokenizer vocab_size",
        });
    }
    let expected_merges = usize::try_from(vocab_size - BPE_TOKEN_START).map_err(|_| {
        ArtifactError::ArithmeticOverflow {
            operation: "tokenizer merge count conversion",
        }
    })?;
    if merges.len() != expected_merges {
        return Err(ArtifactError::CountOutOfRange {
            field: "tokenizer merge_count",
        });
    }

    for (ordinal, token) in vocab.iter().enumerate() {
        let token_id = TokenId::try_from(ordinal).map_err(|_| ArtifactError::InvalidTokenId)?;
        if token_id < BPE_TOKEN_START {
            if !matches_base_token(token_id, token) {
                return Err(ArtifactError::BaseContractMismatch);
            }
        } else if token.is_empty() {
            return Err(ArtifactError::ReconstructedTokenMismatch);
        }
    }

    let merge_count_u64 =
        u64::try_from(merges.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "tokenizer merge count conversion",
        })?;
    let mut seen = HashSet::new();
    seen.try_reserve(merges.len())
        .map_err(|_| ArtifactError::ResourceLimitExceeded {
            resource: "tokenizer_merge_pair_set_allocation",
            limit: u64::from(MAX_VOCAB_SIZE),
            actual: merge_count_u64,
        })?;
    for (ordinal, merge) in merges.iter().enumerate() {
        let ordinal = u64::try_from(ordinal).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "tokenizer merge ordinal conversion",
        })?;
        validate_merge_record(merge.a, merge.b, merge.merged, ordinal, &mut seen)?;
    }
    validate_reconstruction(vocab, merges)
}

fn enforce_file_limit(file_bytes: u64, limits: &ArtifactLimits) -> Result<(), ArtifactError> {
    if file_bytes > limits.max_file_bytes {
        return Err(ArtifactError::ResourceLimitExceeded {
            resource: "file_bytes",
            limit: limits.max_file_bytes,
            actual: file_bytes,
        });
    }
    Ok(())
}

fn read_v3<R: Read + Seek>(
    reader: &mut R,
    file_bytes: u64,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
    purpose: ReadPurpose,
) -> Result<ValidatedArtifact, ArtifactError> {
    if file_bytes < V3_HEADER_BYTES_U64 {
        return Err(ArtifactError::Truncated {
            expected_bytes: V3_HEADER_BYTES_U64,
            actual_bytes: file_bytes,
        });
    }

    reader.seek(SeekFrom::Start(0))?;
    let mut encoded_header = [0u8; V3_HEADER_BYTES];
    read_exact_counted(reader, &mut encoded_header)?;
    let header = ArtifactHeaderV3::parse(&encoded_header, file_bytes, expected_kind)?;

    reader.seek(SeekFrom::Start(V3_HEADER_BYTES_U64))?;
    let actual_payload_sha256 = hash_exact(reader, header.payload_bytes())?;
    if actual_payload_sha256 != header.payload_sha256() {
        return Err(ArtifactError::PayloadDigestMismatch);
    }

    reader.seek(SeekFrom::Start(V3_HEADER_BYTES_U64))?;
    let mut semantic_reader = HashingReader::new(reader);
    let records = match expected_kind {
        ArtifactKind::Vocab => ArtifactRecords::Vocab(parse_vocab_records(
            &mut semantic_reader,
            header.record_count(),
            header.payload_bytes(),
            limits,
            purpose,
        )?),
        ArtifactKind::Merges => parse_merge_records_u32(
            &mut semantic_reader,
            header.record_count(),
            header.payload_bytes(),
            purpose,
        )
        .map(ArtifactRecords::Merges)?,
    };
    if semantic_reader.finish() != header.payload_sha256() {
        return Err(ArtifactError::PayloadDigestMismatch);
    }
    ensure_eof(reader, file_bytes)?;
    Ok(ValidatedArtifact {
        metadata: header.metadata(),
        records,
    })
}

fn read_v2<R: Read + Seek>(
    reader: &mut R,
    file_bytes: u64,
    expected_kind: ArtifactKind,
    limits: &ArtifactLimits,
    purpose: ReadPurpose,
) -> Result<ValidatedArtifact, ArtifactError> {
    if file_bytes >= V3_MAGIC_BYTES_U64 {
        reader.seek(SeekFrom::Start(0))?;
        let mut probe = [0u8; V3_MAGIC.len()];
        read_exact_counted(reader, &mut probe)?;
        if probe == V3_MAGIC {
            return Err(ArtifactError::WrongFormatSelection);
        }
    }
    if file_bytes < V2_COUNT_PREFIX_BYTES {
        return Err(ArtifactError::Truncated {
            expected_bytes: V2_COUNT_PREFIX_BYTES,
            actual_bytes: file_bytes,
        });
    }

    reader.seek(SeekFrom::Start(0))?;
    let count = read_u32_stream(reader)?;
    let remaining_bytes = file_bytes - V2_COUNT_PREFIX_BYTES;

    let artifact = match expected_kind {
        ArtifactKind::Vocab => {
            read_v2_vocab(reader, file_bytes, count, remaining_bytes, limits, purpose)
        }
        ArtifactKind::Merges => read_v2_merges(reader, file_bytes, count, remaining_bytes, purpose),
    }?;
    ensure_eof(reader, file_bytes)?;
    Ok(artifact)
}

fn read_v2_vocab<R: Read>(
    reader: &mut R,
    file_bytes: u64,
    count: u32,
    remaining_bytes: u64,
    limits: &ArtifactLimits,
    purpose: ReadPurpose,
) -> Result<ValidatedArtifact, ArtifactError> {
    if !(V3_BASE_VOCAB_COUNT..=V2_MAX_VOCAB_SIZE).contains(&count) {
        return Err(ArtifactError::CountOutOfRange {
            field: "v2 vocab_count",
        });
    }
    let records = parse_vocab_records(reader, u64::from(count), remaining_bytes, limits, purpose)?;
    Ok(ValidatedArtifact {
        metadata: ArtifactMetadata {
            format: ArtifactFormat::V2U16,
            kind: ArtifactKind::Vocab,
            file_bytes,
            header_bytes: 0,
            record_count: u64::from(count),
            payload_bytes: file_bytes,
            base_vocab_count: V3_BASE_VOCAB_COUNT,
            vocab_size: count,
            merge_count: u64::from(count - V3_BASE_VOCAB_COUNT),
            payload_sha256: None,
            sequence_sha256: None,
        },
        records: ArtifactRecords::Vocab(records),
    })
}

fn read_v2_merges<R: Read>(
    reader: &mut R,
    file_bytes: u64,
    count: u32,
    remaining_bytes: u64,
    purpose: ReadPurpose,
) -> Result<ValidatedArtifact, ArtifactError> {
    if count > V2_MAX_MERGE_COUNT {
        return Err(ArtifactError::CountOutOfRange {
            field: "v2 merge_count",
        });
    }
    let expected_payload_bytes = u64::from(count).checked_mul(V2_MERGE_RECORD_BYTES).ok_or(
        ArtifactError::ArithmeticOverflow {
            operation: "v2 merge payload length",
        },
    )?;
    if remaining_bytes < expected_payload_bytes {
        return Err(ArtifactError::Truncated {
            expected_bytes: expected_payload_bytes,
            actual_bytes: remaining_bytes,
        });
    }
    if remaining_bytes > expected_payload_bytes {
        return Err(ArtifactError::TrailingData {
            expected_bytes: expected_payload_bytes,
            actual_bytes: remaining_bytes,
        });
    }
    let records = parse_merge_records_u16(reader, count, purpose)?;
    Ok(ValidatedArtifact {
        metadata: ArtifactMetadata {
            format: ArtifactFormat::V2U16,
            kind: ArtifactKind::Merges,
            file_bytes,
            header_bytes: 0,
            record_count: u64::from(count),
            payload_bytes: file_bytes,
            base_vocab_count: V3_BASE_VOCAB_COUNT,
            vocab_size: V3_BASE_VOCAB_COUNT + count,
            merge_count: u64::from(count),
            payload_sha256: None,
            sequence_sha256: None,
        },
        records: ArtifactRecords::Merges(records),
    })
}

fn parse_vocab_records<R: Read>(
    reader: &mut R,
    record_count: u64,
    payload_bytes: u64,
    limits: &ArtifactLimits,
    purpose: ReadPurpose,
) -> Result<Vec<Vec<u8>>, ArtifactError> {
    let capacity =
        usize::try_from(record_count).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "vocabulary record capacity conversion",
        })?;
    let mut vocab = Vec::new();
    if purpose.retains_records() {
        vocab
            .try_reserve_exact(capacity)
            .map_err(|_| ArtifactError::ResourceLimitExceeded {
                resource: "vocabulary_record_allocation",
                limit: u64::from(MAX_VOCAB_SIZE),
                actual: record_count,
            })?;
    }
    let mut remaining = payload_bytes;
    let mut total_vocab_bytes = 0u64;
    for ordinal in 0..record_count {
        let token_bytes = read_u32_payload(reader, &mut remaining)?;
        if token_bytes > limits.max_token_bytes {
            return Err(ArtifactError::ResourceLimitExceeded {
                resource: "token_bytes",
                limit: u64::from(limits.max_token_bytes),
                actual: u64::from(token_bytes),
            });
        }
        if remaining < u64::from(token_bytes) {
            return Err(ArtifactError::Truncated {
                expected_bytes: u64::from(token_bytes),
                actual_bytes: remaining,
            });
        }
        let next_total = total_vocab_bytes
            .checked_add(u64::from(token_bytes))
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "total vocabulary bytes",
            })?;
        if next_total > limits.max_total_vocab_bytes {
            return Err(ArtifactError::ResourceLimitExceeded {
                resource: "total_vocab_bytes",
                limit: limits.max_total_vocab_bytes,
                actual: next_total,
            });
        }
        let token_length =
            usize::try_from(token_bytes).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "token length conversion",
            })?;
        let mut token = Vec::new();
        token.try_reserve_exact(token_length).map_err(|_| {
            ArtifactError::ResourceLimitExceeded {
                resource: "token_allocation",
                limit: u64::from(limits.max_token_bytes),
                actual: u64::from(token_bytes),
            }
        })?;
        token.resize(token_length, 0);
        read_payload_exact(reader, &mut remaining, &mut token)?;
        total_vocab_bytes = next_total;

        let token_id = TokenId::try_from(ordinal).map_err(|_| ArtifactError::InvalidTokenId)?;
        if token_id < V3_BASE_VOCAB_COUNT {
            if !matches_base_token(token_id, &token) {
                return Err(ArtifactError::BaseContractMismatch);
            }
        } else if token.is_empty() {
            return Err(ArtifactError::ReconstructedTokenMismatch);
        }
        if purpose.retains_records() {
            vocab.push(token);
        }
    }
    if remaining != 0 {
        return Err(ArtifactError::TrailingData {
            expected_bytes: 0,
            actual_bytes: remaining,
        });
    }
    Ok(vocab)
}

fn matches_base_token(token_id: TokenId, token: &[u8]) -> bool {
    if token_id < SPECIAL_TOKEN_COUNT {
        return canonical_special_bytes(token_id) == Some(token);
    }
    token.len() == 1 && base_byte_token(token[0]) == token_id
}

fn parse_merge_records_u32<R: Read>(
    reader: &mut R,
    record_count: u64,
    payload_bytes: u64,
    purpose: ReadPurpose,
) -> Result<Vec<BpeMerge>, ArtifactError> {
    let mut remaining = payload_bytes;
    let capacity =
        usize::try_from(record_count).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "merge record capacity conversion",
        })?;
    let mut merges = Vec::new();
    if purpose.retains_records() {
        merges
            .try_reserve_exact(capacity)
            .map_err(|_| ArtifactError::ResourceLimitExceeded {
                resource: "merge_record_allocation",
                limit: u64::from(MAX_VOCAB_SIZE),
                actual: record_count,
            })?;
    }
    let mut seen = HashSet::new();
    seen.try_reserve(capacity)
        .map_err(|_| ArtifactError::ResourceLimitExceeded {
            resource: "merge_pair_set_allocation",
            limit: u64::from(MAX_VOCAB_SIZE),
            actual: record_count,
        })?;
    for ordinal in 0..record_count {
        let a: TokenId = read_u32_payload(reader, &mut remaining)?;
        let b: TokenId = read_u32_payload(reader, &mut remaining)?;
        let merged: TokenId = read_u32_payload(reader, &mut remaining)?;
        let merge = validate_merge_record(a, b, merged, ordinal, &mut seen)?;
        if purpose.retains_records() {
            merges.push(merge);
        }
    }
    if remaining != 0 {
        return Err(ArtifactError::TrailingData {
            expected_bytes: 0,
            actual_bytes: remaining,
        });
    }
    Ok(merges)
}

fn parse_merge_records_u16<R: Read>(
    reader: &mut R,
    record_count: u32,
    purpose: ReadPurpose,
) -> Result<Vec<BpeMerge>, ArtifactError> {
    let capacity =
        usize::try_from(record_count).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "legacy merge record capacity conversion",
        })?;
    let mut merges = Vec::new();
    if purpose.retains_records() {
        merges
            .try_reserve_exact(capacity)
            .map_err(|_| ArtifactError::ResourceLimitExceeded {
                resource: "legacy_merge_record_allocation",
                limit: u64::from(V2_MAX_MERGE_COUNT),
                actual: u64::from(record_count),
            })?;
    }
    let mut seen = HashSet::new();
    seen.try_reserve(capacity)
        .map_err(|_| ArtifactError::ResourceLimitExceeded {
            resource: "legacy_merge_pair_set_allocation",
            limit: u64::from(V2_MAX_MERGE_COUNT),
            actual: u64::from(record_count),
        })?;
    for ordinal in 0..record_count {
        let a: TokenId = u32::from(read_u16_stream(reader)?);
        let b: TokenId = u32::from(read_u16_stream(reader)?);
        let merged: TokenId = u32::from(read_u16_stream(reader)?);
        let merge = validate_merge_record(a, b, merged, u64::from(ordinal), &mut seen)?;
        if purpose.retains_records() {
            merges.push(merge);
        }
    }
    Ok(merges)
}

fn validate_merge_record(
    a: TokenId,
    b: TokenId,
    merged: TokenId,
    ordinal: u64,
    seen: &mut HashSet<PairKey>,
) -> Result<BpeMerge, ArtifactError> {
    if a > MAX_TOKEN_ID || b > MAX_TOKEN_ID || merged > MAX_TOKEN_ID {
        return Err(ArtifactError::InvalidTokenId);
    }
    let ordinal_u32 = u32::try_from(ordinal).map_err(|_| ArtifactError::ArithmeticOverflow {
        operation: "merge ordinal conversion",
    })?;
    let expected_merged =
        BPE_TOKEN_START
            .checked_add(ordinal_u32)
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "sequential merge result",
            })?;
    if merged != expected_merged {
        return Err(ArtifactError::NonSequentialResult);
    }
    if a >= merged || b >= merged {
        return Err(ArtifactError::ForwardReference);
    }
    if !seen.insert(pack_pair(a, b)) {
        return Err(ArtifactError::DuplicatePair);
    }
    Ok(BpeMerge { a, b, merged })
}

struct HashingReader<'a, R> {
    inner: &'a mut R,
    hasher: Sha256,
}

impl<'a, R> HashingReader<'a, R> {
    fn new(inner: &'a mut R) -> Self {
        Self {
            inner,
            hasher: Sha256::new(),
        }
    }

    fn finish(self) -> [u8; 32] {
        let digest = self.hasher.finalize();
        let mut result = [0u8; 32];
        result.copy_from_slice(&digest);
        result
    }
}

impl<R: Read> Read for HashingReader<'_, R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let read = self.inner.read(buffer)?;
        self.hasher.update(&buffer[..read]);
        Ok(read)
    }
}

fn hash_exact<R: Read>(reader: &mut R, bytes: u64) -> Result<[u8; 32], ArtifactError> {
    let mut hasher = Sha256::new();
    let mut remaining = bytes;
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    while remaining != 0 {
        let chunk_bytes = remaining.min(HASH_BUFFER_BYTES_U64);
        let chunk_length =
            usize::try_from(chunk_bytes).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "hash buffer length conversion",
            })?;
        read_exact_counted(reader, &mut buffer[..chunk_length])?;
        hasher.update(&buffer[..chunk_length]);
        remaining -= chunk_bytes;
    }
    let digest = hasher.finalize();
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    Ok(result)
}

fn read_u16_stream<R: Read>(reader: &mut R) -> Result<u16, ArtifactError> {
    let mut bytes = [0u8; 2];
    read_exact_counted(reader, &mut bytes)?;
    Ok(u16::from_le_bytes(bytes))
}

fn read_u32_stream<R: Read>(reader: &mut R) -> Result<u32, ArtifactError> {
    let mut bytes = [0u8; 4];
    read_exact_counted(reader, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_u32_payload<R: Read>(reader: &mut R, remaining: &mut u64) -> Result<u32, ArtifactError> {
    let mut bytes = [0u8; 4];
    read_payload_exact(reader, remaining, &mut bytes)?;
    Ok(u32::from_le_bytes(bytes))
}

fn read_payload_exact<R: Read>(
    reader: &mut R,
    remaining: &mut u64,
    buffer: &mut [u8],
) -> Result<(), ArtifactError> {
    let required = u64::try_from(buffer.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
        operation: "payload read length conversion",
    })?;
    if *remaining < required {
        return Err(ArtifactError::Truncated {
            expected_bytes: required,
            actual_bytes: *remaining,
        });
    }
    read_exact_counted(reader, buffer)?;
    *remaining -= required;
    Ok(())
}

fn read_exact_counted<R: Read>(reader: &mut R, buffer: &mut [u8]) -> Result<(), ArtifactError> {
    let mut total_read = 0usize;
    while total_read < buffer.len() {
        match reader.read(&mut buffer[total_read..]) {
            Ok(0) => {
                let expected_bytes =
                    u64::try_from(buffer.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
                        operation: "expected read length conversion",
                    })?;
                let actual_bytes =
                    u64::try_from(total_read).map_err(|_| ArtifactError::ArithmeticOverflow {
                        operation: "actual read length conversion",
                    })?;
                return Err(ArtifactError::Truncated {
                    expected_bytes,
                    actual_bytes,
                });
            }
            Ok(read) => {
                total_read =
                    total_read
                        .checked_add(read)
                        .ok_or(ArtifactError::ArithmeticOverflow {
                            operation: "read byte count",
                        })?
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ArtifactError::Io(error)),
        }
    }
    Ok(())
}

fn ensure_eof<R: Read>(reader: &mut R, expected_file_bytes: u64) -> Result<(), ArtifactError> {
    let mut extra = [0u8; 1];
    loop {
        match reader.read(&mut extra) {
            Ok(0) => return Ok(()),
            Ok(_) => {
                let actual_bytes = expected_file_bytes.checked_add(1).ok_or(
                    ArtifactError::ArithmeticOverflow {
                        operation: "trailing byte diagnostic length",
                    },
                )?;
                return Err(ArtifactError::TrailingData {
                    expected_bytes: expected_file_bytes,
                    actual_bytes,
                });
            }
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(ArtifactError::Io(error)),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactHeaderV3 {
    kind: ArtifactKind,
    record_count: u64,
    payload_bytes: u64,
    vocab_size: u32,
    merge_count: u64,
    payload_sha256: [u8; 32],
    sequence_sha256: [u8; 32],
}

impl ArtifactHeaderV3 {
    pub fn from_vocab_payload(
        vocab_size: u32,
        payload: &[u8],
        sequence_sha256: [u8; 32],
    ) -> Result<Self, ArtifactError> {
        Self::from_payload(ArtifactKind::Vocab, vocab_size, payload, sequence_sha256)
    }

    pub fn from_merge_payload(vocab_size: u32, payload: &[u8]) -> Result<Self, ArtifactError> {
        let sequence_sha256 = sha256(payload);
        Self::from_payload(ArtifactKind::Merges, vocab_size, payload, sequence_sha256)
    }

    pub(crate) fn from_prehashed_vocab_payload(
        vocab_size: u32,
        payload_bytes: u64,
        payload_sha256: [u8; 32],
        sequence_sha256: [u8; 32],
    ) -> Result<Self, ArtifactError> {
        Self::new(
            ArtifactKind::Vocab,
            vocab_size,
            payload_bytes,
            payload_sha256,
            sequence_sha256,
        )
    }

    pub(crate) fn from_prehashed_merge_payload(
        vocab_size: u32,
        payload_bytes: u64,
        payload_sha256: [u8; 32],
    ) -> Result<Self, ArtifactError> {
        Self::new(
            ArtifactKind::Merges,
            vocab_size,
            payload_bytes,
            payload_sha256,
            payload_sha256,
        )
    }

    fn from_payload(
        kind: ArtifactKind,
        vocab_size: u32,
        payload: &[u8],
        sequence_sha256: [u8; 32],
    ) -> Result<Self, ArtifactError> {
        let payload_bytes =
            u64::try_from(payload.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "payload length conversion",
            })?;
        let payload_sha256 = sha256(payload);
        if kind == ArtifactKind::Merges && sequence_sha256 != payload_sha256 {
            return Err(ArtifactError::SequenceDigestMismatch);
        }
        Self::new(
            kind,
            vocab_size,
            payload_bytes,
            payload_sha256,
            sequence_sha256,
        )
    }

    fn new(
        kind: ArtifactKind,
        vocab_size: u32,
        payload_bytes: u64,
        payload_sha256: [u8; 32],
        sequence_sha256: [u8; 32],
    ) -> Result<Self, ArtifactError> {
        let merge_count = checked_merge_count(vocab_size)?;
        let record_count = match kind {
            ArtifactKind::Vocab => u64::from(vocab_size),
            ArtifactKind::Merges => merge_count,
        };
        let header = Self {
            kind,
            record_count,
            payload_bytes,
            vocab_size,
            merge_count,
            payload_sha256,
            sequence_sha256,
        };
        let file_bytes = V3_HEADER_BYTES_U64.checked_add(payload_bytes).ok_or(
            ArtifactError::ArithmeticOverflow {
                operation: "header plus payload length",
            },
        )?;
        header.validate(file_bytes)?;
        Ok(header)
    }

    pub fn parse(
        header_bytes: &[u8],
        file_bytes: u64,
        expected_kind: ArtifactKind,
    ) -> Result<Self, ArtifactError> {
        if header_bytes.len() < V3_HEADER_BYTES {
            let actual_bytes = u64::try_from(header_bytes.len()).map_err(|_| {
                ArtifactError::ArithmeticOverflow {
                    operation: "header slice length conversion",
                }
            })?;
            return Err(ArtifactError::Truncated {
                expected_bytes: V3_HEADER_BYTES_U64,
                actual_bytes,
            });
        }
        if header_bytes.len() > V3_HEADER_BYTES {
            let actual_bytes = u64::try_from(header_bytes.len()).map_err(|_| {
                ArtifactError::ArithmeticOverflow {
                    operation: "header slice length conversion",
                }
            })?;
            return Err(ArtifactError::TrailingData {
                expected_bytes: V3_HEADER_BYTES_U64,
                actual_bytes,
            });
        }
        if header_bytes[0..8] != V3_MAGIC {
            return Err(ArtifactError::BadMagic);
        }

        let major = read_u16(header_bytes, 8);
        let minor = read_u16(header_bytes, 10);
        if major != V3_FORMAT_MAJOR || minor != V3_FORMAT_MINOR {
            return Err(ArtifactError::UnsupportedVersion { major, minor });
        }

        let actual_kind = header_bytes[12];
        let kind =
            ArtifactKind::from_encoded(actual_kind).ok_or(ArtifactError::WrongArtifactKind {
                expected: expected_kind,
                actual: actual_kind,
            })?;
        if kind != expected_kind {
            return Err(ArtifactError::WrongArtifactKind {
                expected: expected_kind,
                actual: actual_kind,
            });
        }
        if header_bytes[13] != V3_ENDIAN_LITTLE {
            return Err(ArtifactError::BadEndianness {
                actual: header_bytes[13],
            });
        }
        if header_bytes[14] != V3_TOKEN_ID_BYTES {
            return Err(ArtifactError::BadIdWidth {
                actual: header_bytes[14],
            });
        }
        if header_bytes[15] != kind.fixed_record_bytes() {
            return Err(ArtifactError::BadRecordWidth {
                kind,
                actual: header_bytes[15],
            });
        }

        let encoded_header_bytes = read_u16(header_bytes, 16);
        if encoded_header_bytes != V3_HEADER_BYTES_U16 {
            return Err(ArtifactError::BadHeaderSize {
                actual: encoded_header_bytes,
            });
        }
        let flags = read_u16(header_bytes, 18);
        if flags != 0 {
            return Err(ArtifactError::UnsupportedFlags { actual: flags });
        }
        if header_bytes[20..24].iter().any(|value| *value != 0)
            || header_bytes[56..64].iter().any(|value| *value != 0)
        {
            return Err(ArtifactError::NonZeroReserved);
        }

        let record_count = read_u64(header_bytes, 24);
        let payload_bytes = read_u64(header_bytes, 32);
        let base_vocab_count = read_u32(header_bytes, 40);
        if base_vocab_count != V3_BASE_VOCAB_COUNT {
            return Err(ArtifactError::BaseContractMismatch);
        }
        let vocab_size = read_u32(header_bytes, 44);
        let merge_count = read_u64(header_bytes, 48);
        let payload_sha256 = read_digest(header_bytes, 64);
        let sequence_sha256 = read_digest(header_bytes, 96);

        let header = Self {
            kind,
            record_count,
            payload_bytes,
            vocab_size,
            merge_count,
            payload_sha256,
            sequence_sha256,
        };
        header.validate(file_bytes)?;
        Ok(header)
    }

    pub fn to_bytes(&self) -> [u8; V3_HEADER_BYTES] {
        let mut bytes = [0u8; V3_HEADER_BYTES];
        bytes[0..8].copy_from_slice(&V3_MAGIC);
        bytes[8..10].copy_from_slice(&V3_FORMAT_MAJOR.to_le_bytes());
        bytes[10..12].copy_from_slice(&V3_FORMAT_MINOR.to_le_bytes());
        bytes[12] = self.kind.encoded();
        bytes[13] = V3_ENDIAN_LITTLE;
        bytes[14] = V3_TOKEN_ID_BYTES;
        bytes[15] = self.kind.fixed_record_bytes();
        bytes[16..18].copy_from_slice(&V3_HEADER_BYTES_U16.to_le_bytes());
        bytes[24..32].copy_from_slice(&self.record_count.to_le_bytes());
        bytes[32..40].copy_from_slice(&self.payload_bytes.to_le_bytes());
        bytes[40..44].copy_from_slice(&V3_BASE_VOCAB_COUNT.to_le_bytes());
        bytes[44..48].copy_from_slice(&self.vocab_size.to_le_bytes());
        bytes[48..56].copy_from_slice(&self.merge_count.to_le_bytes());
        bytes[64..96].copy_from_slice(&self.payload_sha256);
        bytes[96..128].copy_from_slice(&self.sequence_sha256);
        bytes
    }

    pub const fn kind(&self) -> ArtifactKind {
        self.kind
    }

    pub const fn record_count(&self) -> u64 {
        self.record_count
    }

    pub const fn payload_bytes(&self) -> u64 {
        self.payload_bytes
    }

    pub const fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    pub const fn merge_count(&self) -> u64 {
        self.merge_count
    }

    pub const fn payload_sha256(&self) -> [u8; 32] {
        self.payload_sha256
    }

    pub const fn sequence_sha256(&self) -> [u8; 32] {
        self.sequence_sha256
    }

    pub const fn metadata(&self) -> ArtifactMetadata {
        ArtifactMetadata {
            format: ArtifactFormat::V3U32,
            kind: self.kind,
            file_bytes: V3_HEADER_BYTES_U64 + self.payload_bytes,
            header_bytes: V3_HEADER_BYTES_U16,
            record_count: self.record_count,
            payload_bytes: self.payload_bytes,
            base_vocab_count: V3_BASE_VOCAB_COUNT,
            vocab_size: self.vocab_size,
            merge_count: self.merge_count,
            payload_sha256: Some(self.payload_sha256),
            sequence_sha256: Some(self.sequence_sha256),
        }
    }

    fn validate(&self, file_bytes: u64) -> Result<(), ArtifactError> {
        let expected_merge_count = checked_merge_count(self.vocab_size)?;
        if self.merge_count != expected_merge_count {
            return Err(ArtifactError::CountOutOfRange {
                field: "merge_count",
            });
        }
        let expected_record_count = match self.kind {
            ArtifactKind::Vocab => u64::from(self.vocab_size),
            ArtifactKind::Merges => self.merge_count,
        };
        if self.record_count != expected_record_count {
            return Err(ArtifactError::CountOutOfRange {
                field: "record_count",
            });
        }
        if self.kind == ArtifactKind::Merges {
            let expected_payload_bytes = self
                .record_count
                .checked_mul(u64::from(V3_MERGE_RECORD_BYTES))
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "merge record byte count",
                })?;
            if self.payload_bytes != expected_payload_bytes {
                return Err(ArtifactError::CountOutOfRange {
                    field: "payload_bytes",
                });
            }
            if self.sequence_sha256 != self.payload_sha256 {
                return Err(ArtifactError::SequenceDigestMismatch);
            }
        }

        let expected_file_bytes = V3_HEADER_BYTES_U64.checked_add(self.payload_bytes).ok_or(
            ArtifactError::ArithmeticOverflow {
                operation: "header plus payload length",
            },
        )?;
        if file_bytes < expected_file_bytes {
            return Err(ArtifactError::Truncated {
                expected_bytes: expected_file_bytes,
                actual_bytes: file_bytes,
            });
        }
        if file_bytes > expected_file_bytes {
            return Err(ArtifactError::TrailingData {
                expected_bytes: expected_file_bytes,
                actual_bytes: file_bytes,
            });
        }
        Ok(())
    }
}

fn checked_merge_count(vocab_size: u32) -> Result<u64, ArtifactError> {
    if !(V3_BASE_VOCAB_COUNT..=MAX_VOCAB_SIZE).contains(&vocab_size) {
        return Err(ArtifactError::CountOutOfRange {
            field: "vocab_size",
        });
    }
    Ok(u64::from(vocab_size - V3_BASE_VOCAB_COUNT))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    let digest = Sha256::digest(bytes);
    let mut result = [0u8; 32];
    result.copy_from_slice(&digest);
    result
}

fn read_u16(bytes: &[u8], offset: usize) -> u16 {
    u16::from_le_bytes([bytes[offset], bytes[offset + 1]])
}

fn read_u32(bytes: &[u8], offset: usize) -> u32 {
    u32::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
    ])
}

fn read_u64(bytes: &[u8], offset: usize) -> u64 {
    u64::from_le_bytes([
        bytes[offset],
        bytes[offset + 1],
        bytes[offset + 2],
        bytes[offset + 3],
        bytes[offset + 4],
        bytes[offset + 5],
        bytes[offset + 6],
        bytes[offset + 7],
    ])
}

fn read_digest(bytes: &[u8], offset: usize) -> [u8; 32] {
    let mut digest = [0u8; 32];
    digest.copy_from_slice(&bytes[offset..offset + 32]);
    digest
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::token::CANONICAL_SPECIAL_TOKENS;
    use std::io::Cursor;

    const EMPTY_SHA256_HEX: &str =
        "E3B0C44298FC1C149AFBF4C8996FB92427AE41E4649B934CA495991B7852B855";
    const ONE_MERGE_SHA256_HEX: &str =
        "53887809DDF78304283754676329289EC235EADA6AE7F5BFB02C97A2FB276FA9";

    fn decode_digest(hex: &str) -> [u8; 32] {
        assert_eq!(hex.len(), 64);
        let mut digest = [0u8; 32];
        for (index, pair) in hex.as_bytes().chunks_exact(2).enumerate() {
            digest[index] = (nibble(pair[0]) << 4) | nibble(pair[1]);
        }
        digest
    }

    fn nibble(value: u8) -> u8 {
        match value {
            b'0'..=b'9' => value - b'0',
            b'A'..=b'F' => value - b'A' + 10,
            _ => panic!("invalid test digest hex"),
        }
    }

    fn empty_merge_header() -> ArtifactHeaderV3 {
        ArtifactHeaderV3::from_merge_payload(276, &[]).unwrap()
    }

    fn canonical_base_vocab_payload() -> Vec<u8> {
        let mut payload = Vec::new();
        for special in CANONICAL_SPECIAL_TOKENS {
            let length = u32::try_from(special.bytes.len()).unwrap();
            payload.extend_from_slice(&length.to_le_bytes());
            payload.extend_from_slice(special.bytes);
        }
        for byte in u8::MIN..=u8::MAX {
            payload.extend_from_slice(&1u32.to_le_bytes());
            payload.push(byte);
        }
        payload
    }

    fn parse_empty_merge(bytes: &[u8], file_bytes: u64) -> Result<ArtifactHeaderV3, ArtifactError> {
        ArtifactHeaderV3::parse(bytes, file_bytes, ArtifactKind::Merges)
    }

    fn generous_limits() -> ArtifactLimits {
        ArtifactLimits {
            max_file_bytes: 1_000_000,
            max_total_vocab_bytes: 10_000_000,
            max_token_bytes: 1_000_000,
        }
    }

    fn inspect_bytes(
        bytes: &[u8],
        format: ArtifactFormat,
        kind: ArtifactKind,
        limits: &ArtifactLimits,
    ) -> Result<ArtifactMetadata, ArtifactError> {
        let mut cursor = Cursor::new(bytes);
        inspect_seekable(
            &mut cursor,
            u64::try_from(bytes.len()).unwrap(),
            format,
            kind,
            limits,
        )
    }

    fn load_bytes(
        vocab_bytes: &[u8],
        merge_bytes: &[u8],
        format: ArtifactFormat,
        limits: &ArtifactLimits,
    ) -> Result<Tokenizer, ArtifactError> {
        let mut vocab_reader = Cursor::new(vocab_bytes);
        let mut merge_reader = Cursor::new(merge_bytes);
        load_tokenizer_seekable(
            &mut vocab_reader,
            u64::try_from(vocab_bytes.len()).unwrap(),
            &mut merge_reader,
            u64::try_from(merge_bytes.len()).unwrap(),
            format,
            limits,
        )
    }

    fn v3_file(header: &ArtifactHeaderV3, payload: &[u8]) -> Vec<u8> {
        let mut file = header.to_bytes().to_vec();
        file.extend_from_slice(payload);
        file
    }

    fn push_vocab_record(payload: &mut Vec<u8>, token: &[u8]) {
        payload.extend_from_slice(&u32::try_from(token.len()).unwrap().to_le_bytes());
        payload.extend_from_slice(token);
    }

    fn v2_vocab_file(count: u32, payload: &[u8]) -> Vec<u8> {
        let mut file = count.to_le_bytes().to_vec();
        file.extend_from_slice(payload);
        file
    }

    fn v2_merge_file(records: &[(u16, u16, u16)]) -> Vec<u8> {
        let mut file = u32::try_from(records.len()).unwrap().to_le_bytes().to_vec();
        for &(a, b, merged) in records {
            file.extend_from_slice(&a.to_le_bytes());
            file.extend_from_slice(&b.to_le_bytes());
            file.extend_from_slice(&merged.to_le_bytes());
        }
        file
    }

    #[test]
    fn empty_merge_header_matches_locked_vector() {
        let expected_digest = decode_digest(EMPTY_SHA256_HEX);
        let header = empty_merge_header();
        let bytes = header.to_bytes();

        assert_eq!(&bytes[0..8], &V3_MAGIC);
        assert_eq!(read_u16(&bytes, 8), 3);
        assert_eq!(read_u16(&bytes, 10), 0);
        assert_eq!(bytes[12], 2);
        assert_eq!(bytes[13], 1);
        assert_eq!(bytes[14], 4);
        assert_eq!(bytes[15], 12);
        assert_eq!(read_u16(&bytes, 16), 128);
        assert_eq!(read_u64(&bytes, 24), 0);
        assert_eq!(read_u64(&bytes, 32), 0);
        assert_eq!(read_u32(&bytes, 40), 276);
        assert_eq!(read_u32(&bytes, 44), 276);
        assert_eq!(read_u64(&bytes, 48), 0);
        assert_eq!(read_digest(&bytes, 64), expected_digest);
        assert_eq!(read_digest(&bytes, 96), expected_digest);

        let parsed = parse_empty_merge(&bytes, 128).unwrap();
        assert_eq!(parsed, header);
        assert_eq!(parsed.metadata().format, ArtifactFormat::V3U32);
        assert_eq!(parsed.metadata().file_bytes, 128);
    }

    #[test]
    fn canonical_base_vocab_header_uses_vocab_kind_fields() {
        let payload = canonical_base_vocab_payload();
        let sequence = decode_digest(EMPTY_SHA256_HEX);
        let header =
            ArtifactHeaderV3::from_vocab_payload(V3_BASE_VOCAB_COUNT, &payload, sequence).unwrap();
        let bytes = header.to_bytes();
        let file_bytes = V3_HEADER_BYTES_U64 + u64::try_from(payload.len()).unwrap();

        assert_eq!(bytes[12], 1);
        assert_eq!(bytes[15], 0);
        assert_eq!(read_u64(&bytes, 24), 276);
        assert_eq!(read_u64(&bytes, 32), u64::try_from(payload.len()).unwrap());
        assert_eq!(read_u32(&bytes, 44), 276);
        assert_eq!(read_u64(&bytes, 48), 0);
        assert_eq!(read_digest(&bytes, 64), sha256(&payload));
        assert_eq!(read_digest(&bytes, 96), sequence);
        assert_eq!(
            ArtifactHeaderV3::parse(&bytes, file_bytes, ArtifactKind::Vocab).unwrap(),
            header
        );
        assert_eq!(header.metadata().file_bytes, file_bytes);
    }

    #[test]
    fn one_merge_payload_matches_locked_digest_and_header_fields() {
        let payload = [
            0x34, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x14, 0x01, 0x00, 0x00,
        ];
        let expected_digest = decode_digest(ONE_MERGE_SHA256_HEX);
        let header = ArtifactHeaderV3::from_merge_payload(277, &payload).unwrap();
        let bytes = header.to_bytes();

        assert_eq!(header.payload_sha256(), expected_digest);
        assert_eq!(header.sequence_sha256(), expected_digest);
        assert_eq!(read_u64(&bytes, 24), 1);
        assert_eq!(read_u64(&bytes, 32), 12);
        assert_eq!(read_u32(&bytes, 44), 277);
        assert_eq!(read_u64(&bytes, 48), 1);
        assert_eq!(
            ArtifactHeaderV3::parse(&bytes, 140, ArtifactKind::Merges).unwrap(),
            header
        );
    }

    #[test]
    fn header_rejects_fixed_field_mutations_in_validation_order() {
        let original = empty_merge_header().to_bytes();
        let cases: &[(usize, u8, &str)] = &[
            (0, 0, "BadMagic"),
            (8, 4, "UnsupportedVersion"),
            (10, 1, "UnsupportedVersion"),
            (12, 1, "WrongArtifactKind"),
            (13, 2, "BadEndianness"),
            (14, 2, "BadIdWidth"),
            (15, 6, "BadRecordWidth"),
            (16, 96, "BadHeaderSize"),
            (18, 1, "UnsupportedFlags"),
            (20, 1, "NonZeroReserved"),
            (56, 1, "NonZeroReserved"),
            (40, 21, "BaseContractMismatch"),
        ];

        for &(offset, value, expected_class) in cases {
            let mut mutated = original;
            mutated[offset] = value;
            let error = parse_empty_merge(&mutated, 128).unwrap_err();
            assert_eq!(error.class(), expected_class, "offset {offset}");
        }
    }

    #[test]
    fn header_rejects_count_length_sequence_and_boundary_defects() {
        let original = empty_merge_header().to_bytes();

        let mut low_vocab = original;
        low_vocab[44..48].copy_from_slice(&275u32.to_le_bytes());
        assert_eq!(
            parse_empty_merge(&low_vocab, 128).unwrap_err().class(),
            "CountOutOfRange"
        );

        let mut high_vocab = original;
        high_vocab[44..48].copy_from_slice(&131_073u32.to_le_bytes());
        assert_eq!(
            parse_empty_merge(&high_vocab, 128).unwrap_err().class(),
            "CountOutOfRange"
        );

        let mut bad_merge_count = original;
        bad_merge_count[48..56].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(
            parse_empty_merge(&bad_merge_count, 128)
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );

        let mut bad_record_count = original;
        bad_record_count[24..32].copy_from_slice(&1u64.to_le_bytes());
        assert_eq!(
            parse_empty_merge(&bad_record_count, 128)
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );

        let mut bad_payload_bytes = original;
        bad_payload_bytes[32..40].copy_from_slice(&12u64.to_le_bytes());
        assert_eq!(
            parse_empty_merge(&bad_payload_bytes, 140)
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );

        let mut bad_sequence = original;
        bad_sequence[96] ^= 1;
        assert_eq!(
            parse_empty_merge(&bad_sequence, 128).unwrap_err().class(),
            "SequenceDigestMismatch"
        );

        assert_eq!(
            parse_empty_merge(&original[..127], 128)
                .unwrap_err()
                .class(),
            "Truncated"
        );
        let mut oversized_header = original.to_vec();
        oversized_header.push(0);
        assert_eq!(
            parse_empty_merge(&oversized_header, 128)
                .unwrap_err()
                .class(),
            "TrailingData"
        );
        assert_eq!(
            parse_empty_merge(&original, 127).unwrap_err().class(),
            "Truncated"
        );
        assert_eq!(
            parse_empty_merge(&original, 129).unwrap_err().class(),
            "TrailingData"
        );
    }

    #[test]
    fn constructor_rejects_invalid_vocab_and_merge_payload_relations() {
        assert_eq!(
            ArtifactHeaderV3::from_merge_payload(275, &[])
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );
        assert_eq!(
            ArtifactHeaderV3::from_merge_payload(131_073, &[])
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );
        assert_eq!(
            ArtifactHeaderV3::from_merge_payload(277, &[0; 11])
                .unwrap_err()
                .class(),
            "CountOutOfRange"
        );
    }

    #[test]
    fn inspect_v3_vocab_and_merges_after_digest_verification() {
        let limits = generous_limits();
        let vocab_payload = canonical_base_vocab_payload();
        let sequence = decode_digest(EMPTY_SHA256_HEX);
        let vocab_header =
            ArtifactHeaderV3::from_vocab_payload(V3_BASE_VOCAB_COUNT, &vocab_payload, sequence)
                .unwrap();
        let vocab_file = v3_file(&vocab_header, &vocab_payload);
        let vocab = inspect_bytes(
            &vocab_file,
            ArtifactFormat::V3U32,
            ArtifactKind::Vocab,
            &limits,
        )
        .unwrap();
        assert_eq!(vocab.vocab_size, 276);
        assert_eq!(vocab.merge_count, 0);
        assert_eq!(vocab.payload_sha256, Some(sha256(&vocab_payload)));
        assert_eq!(vocab.sequence_sha256, Some(sequence));

        let merge_payload = [
            0x34, 0x00, 0x00, 0x00, 0x88, 0x00, 0x00, 0x00, 0x14, 0x01, 0x00, 0x00,
        ];
        let merge_header = ArtifactHeaderV3::from_merge_payload(277, &merge_payload).unwrap();
        let merge_file = v3_file(&merge_header, &merge_payload);
        let merges = inspect_bytes(
            &merge_file,
            ArtifactFormat::V3U32,
            ArtifactKind::Merges,
            &limits,
        )
        .unwrap();
        assert_eq!(merges.record_count, 1);
        assert_eq!(merges.vocab_size, 277);
        assert_eq!(merges.payload_sha256, merges.sequence_sha256);
    }

    #[test]
    fn load_v3_constructs_immutable_tokenizer_and_lookup() {
        let merge_payload = [
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let merge_header = ArtifactHeaderV3::from_merge_payload(277, &merge_payload).unwrap();
        let merge_file = v3_file(&merge_header, &merge_payload);

        let mut vocab_payload = canonical_base_vocab_payload();
        push_vocab_record(&mut vocab_payload, b" t");
        let vocab_header =
            ArtifactHeaderV3::from_vocab_payload(277, &vocab_payload, sha256(&merge_payload))
                .unwrap();
        let vocab_file = v3_file(&vocab_header, &vocab_payload);

        let tokenizer = load_bytes(
            &vocab_file,
            &merge_file,
            ArtifactFormat::V3U32,
            &generous_limits(),
        )
        .unwrap();
        assert_eq!(tokenizer.vocab_size(), 277);
        assert_eq!(tokenizer.merge_count(), 1);
        assert_eq!(tokenizer.token_bytes(276), Some(b" t".as_slice()));
        assert_eq!(
            tokenizer.merge_at(0),
            Some(&BpeMerge {
                a: 52,
                b: 136,
                merged: 276,
            })
        );
        assert_eq!(tokenizer.merged_token(52, 136), Some(276));
        assert_eq!(tokenizer.merged_token(136, 52), None);
    }

    #[test]
    fn paired_v3_loader_rejects_sequence_and_reconstruction_mismatches() {
        let merge_a = [
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let merge_b = [
            52u32.to_le_bytes(),
            137u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let merge_a_file = v3_file(
            &ArtifactHeaderV3::from_merge_payload(277, &merge_a).unwrap(),
            &merge_a,
        );
        let merge_b_file = v3_file(
            &ArtifactHeaderV3::from_merge_payload(277, &merge_b).unwrap(),
            &merge_b,
        );

        let mut matching_vocab_payload = canonical_base_vocab_payload();
        push_vocab_record(&mut matching_vocab_payload, b" t");
        let matching_vocab_file = v3_file(
            &ArtifactHeaderV3::from_vocab_payload(277, &matching_vocab_payload, sha256(&merge_a))
                .unwrap(),
            &matching_vocab_payload,
        );
        assert_eq!(
            load_bytes(
                &matching_vocab_file,
                &merge_b_file,
                ArtifactFormat::V3U32,
                &generous_limits(),
            )
            .unwrap_err()
            .class(),
            "SequenceDigestMismatch"
        );

        let mut wrong_vocab_payload = canonical_base_vocab_payload();
        push_vocab_record(&mut wrong_vocab_payload, b" x");
        let wrong_vocab_file = v3_file(
            &ArtifactHeaderV3::from_vocab_payload(277, &wrong_vocab_payload, sha256(&merge_a))
                .unwrap(),
            &wrong_vocab_payload,
        );
        assert_eq!(
            load_bytes(
                &wrong_vocab_file,
                &merge_a_file,
                ArtifactFormat::V3U32,
                &generous_limits(),
            )
            .unwrap_err()
            .class(),
            "ReconstructedTokenMismatch"
        );
    }

    #[test]
    fn paired_v2_loader_rejects_count_mismatch_and_mixed_format() {
        let mut vocab_payload = canonical_base_vocab_payload();
        push_vocab_record(&mut vocab_payload, b" t");
        push_vocab_record(&mut vocab_payload, b" u");
        let vocab_file = v2_vocab_file(278, &vocab_payload);
        let merge_file = v2_merge_file(&[(52, 136, 276)]);
        assert_eq!(
            load_bytes(
                &vocab_file,
                &merge_file,
                ArtifactFormat::V2U16,
                &generous_limits(),
            )
            .unwrap_err()
            .class(),
            "CountOutOfRange"
        );

        let v2_vocab = v2_vocab_file(276, &canonical_base_vocab_payload());
        let v3_merges = v3_file(
            &ArtifactHeaderV3::from_merge_payload(276, &[]).unwrap(),
            &[],
        );
        assert_eq!(
            load_bytes(
                &v2_vocab,
                &v3_merges,
                ArtifactFormat::V2U16,
                &generous_limits(),
            )
            .unwrap_err()
            .class(),
            "WrongFormatSelection"
        );
    }

    #[test]
    fn inspect_v3_rejects_corruption_before_semantics_and_duplicates_after_hash() {
        let limits = generous_limits();
        let one_merge = [
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let header = ArtifactHeaderV3::from_merge_payload(277, &one_merge).unwrap();
        let mut corrupted = v3_file(&header, &one_merge);
        corrupted[V3_HEADER_BYTES] ^= 1;
        assert_eq!(
            inspect_bytes(
                &corrupted,
                ArtifactFormat::V3U32,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "PayloadDigestMismatch"
        );

        let duplicate_payload = [
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            277u32.to_le_bytes(),
        ]
        .concat();
        let duplicate_header =
            ArtifactHeaderV3::from_merge_payload(278, &duplicate_payload).unwrap();
        let duplicate_file = v3_file(&duplicate_header, &duplicate_payload);
        assert_eq!(
            inspect_bytes(
                &duplicate_file,
                ArtifactFormat::V3U32,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "DuplicatePair"
        );
    }

    #[test]
    fn inspect_v3_binds_successful_semantic_pass_to_verified_payload() {
        struct MutatingCursor {
            cursor: Cursor<Vec<u8>>,
            replacement_payload: Vec<u8>,
            payload_seek_count: u8,
        }

        impl Read for MutatingCursor {
            fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
                self.cursor.read(buffer)
            }
        }

        impl Seek for MutatingCursor {
            fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
                if position == SeekFrom::Start(V3_HEADER_BYTES_U64) {
                    self.payload_seek_count += 1;
                    if self.payload_seek_count == 2 {
                        self.cursor.get_mut()[V3_HEADER_BYTES..]
                            .copy_from_slice(&self.replacement_payload);
                    }
                }
                self.cursor.seek(position)
            }
        }

        let original_payload = [
            52u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let replacement_payload = [
            53u32.to_le_bytes(),
            136u32.to_le_bytes(),
            276u32.to_le_bytes(),
        ]
        .concat();
        let header = ArtifactHeaderV3::from_merge_payload(277, &original_payload).unwrap();
        let file = v3_file(&header, &original_payload);
        let file_bytes = u64::try_from(file.len()).unwrap();
        let mut reader = MutatingCursor {
            cursor: Cursor::new(file),
            replacement_payload,
            payload_seek_count: 0,
        };

        assert_eq!(
            inspect_seekable(
                &mut reader,
                file_bytes,
                ArtifactFormat::V3U32,
                ArtifactKind::Merges,
                &generous_limits(),
            )
            .unwrap_err()
            .class(),
            "PayloadDigestMismatch"
        );
    }

    #[test]
    fn inspect_v3_enforces_resource_limits_and_base_contract() {
        let payload = canonical_base_vocab_payload();
        let header =
            ArtifactHeaderV3::from_vocab_payload(276, &payload, decode_digest(EMPTY_SHA256_HEX))
                .unwrap();
        let file = v3_file(&header, &payload);

        let mut file_limited = generous_limits();
        file_limited.max_file_bytes = u64::try_from(file.len() - 1).unwrap();
        assert_eq!(
            inspect_bytes(
                &file,
                ArtifactFormat::V3U32,
                ArtifactKind::Vocab,
                &file_limited
            )
            .unwrap_err()
            .class(),
            "ResourceLimitExceeded"
        );

        let mut token_limited = generous_limits();
        token_limited.max_token_bytes = 4;
        assert_eq!(
            inspect_bytes(
                &file,
                ArtifactFormat::V3U32,
                ArtifactKind::Vocab,
                &token_limited
            )
            .unwrap_err()
            .class(),
            "ResourceLimitExceeded"
        );

        let mut bad_base = payload;
        bad_base[4] ^= 1;
        let bad_header =
            ArtifactHeaderV3::from_vocab_payload(276, &bad_base, decode_digest(EMPTY_SHA256_HEX))
                .unwrap();
        let bad_file = v3_file(&bad_header, &bad_base);
        assert_eq!(
            inspect_bytes(
                &bad_file,
                ArtifactFormat::V3U32,
                ArtifactKind::Vocab,
                &generous_limits()
            )
            .unwrap_err()
            .class(),
            "BaseContractMismatch"
        );
    }

    #[test]
    fn inspect_v2_widens_vocab_and_merges_without_digest_claims() {
        let limits = generous_limits();
        let mut vocab_payload = canonical_base_vocab_payload();
        push_vocab_record(&mut vocab_payload, b" t");
        let vocab_file = v2_vocab_file(277, &vocab_payload);
        let vocab = inspect_bytes(
            &vocab_file,
            ArtifactFormat::V2U16,
            ArtifactKind::Vocab,
            &limits,
        )
        .unwrap();
        assert_eq!(vocab.vocab_size, 277);
        assert_eq!(vocab.merge_count, 1);
        assert_eq!(vocab.payload_sha256, None);
        assert_eq!(vocab.sequence_sha256, None);

        let merge_file = v2_merge_file(&[(52, 136, 276)]);
        let merges = inspect_bytes(
            &merge_file,
            ArtifactFormat::V2U16,
            ArtifactKind::Merges,
            &limits,
        )
        .unwrap();
        assert_eq!(merges.record_count, 1);
        assert_eq!(merges.vocab_size, 277);
        assert_eq!(merges.header_bytes, 0);
        assert_eq!(merges.payload_sha256, None);
    }

    #[test]
    fn inspection_validates_without_retaining_package_records() {
        let vocab_file = v2_vocab_file(276, &canonical_base_vocab_payload());
        let mut vocab_reader = Cursor::new(vocab_file.as_slice());
        let vocab = read_seekable(
            &mut vocab_reader,
            u64::try_from(vocab_file.len()).unwrap(),
            ArtifactFormat::V2U16,
            ArtifactKind::Vocab,
            &generous_limits(),
            ReadPurpose::Inspect,
        )
        .unwrap();
        match vocab.records {
            ArtifactRecords::Vocab(records) => assert!(records.is_empty()),
            ArtifactRecords::Merges(_) => panic!("vocab inspection returned merge records"),
        }

        let merge_file = v2_merge_file(&[(52, 136, 276)]);
        let mut merge_reader = Cursor::new(merge_file.as_slice());
        let merges = read_seekable(
            &mut merge_reader,
            u64::try_from(merge_file.len()).unwrap(),
            ArtifactFormat::V2U16,
            ArtifactKind::Merges,
            &generous_limits(),
            ReadPurpose::Inspect,
        )
        .unwrap();
        match merges.records {
            ArtifactRecords::Merges(records) => assert!(records.is_empty()),
            ArtifactRecords::Vocab(_) => panic!("merge inspection returned vocab records"),
        }
    }

    #[test]
    fn inspect_selection_and_v2_malformed_inputs_fail_closed() {
        let limits = generous_limits();
        let v3_header = ArtifactHeaderV3::from_merge_payload(276, &[]).unwrap();
        let v3_file = v3_file(&v3_header, &[]);
        assert_eq!(
            inspect_bytes(
                &v3_file,
                ArtifactFormat::V2U16,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "WrongFormatSelection"
        );

        let v2_vocab = v2_vocab_file(276, &canonical_base_vocab_payload());
        assert_eq!(
            inspect_bytes(
                &v2_vocab,
                ArtifactFormat::V3U32,
                ArtifactKind::Vocab,
                &limits
            )
            .unwrap_err()
            .class(),
            "BadMagic"
        );

        let mut trailing = v2_merge_file(&[(52, 136, 276)]);
        trailing.push(0);
        assert_eq!(
            inspect_bytes(
                &trailing,
                ArtifactFormat::V2U16,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "TrailingData"
        );

        let duplicate = v2_merge_file(&[(52, 136, 276), (52, 136, 277)]);
        assert_eq!(
            inspect_bytes(
                &duplicate,
                ArtifactFormat::V2U16,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "DuplicatePair"
        );

        let forward = v2_merge_file(&[(276, 1, 276)]);
        assert_eq!(
            inspect_bytes(
                &forward,
                ArtifactFormat::V2U16,
                ArtifactKind::Merges,
                &limits
            )
            .unwrap_err()
            .class(),
            "ForwardReference"
        );
    }

    #[test]
    fn inspect_v2_vocab_rejects_base_mismatch_and_token_limit() {
        let mut payload = canonical_base_vocab_payload();
        payload[4] ^= 1;
        let bad_base = v2_vocab_file(276, &payload);
        assert_eq!(
            inspect_bytes(
                &bad_base,
                ArtifactFormat::V2U16,
                ArtifactKind::Vocab,
                &generous_limits()
            )
            .unwrap_err()
            .class(),
            "BaseContractMismatch"
        );

        let file = v2_vocab_file(276, &canonical_base_vocab_payload());
        let mut limits = generous_limits();
        limits.max_total_vocab_bytes = 1;
        assert_eq!(
            inspect_bytes(&file, ArtifactFormat::V2U16, ArtifactKind::Vocab, &limits)
                .unwrap_err()
                .class(),
            "ResourceLimitExceeded"
        );
    }

    #[test]
    fn declared_token_length_must_fit_remaining_before_allocation() {
        let mut file = 276u32.to_le_bytes().to_vec();
        file.extend_from_slice(&u32::MAX.to_le_bytes());
        let limits = ArtifactLimits {
            max_file_bytes: 1_000_000,
            max_total_vocab_bytes: u64::MAX,
            max_token_bytes: u32::MAX,
        };
        assert_eq!(
            inspect_bytes(&file, ArtifactFormat::V2U16, ArtifactKind::Vocab, &limits)
                .unwrap_err()
                .class(),
            "Truncated"
        );
    }

    #[test]
    fn v2_reader_rechecks_eof_after_declared_bytes() {
        let mut file = v2_vocab_file(276, &canonical_base_vocab_payload());
        let declared_bytes = u64::try_from(file.len()).unwrap();
        file.push(0);
        let mut cursor = Cursor::new(file.as_slice());
        assert_eq!(
            inspect_seekable(
                &mut cursor,
                declared_bytes,
                ArtifactFormat::V2U16,
                ArtifactKind::Vocab,
                &generous_limits()
            )
            .unwrap_err()
            .class(),
            "TrailingData"
        );
    }

    #[test]
    fn canonical_public_v2_32k_artifacts_inspect_strictly() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("runs/full_32768");
        let limits = ArtifactLimits {
            max_file_bytes: 1_000_000,
            max_total_vocab_bytes: 10_000_000,
            max_token_bytes: 1_000_000,
        };
        let vocab = inspect_artifact(
            &root.join("vocab.bin"),
            ArtifactFormat::V2U16,
            ArtifactKind::Vocab,
            &limits,
        )
        .unwrap();
        let merges = inspect_artifact(
            &root.join("merges.bin"),
            ArtifactFormat::V2U16,
            ArtifactKind::Merges,
            &limits,
        )
        .unwrap();

        assert_eq!(vocab.vocab_size, 32_768);
        assert_eq!(vocab.merge_count, 32_492);
        assert_eq!(merges.vocab_size, 32_768);
        assert_eq!(merges.merge_count, 32_492);
        assert_eq!(vocab.payload_sha256, None);
        assert_eq!(merges.payload_sha256, None);

        let tokenizer = load_tokenizer_package(
            &root.join("vocab.bin"),
            &root.join("merges.bin"),
            ArtifactFormat::V2U16,
            &limits,
        )
        .unwrap();
        assert_eq!(tokenizer.vocab_size(), 32_768);
        assert_eq!(tokenizer.merge_count(), 32_492);
        assert_eq!(tokenizer.token_bytes(0), Some(b"<PAD>".as_slice()));
        assert_eq!(tokenizer.token_bytes(20), Some([0].as_slice()));
        assert_eq!(tokenizer.token_bytes(275), Some([255].as_slice()));
        assert_eq!(tokenizer.token_bytes(276), Some(b" t".as_slice()));
        assert_eq!(tokenizer.token_bytes(32_767), Some(b" Vikram".as_slice()));
        assert_eq!(
            tokenizer.merge_at(0),
            Some(&BpeMerge {
                a: 52,
                b: 136,
                merged: 276,
            })
        );
        assert_eq!(tokenizer.merged_token(52, 136), Some(276));
        assert_eq!(tokenizer.merge_lookup.len(), tokenizer.merge_count());
        for merge in &tokenizer.merges {
            assert_eq!(tokenizer.merged_token(merge.a, merge.b), Some(merge.merged));
        }
    }
}
