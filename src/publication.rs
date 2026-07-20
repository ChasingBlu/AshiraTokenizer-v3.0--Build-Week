use crate::artifact::{
    ArtifactError, ArtifactHeaderV3, Tokenizer, V3_FORMAT_MAJOR, V3_FORMAT_MINOR, V3_HEADER_BYTES,
};
use crate::token::{BPE_TOKEN_START, MAX_VOCAB_SIZE};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const PACKAGE_MANIFEST_SCHEMA: &str = "ashira_v3_artifact_package_manifest_v1";
pub const MAX_PACKAGE_MANIFEST_BYTES: usize = 32 * 1024;

const FORMAT_NAME: &str = "V3U32";
const VOCAB_FILENAME: &str = "vocab.bin";
const MERGES_FILENAME: &str = "merges.bin";
const PACKAGE_MANIFEST_FILENAME: &str = "package_manifest.json";
const MAX_ID_BYTES: usize = 128;
const MAX_LABEL_BYTES: usize = 256;
const MAX_STAGING_ATTEMPTS: u64 = 1_024;
const SHA256_BYTES: usize = 32;
const V3_HEADER_BYTES_U64: u64 = 128;
const V3_VOCAB_LENGTH_PREFIX_BYTES: u64 = 4;
const V3_MERGE_RECORD_BYTES: u64 = 12;

static NEXT_STAGING_ORDINAL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum DirectorySyncStatus {
    Synced,
    Unsupported,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct PublicationDurability {
    artifact_files_synced: bool,
    manifest_file_synced: bool,
    staging_directory: DirectorySyncStatus,
    parent_directory: DirectorySyncStatus,
}

impl PublicationDurability {
    pub const fn artifact_files_synced(&self) -> bool {
        self.artifact_files_synced
    }

    pub const fn manifest_file_synced(&self) -> bool {
        self.manifest_file_synced
    }

    pub const fn staging_directory(&self) -> DirectorySyncStatus {
        self.staging_directory
    }

    pub const fn parent_directory(&self) -> DirectorySyncStatus {
        self.parent_directory
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedPackage {
    destination: PathBuf,
    vocab_size: u32,
    merge_count: u64,
    vocab: ArtifactFileEvidenceInput,
    merges: ArtifactFileEvidenceInput,
    manifest_bytes: u64,
    manifest_sha256: [u8; 32],
    manifest_sha512: [u8; 64],
    durability: PublicationDurability,
}

impl PublishedPackage {
    pub fn destination(&self) -> &Path {
        &self.destination
    }

    pub fn vocab_relative_path(&self) -> &Path {
        Path::new(VOCAB_FILENAME)
    }

    pub fn merges_relative_path(&self) -> &Path {
        Path::new(MERGES_FILENAME)
    }

    pub fn manifest_relative_path(&self) -> &Path {
        Path::new(PACKAGE_MANIFEST_FILENAME)
    }

    pub const fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    pub const fn merge_count(&self) -> u64 {
        self.merge_count
    }

    pub const fn vocab_evidence(&self) -> ArtifactFileEvidenceInput {
        self.vocab
    }

    pub const fn merges_evidence(&self) -> ArtifactFileEvidenceInput {
        self.merges
    }

    pub const fn manifest_bytes(&self) -> u64 {
        self.manifest_bytes
    }

    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    pub const fn manifest_sha512(&self) -> [u8; 64] {
        self.manifest_sha512
    }

    pub const fn durability(&self) -> PublicationDurability {
        self.durability
    }
}

struct MeasuredArtifactPair {
    vocab: ArtifactHeaderV3,
    merges: ArtifactHeaderV3,
    total_vocab_bytes: u64,
    max_token_bytes: u32,
}

struct WrittenArtifactPair {
    vocab: ArtifactFileEvidenceInput,
    merges: ArtifactFileEvidenceInput,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PublicationPhase {
    StagingCreated,
    VocabWritten,
    MergesWritten,
    ReadbackCompleted,
    CompleteHashesVerified,
    ManifestWritten,
    ManifestReadbackCompleted,
    StagingDirectorySynced,
}

struct ArtifactStream {
    output: BufWriter<File>,
    file_sha256: Sha256,
    file_sha512: Sha512,
    payload_sha256: Sha256,
    file_bytes: u64,
    payload_bytes: u64,
}

impl ArtifactStream {
    fn create_new(path: &Path) -> Result<Self, ArtifactError> {
        let file = match OpenOptions::new().write(true).create_new(true).open(path) {
            Ok(file) => file,
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
                return Err(ArtifactError::ExistingDestination);
            }
            Err(error) => return Err(ArtifactError::Io(error)),
        };
        Ok(Self {
            output: BufWriter::new(file),
            file_sha256: Sha256::new(),
            file_sha512: Sha512::new(),
            payload_sha256: Sha256::new(),
            file_bytes: 0,
            payload_bytes: 0,
        })
    }

    fn write_header(&mut self, bytes: &[u8; V3_HEADER_BYTES]) -> Result<(), ArtifactError> {
        self.output.write_all(bytes)?;
        self.file_sha256.update(bytes);
        self.file_sha512.update(bytes);
        self.file_bytes = self.file_bytes.checked_add(V3_HEADER_BYTES_U64).ok_or(
            ArtifactError::ArithmeticOverflow {
                operation: "streamed file header length",
            },
        )?;
        Ok(())
    }

    fn write_payload(&mut self, bytes: &[u8]) -> Result<(), ArtifactError> {
        self.output.write_all(bytes)?;
        self.file_sha256.update(bytes);
        self.file_sha512.update(bytes);
        self.payload_sha256.update(bytes);
        let chunk_bytes =
            u64::try_from(bytes.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "streamed payload chunk length conversion",
            })?;
        self.file_bytes =
            self.file_bytes
                .checked_add(chunk_bytes)
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "streamed complete file length",
                })?;
        self.payload_bytes = self.payload_bytes.checked_add(chunk_bytes).ok_or(
            ArtifactError::ArithmeticOverflow {
                operation: "streamed payload length",
            },
        )?;
        Ok(())
    }

    fn finish(
        mut self,
        header: &ArtifactHeaderV3,
    ) -> Result<ArtifactFileEvidenceInput, ArtifactError> {
        if self.payload_bytes != header.payload_bytes() {
            return Err(ArtifactError::CountOutOfRange {
                field: "streamed payload_bytes",
            });
        }
        let actual_payload_sha256 = finalize_sha256(self.payload_sha256);
        if actual_payload_sha256 != header.payload_sha256() {
            return Err(ArtifactError::PayloadDigestMismatch);
        }
        let expected_file_bytes = V3_HEADER_BYTES_U64
            .checked_add(header.payload_bytes())
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "streamed header plus payload length",
            })?;
        if self.file_bytes != expected_file_bytes {
            return Err(ArtifactError::CountOutOfRange {
                field: "streamed file_bytes",
            });
        }
        self.output
            .flush()
            .map_err(|source| ArtifactError::DurabilityFailure {
                operation: "artifact file flush",
                source,
            })?;
        self.output
            .get_ref()
            .sync_all()
            .map_err(|source| ArtifactError::DurabilityFailure {
                operation: "artifact file sync_all",
                source,
            })?;

        Ok(ArtifactFileEvidenceInput {
            file_bytes: self.file_bytes,
            payload_bytes: self.payload_bytes,
            record_count: header.record_count(),
            file_sha256: finalize_sha256(self.file_sha256),
            file_sha512: finalize_sha512(self.file_sha512),
            payload_sha256: actual_payload_sha256,
            sequence_sha256: header.sequence_sha256(),
        })
    }
}

fn measure_v3_artifacts(tokenizer: &Tokenizer) -> Result<MeasuredArtifactPair, ArtifactError> {
    let vocab_size =
        u32::try_from(tokenizer.vocab_size()).map_err(|_| ArtifactError::CountOutOfRange {
            field: "writer vocab_size",
        })?;
    if !(BPE_TOKEN_START..=MAX_VOCAB_SIZE).contains(&vocab_size) {
        return Err(ArtifactError::CountOutOfRange {
            field: "writer vocab_size",
        });
    }
    let merge_count =
        u64::try_from(tokenizer.merge_count()).map_err(|_| ArtifactError::CountOutOfRange {
            field: "writer merge_count",
        })?;
    let expected_merge_count = u64::from(vocab_size - BPE_TOKEN_START);
    if merge_count != expected_merge_count {
        return Err(ArtifactError::CountOutOfRange {
            field: "writer merge_count",
        });
    }

    let mut merge_sha256 = Sha256::new();
    for ordinal in 0..tokenizer.merge_count() {
        let merge = tokenizer
            .merge_at(ordinal)
            .ok_or(ArtifactError::CountOutOfRange {
                field: "writer merge record",
            })?;
        let ordinal_u32 =
            u32::try_from(ordinal).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "writer merge ordinal conversion",
            })?;
        let expected_merged =
            BPE_TOKEN_START
                .checked_add(ordinal_u32)
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "writer sequential merge ID",
                })?;
        if merge.merged != expected_merged {
            return Err(ArtifactError::NonSequentialResult);
        }
        if merge.a >= merge.merged || merge.b >= merge.merged {
            return Err(ArtifactError::ForwardReference);
        }
        merge_sha256.update(merge.a.to_le_bytes());
        merge_sha256.update(merge.b.to_le_bytes());
        merge_sha256.update(merge.merged.to_le_bytes());
    }
    let merge_payload_bytes = merge_count.checked_mul(V3_MERGE_RECORD_BYTES).ok_or(
        ArtifactError::ArithmeticOverflow {
            operation: "writer merge payload length",
        },
    )?;
    let sequence_sha256 = finalize_sha256(merge_sha256);
    let merges = ArtifactHeaderV3::from_prehashed_merge_payload(
        vocab_size,
        merge_payload_bytes,
        sequence_sha256,
    )?;

    let mut vocab_payload_bytes = 0u64;
    let mut total_vocab_bytes = 0u64;
    let mut max_token_bytes = 0u32;
    let mut vocab_sha256 = Sha256::new();
    for token_id in 0..vocab_size {
        let token = tokenizer
            .token_bytes(token_id)
            .ok_or(ArtifactError::InvalidTokenId)?;
        let token_length =
            u32::try_from(token.len()).map_err(|_| ArtifactError::CountOutOfRange {
                field: "writer token byte length",
            })?;
        total_vocab_bytes = total_vocab_bytes
            .checked_add(u64::from(token_length))
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "writer total vocabulary bytes",
            })?;
        max_token_bytes = max_token_bytes.max(token_length);
        vocab_payload_bytes = vocab_payload_bytes
            .checked_add(V3_VOCAB_LENGTH_PREFIX_BYTES)
            .and_then(|length| length.checked_add(u64::from(token_length)))
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "writer vocab payload length",
            })?;
        vocab_sha256.update(token_length.to_le_bytes());
        vocab_sha256.update(token);
    }
    let vocab = ArtifactHeaderV3::from_prehashed_vocab_payload(
        vocab_size,
        vocab_payload_bytes,
        finalize_sha256(vocab_sha256),
        sequence_sha256,
    )?;
    Ok(MeasuredArtifactPair {
        vocab,
        merges,
        total_vocab_bytes,
        max_token_bytes,
    })
}

fn write_vocab_file_create_new(
    tokenizer: &Tokenizer,
    path: &Path,
    header: &ArtifactHeaderV3,
) -> Result<ArtifactFileEvidenceInput, ArtifactError> {
    let mut stream = ArtifactStream::create_new(path)?;
    stream.write_header(&header.to_bytes())?;
    for token_id in 0..header.vocab_size() {
        let token = tokenizer
            .token_bytes(token_id)
            .ok_or(ArtifactError::InvalidTokenId)?;
        let token_length =
            u32::try_from(token.len()).map_err(|_| ArtifactError::CountOutOfRange {
                field: "writer token byte length",
            })?;
        stream.write_payload(&token_length.to_le_bytes())?;
        stream.write_payload(token)?;
    }
    stream.finish(header)
}

fn write_merge_file_create_new(
    tokenizer: &Tokenizer,
    path: &Path,
    header: &ArtifactHeaderV3,
) -> Result<ArtifactFileEvidenceInput, ArtifactError> {
    let mut stream = ArtifactStream::create_new(path)?;
    stream.write_header(&header.to_bytes())?;
    for ordinal in 0..tokenizer.merge_count() {
        let merge = tokenizer
            .merge_at(ordinal)
            .ok_or(ArtifactError::CountOutOfRange {
                field: "writer merge record",
            })?;
        let mut record = [0u8; V3_MERGE_RECORD_BYTES as usize];
        record[0..4].copy_from_slice(&merge.a.to_le_bytes());
        record[4..8].copy_from_slice(&merge.b.to_le_bytes());
        record[8..12].copy_from_slice(&merge.merged.to_le_bytes());
        stream.write_payload(&record)?;
    }
    stream.finish(header)
}

fn finalize_sha256(hasher: Sha256) -> [u8; 32] {
    let digest = hasher.finalize();
    let mut output = [0u8; 32];
    output.copy_from_slice(&digest);
    output
}

fn finalize_sha512(hasher: Sha512) -> [u8; 64] {
    let digest = hasher.finalize();
    let mut output = [0u8; 64];
    output.copy_from_slice(&digest);
    output
}

pub fn write_v3_package(
    tokenizer: &Tokenizer,
    destination: &Path,
    context: &PublicationContext,
) -> Result<PublishedPackage, ArtifactError> {
    write_v3_package_with_hook(tokenizer, destination, context, |_, _, _| Ok(()))
}

fn write_v3_package_with_hook<F>(
    tokenizer: &Tokenizer,
    destination: &Path,
    context: &PublicationContext,
    mut phase_hook: F,
) -> Result<PublishedPackage, ArtifactError>
where
    F: FnMut(PublicationPhase, &Path, &Path) -> Result<(), ArtifactError>,
{
    let (parent, final_destination) = validate_publication_destination(destination)?;
    let measured = measure_v3_artifacts(tokenizer)?;
    let staging = create_staging_directory(&parent)?;
    phase_hook(
        PublicationPhase::StagingCreated,
        &staging,
        &final_destination,
    )?;

    let vocab_written =
        write_vocab_file_create_new(tokenizer, &staging.join(VOCAB_FILENAME), &measured.vocab)?;
    phase_hook(PublicationPhase::VocabWritten, &staging, &final_destination)?;

    let merges_written =
        write_merge_file_create_new(tokenizer, &staging.join(MERGES_FILENAME), &measured.merges)?;
    phase_hook(
        PublicationPhase::MergesWritten,
        &staging,
        &final_destination,
    )?;

    let limits = strict_readback_limits(&measured)?;
    let readback = crate::artifact::load_tokenizer_package(
        &staging.join(VOCAB_FILENAME),
        &staging.join(MERGES_FILENAME),
        crate::artifact::ArtifactFormat::V3U32,
        &limits,
    )
    .map_err(|error| durability_from_artifact_error("strict paired artifact readback", error))?;
    if &readback != tokenizer {
        return Err(ArtifactError::ReconstructedTokenMismatch);
    }
    phase_hook(
        PublicationPhase::ReadbackCompleted,
        &staging,
        &final_destination,
    )?;

    let written = WrittenArtifactPair {
        vocab: verify_complete_file_hashes(&staging.join(VOCAB_FILENAME), vocab_written)?,
        merges: verify_complete_file_hashes(&staging.join(MERGES_FILENAME), merges_written)?,
    };
    phase_hook(
        PublicationPhase::CompleteHashesVerified,
        &staging,
        &final_destination,
    )?;

    let manifest = ArtifactPackageManifestV1::try_from_input(ArtifactPackageManifestInput {
        context,
        vocab_size: measured.vocab.vocab_size(),
        merge_count: measured.merges.merge_count(),
        vocab: written.vocab,
        merges: written.merges,
    })?;
    let manifest_bytes = manifest.to_canonical_json()?;
    write_manifest_create_new(&staging.join(PACKAGE_MANIFEST_FILENAME), &manifest_bytes)?;
    phase_hook(
        PublicationPhase::ManifestWritten,
        &staging,
        &final_destination,
    )?;

    let manifest_readback = read_manifest_strict(&staging.join(PACKAGE_MANIFEST_FILENAME))?;
    if manifest_readback.bytes != manifest_bytes || manifest_readback.manifest != manifest {
        return Err(ArtifactError::InvalidPackageManifest {
            field: "strict readback equality",
        });
    }
    phase_hook(
        PublicationPhase::ManifestReadbackCompleted,
        &staging,
        &final_destination,
    )?;

    let staging_directory_sync = sync_directory_if_supported(&staging)?;
    phase_hook(
        PublicationPhase::StagingDirectorySynced,
        &staging,
        &final_destination,
    )?;
    rename_staging_no_replace(&staging, &final_destination)?;
    let parent_directory_sync = sync_directory_if_supported(&parent)?;

    Ok(PublishedPackage {
        destination: final_destination,
        vocab_size: measured.vocab.vocab_size(),
        merge_count: measured.merges.merge_count(),
        vocab: written.vocab,
        merges: written.merges,
        manifest_bytes: manifest_readback.file_bytes,
        manifest_sha256: manifest_readback.sha256,
        manifest_sha512: manifest_readback.sha512,
        durability: PublicationDurability {
            artifact_files_synced: true,
            manifest_file_synced: true,
            staging_directory: staging_directory_sync,
            parent_directory: parent_directory_sync,
        },
    })
}

/// Loads one manifest-bound V3U32 tokenizer package.
///
/// `package` must name either the package directory or its exact
/// `package_manifest.json` file. Direct artifact paths, format inference, and
/// fallback to headerless/V2 artifacts are intentionally unsupported.
pub fn load_v3_tokenizer_package(
    package: &Path,
    limits: &crate::artifact::ArtifactLimits,
) -> Result<Tokenizer, ArtifactError> {
    let manifest_path = resolve_package_manifest_path(package)?;
    let package_root = manifest_path
        .parent()
        .ok_or(ArtifactError::InvalidPublicationPath {
            field: "package manifest parent",
        })?;
    let vocab_path = package_root.join(VOCAB_FILENAME);
    let merges_path = package_root.join(MERGES_FILENAME);

    ensure_no_link_like_ancestors(&vocab_path)?;
    ensure_no_link_like_ancestors(&merges_path)?;
    let manifest_before = read_manifest_strict(&manifest_path)?;
    let vocab_expected = manifest_artifact_evidence(&manifest_before.manifest.wire.artifacts[0])?;
    let merges_expected = manifest_artifact_evidence(&manifest_before.manifest.wire.artifacts[1])?;
    enforce_manifest_file_limit(vocab_expected.file_bytes, limits)?;
    enforce_manifest_file_limit(merges_expected.file_bytes, limits)?;

    verify_complete_file_hashes(&vocab_path, vocab_expected)?;
    verify_complete_file_hashes(&merges_path, merges_expected)?;
    let tokenizer = crate::artifact::load_tokenizer_package(
        &vocab_path,
        &merges_path,
        crate::artifact::ArtifactFormat::V3U32,
        limits,
    )?;
    verify_complete_file_hashes(&vocab_path, vocab_expected)?;
    verify_complete_file_hashes(&merges_path, merges_expected)?;

    let tokenizer_vocab =
        u32::try_from(tokenizer.vocab_size()).map_err(|_| ArtifactError::CountOutOfRange {
            field: "loaded package vocab_size",
        })?;
    let tokenizer_merges =
        u64::try_from(tokenizer.merge_count()).map_err(|_| ArtifactError::CountOutOfRange {
            field: "loaded package merge_count",
        })?;
    if tokenizer_vocab != manifest_before.manifest.vocab_size()
        || tokenizer_merges != manifest_before.manifest.merge_count()
    {
        return Err(ArtifactError::InvalidPackageManifest {
            field: "loaded tokenizer counts",
        });
    }

    let manifest_after = read_manifest_strict(&manifest_path)?;
    if manifest_after.bytes != manifest_before.bytes
        || manifest_after.manifest != manifest_before.manifest
    {
        return Err(ArtifactError::InvalidPackageManifest {
            field: "manifest changed during package load",
        });
    }
    Ok(tokenizer)
}

struct ManifestFileReadback {
    manifest: ArtifactPackageManifestV1,
    bytes: Vec<u8>,
    file_bytes: u64,
    sha256: [u8; 32],
    sha512: [u8; 64],
}

fn resolve_package_manifest_path(package: &Path) -> Result<PathBuf, ArtifactError> {
    if package.as_os_str().is_empty() {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "package selection",
        });
    }
    let absolute = if package.is_absolute() {
        package.to_path_buf()
    } else {
        std::env::current_dir()?.join(package)
    };
    ensure_no_link_like_ancestors(&absolute)?;
    let canonical = fs::canonicalize(&absolute)?;
    ensure_no_link_like_ancestors(&canonical)?;
    let metadata = fs::symlink_metadata(&canonical)?;
    if metadata_is_link_like(&metadata) {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "package selection link type",
        });
    }

    let manifest_path = if metadata.is_dir() {
        canonical.join(PACKAGE_MANIFEST_FILENAME)
    } else if metadata.is_file()
        && canonical.file_name() == Some(std::ffi::OsStr::new(PACKAGE_MANIFEST_FILENAME))
    {
        canonical
    } else {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "package selection kind",
        });
    };
    ensure_no_link_like_ancestors(&manifest_path)?;
    ensure_regular_unlinked_file(&manifest_path)?;
    Ok(manifest_path)
}

fn enforce_manifest_file_limit(
    file_bytes: u64,
    limits: &crate::artifact::ArtifactLimits,
) -> Result<(), ArtifactError> {
    if file_bytes > limits.max_file_bytes {
        return Err(ArtifactError::ResourceLimitExceeded {
            resource: "package_artifact_file_bytes",
            limit: limits.max_file_bytes,
            actual: file_bytes,
        });
    }
    Ok(())
}

fn validate_publication_destination(
    destination: &Path,
) -> Result<(PathBuf, PathBuf), ArtifactError> {
    let filename = destination
        .file_name()
        .ok_or(ArtifactError::InvalidPublicationPath {
            field: "destination filename",
        })?;
    if filename.is_empty() || filename == "." || filename == ".." {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "destination filename",
        });
    }
    let parent_input = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_absolute = if parent_input.is_absolute() {
        parent_input.to_path_buf()
    } else {
        std::env::current_dir()?.join(parent_input)
    };
    ensure_no_link_like_ancestors(&parent_absolute)?;
    let parent = fs::canonicalize(&parent_absolute)?;
    ensure_no_link_like_ancestors(&parent)?;
    if !fs::metadata(&parent)?.is_dir() {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "destination parent directory",
        });
    }
    let final_destination = parent.join(filename);
    ensure_destination_absent(&final_destination)?;
    Ok((parent, final_destination))
}

fn ensure_destination_absent(path: &Path) -> Result<(), ArtifactError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(ArtifactError::ExistingDestination),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(ArtifactError::Io(error)),
    }
}

fn ensure_no_link_like_ancestors(path: &Path) -> Result<(), ArtifactError> {
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        let metadata = fs::symlink_metadata(ancestor)?;
        if metadata_is_link_like(&metadata) {
            return Err(ArtifactError::InvalidPublicationPath {
                field: "symlink or reparse ancestor",
            });
        }
    }
    Ok(())
}

#[cfg(windows)]
fn metadata_is_link_like(metadata: &Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0400;
    metadata.file_type().is_symlink()
        || metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_link_like(metadata: &Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn ensure_regular_unlinked_file(path: &Path) -> Result<Metadata, ArtifactError> {
    let metadata = fs::symlink_metadata(path)?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(ArtifactError::InvalidPublicationPath {
            field: "staged artifact file type",
        });
    }
    Ok(metadata)
}

fn create_staging_directory(parent: &Path) -> Result<PathBuf, ArtifactError> {
    for _ in 0..MAX_STAGING_ATTEMPTS {
        let ordinal = NEXT_STAGING_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let name = format!(".ashira-v3-staging-{}-{ordinal:016X}", std::process::id());
        let staging = parent.join(name);
        match fs::create_dir(&staging) {
            Ok(()) => {
                let metadata = fs::symlink_metadata(&staging)?;
                if metadata_is_link_like(&metadata) || !metadata.is_dir() {
                    return Err(ArtifactError::InvalidPublicationPath {
                        field: "staging directory type",
                    });
                }
                return Ok(staging);
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(ArtifactError::Io(error)),
        }
    }
    Err(ArtifactError::ResourceLimitExceeded {
        resource: "staging_directory_attempts",
        limit: MAX_STAGING_ATTEMPTS,
        actual: MAX_STAGING_ATTEMPTS,
    })
}

fn strict_readback_limits(
    measured: &MeasuredArtifactPair,
) -> Result<crate::artifact::ArtifactLimits, ArtifactError> {
    let vocab_file_bytes = V3_HEADER_BYTES_U64
        .checked_add(measured.vocab.payload_bytes())
        .ok_or(ArtifactError::ArithmeticOverflow {
            operation: "strict readback vocab file length",
        })?;
    let merge_file_bytes = V3_HEADER_BYTES_U64
        .checked_add(measured.merges.payload_bytes())
        .ok_or(ArtifactError::ArithmeticOverflow {
            operation: "strict readback merge file length",
        })?;
    Ok(crate::artifact::ArtifactLimits {
        max_file_bytes: vocab_file_bytes.max(merge_file_bytes),
        max_total_vocab_bytes: measured.total_vocab_bytes,
        max_token_bytes: measured.max_token_bytes,
    })
}

fn verify_complete_file_hashes(
    path: &Path,
    expected: ArtifactFileEvidenceInput,
) -> Result<ArtifactFileEvidenceInput, ArtifactError> {
    let metadata = ensure_regular_unlinked_file(path)?;
    if metadata.len() != expected.file_bytes {
        return Err(length_mismatch(expected.file_bytes, metadata.len()));
    }
    let file = File::open(path)?;
    let mut reader = file;
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut actual_bytes = 0u64;
    let mut buffer = [0u8; 8_192];
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        sha256.update(chunk);
        sha512.update(chunk);
        actual_bytes = actual_bytes
            .checked_add(
                u64::try_from(read).map_err(|_| ArtifactError::ArithmeticOverflow {
                    operation: "complete-file read length conversion",
                })?,
            )
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "complete-file read length",
            })?;
    }
    if actual_bytes != expected.file_bytes {
        return Err(length_mismatch(expected.file_bytes, actual_bytes));
    }
    let file_sha256 = finalize_sha256(sha256);
    let file_sha512 = finalize_sha512(sha512);
    if file_sha256 != expected.file_sha256 || file_sha512 != expected.file_sha512 {
        return Err(durability_invalid_data(
            "complete-file hash readback mismatch",
        ));
    }
    Ok(ArtifactFileEvidenceInput {
        file_sha256,
        file_sha512,
        ..expected
    })
}

fn length_mismatch(expected: u64, actual: u64) -> ArtifactError {
    if actual < expected {
        ArtifactError::Truncated {
            expected_bytes: expected,
            actual_bytes: actual,
        }
    } else {
        ArtifactError::TrailingData {
            expected_bytes: expected,
            actual_bytes: actual,
        }
    }
}

fn write_manifest_create_new(path: &Path, bytes: &[u8]) -> Result<(), ArtifactError> {
    let file = match OpenOptions::new().write(true).create_new(true).open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            return Err(ArtifactError::ExistingDestination);
        }
        Err(error) => return Err(ArtifactError::Io(error)),
    };
    let mut output = BufWriter::new(file);
    output.write_all(bytes)?;
    output
        .flush()
        .map_err(|source| ArtifactError::DurabilityFailure {
            operation: "package manifest flush",
            source,
        })?;
    output
        .get_ref()
        .sync_all()
        .map_err(|source| ArtifactError::DurabilityFailure {
            operation: "package manifest sync_all",
            source,
        })
}

fn read_manifest_strict(path: &Path) -> Result<ManifestFileReadback, ArtifactError> {
    let metadata = ensure_regular_unlinked_file(path)?;
    let limit = u64::try_from(MAX_PACKAGE_MANIFEST_BYTES).map_err(|_| {
        ArtifactError::ArithmeticOverflow {
            operation: "manifest readback limit conversion",
        }
    })?;
    if metadata.len() > limit {
        return Err(ArtifactError::ResourceLimitExceeded {
            resource: "package_manifest_readback",
            limit,
            actual: metadata.len(),
        });
    }
    let capacity =
        usize::try_from(metadata.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
            operation: "manifest readback capacity conversion",
        })?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(capacity)
        .map_err(|_| ArtifactError::ResourceLimitExceeded {
            resource: "package_manifest_readback",
            limit,
            actual: metadata.len(),
        })?;
    let file = File::open(path)?;
    file.take(
        limit
            .checked_add(1)
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "manifest readback take limit",
            })?,
    )
    .read_to_end(&mut bytes)?;
    let file_bytes = u64::try_from(bytes.len()).map_err(|_| ArtifactError::ArithmeticOverflow {
        operation: "manifest readback length conversion",
    })?;
    if file_bytes != metadata.len() {
        return Err(length_mismatch(metadata.len(), file_bytes));
    }
    let manifest = ArtifactPackageManifestV1::parse_canonical_json(&bytes)?;
    Ok(ManifestFileReadback {
        manifest,
        file_bytes,
        sha256: finalize_sha256(Sha256::new_with_prefix(&bytes)),
        sha512: finalize_sha512(Sha512::new_with_prefix(&bytes)),
        bytes,
    })
}

fn durability_from_artifact_error(operation: &'static str, error: ArtifactError) -> ArtifactError {
    let source = std::io::Error::new(std::io::ErrorKind::InvalidData, error);
    ArtifactError::DurabilityFailure { operation, source }
}

fn durability_invalid_data(operation: &'static str) -> ArtifactError {
    ArtifactError::DurabilityFailure {
        operation,
        source: std::io::Error::new(std::io::ErrorKind::InvalidData, operation),
    }
}

#[cfg(windows)]
fn sync_directory_if_supported(_path: &Path) -> Result<DirectorySyncStatus, ArtifactError> {
    Ok(DirectorySyncStatus::Unsupported)
}

#[cfg(unix)]
fn sync_directory_if_supported(path: &Path) -> Result<DirectorySyncStatus, ArtifactError> {
    let directory = File::open(path).map_err(|source| ArtifactError::DurabilityFailure {
        operation: "directory open for sync",
        source,
    })?;
    directory
        .sync_all()
        .map_err(|source| ArtifactError::DurabilityFailure {
            operation: "directory sync_all",
            source,
        })?;
    Ok(DirectorySyncStatus::Synced)
}

#[cfg(not(any(windows, unix)))]
fn sync_directory_if_supported(_path: &Path) -> Result<DirectorySyncStatus, ArtifactError> {
    Ok(DirectorySyncStatus::Unsupported)
}

#[cfg(windows)]
fn rename_staging_no_replace(staging: &Path, destination: &Path) -> Result<(), ArtifactError> {
    ensure_destination_absent(destination)?;
    match fs::rename(staging, destination) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            Err(ArtifactError::ExistingDestination)
        }
        Err(source) => Err(ArtifactError::DurabilityFailure {
            operation: "same-parent final directory rename",
            source,
        }),
    }
}

#[cfg(not(windows))]
fn rename_staging_no_replace(_staging: &Path, _destination: &Path) -> Result<(), ArtifactError> {
    Err(ArtifactError::DurabilityFailure {
        operation: "non-overwrite directory rename unsupported on this platform",
        source: std::io::Error::new(
            std::io::ErrorKind::Unsupported,
            "standard-library directory rename cannot prove no-replace semantics",
        ),
    })
}

#[derive(Clone, Copy, Debug)]
pub struct PublicationContextInput<'a> {
    pub run_id: &'a str,
    pub checkpoint_id: &'a str,
    pub parent_checkpoint_id: Option<&'a str>,
    pub deterministic_config_sha256: [u8; 32],
    pub corpus_manifest_sha256: [u8; 32],
    pub calibration_report_sha256: [u8; 32],
    pub probe_selection_sha256: [u8; 32],
    pub source_commit: &'a str,
    pub source_tree: &'a str,
    pub source_tracked_files_sha256: [u8; 32],
    pub writer_version: &'a str,
    pub toolchain_identity: &'a str,
    pub readback_evidence_id: &'a str,
    pub prefix_proof_evidence_id: &'a str,
    pub effective_backend: &'a str,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublicationContext {
    run_id: String,
    checkpoint_id: String,
    parent_checkpoint_id: Option<String>,
    deterministic_config_sha256: [u8; 32],
    corpus_manifest_sha256: [u8; 32],
    calibration_report_sha256: [u8; 32],
    probe_selection_sha256: [u8; 32],
    source_commit: String,
    source_tree: String,
    source_tracked_files_sha256: [u8; 32],
    writer_version: String,
    toolchain_identity: String,
    readback_evidence_id: String,
    prefix_proof_evidence_id: String,
    effective_backend: String,
}

impl PublicationContext {
    pub fn try_from_input(input: PublicationContextInput<'_>) -> Result<Self, ArtifactError> {
        validate_id("run_id", input.run_id)?;
        validate_id("checkpoint_id", input.checkpoint_id)?;
        if let Some(parent) = input.parent_checkpoint_id {
            validate_id("parent_checkpoint_id", parent)?;
            if parent == input.checkpoint_id {
                return Err(invalid_context("parent_checkpoint_id"));
            }
        }
        validate_digest(
            "deterministic_config_sha256",
            &input.deterministic_config_sha256,
        )?;
        validate_digest("corpus_manifest_sha256", &input.corpus_manifest_sha256)?;
        validate_digest(
            "calibration_report_sha256",
            &input.calibration_report_sha256,
        )?;
        validate_digest("probe_selection_sha256", &input.probe_selection_sha256)?;
        validate_git_object_id("source_commit", input.source_commit)?;
        validate_git_object_id("source_tree", input.source_tree)?;
        validate_digest(
            "source_tracked_files_sha256",
            &input.source_tracked_files_sha256,
        )?;
        validate_label("writer_version", input.writer_version)?;
        validate_label("toolchain_identity", input.toolchain_identity)?;
        validate_id("readback_evidence_id", input.readback_evidence_id)?;
        validate_id("prefix_proof_evidence_id", input.prefix_proof_evidence_id)?;
        validate_id("effective_backend", input.effective_backend)?;

        Ok(Self {
            run_id: input.run_id.to_owned(),
            checkpoint_id: input.checkpoint_id.to_owned(),
            parent_checkpoint_id: input.parent_checkpoint_id.map(str::to_owned),
            deterministic_config_sha256: input.deterministic_config_sha256,
            corpus_manifest_sha256: input.corpus_manifest_sha256,
            calibration_report_sha256: input.calibration_report_sha256,
            probe_selection_sha256: input.probe_selection_sha256,
            source_commit: input.source_commit.to_owned(),
            source_tree: input.source_tree.to_owned(),
            source_tracked_files_sha256: input.source_tracked_files_sha256,
            writer_version: input.writer_version.to_owned(),
            toolchain_identity: input.toolchain_identity.to_owned(),
            readback_evidence_id: input.readback_evidence_id.to_owned(),
            prefix_proof_evidence_id: input.prefix_proof_evidence_id.to_owned(),
            effective_backend: input.effective_backend.to_owned(),
        })
    }

    pub fn run_id(&self) -> &str {
        &self.run_id
    }

    pub fn checkpoint_id(&self) -> &str {
        &self.checkpoint_id
    }

    pub fn parent_checkpoint_id(&self) -> Option<&str> {
        self.parent_checkpoint_id.as_deref()
    }

    pub fn deterministic_config_sha256(&self) -> [u8; 32] {
        self.deterministic_config_sha256
    }

    pub fn corpus_manifest_sha256(&self) -> [u8; 32] {
        self.corpus_manifest_sha256
    }

    pub fn calibration_report_sha256(&self) -> [u8; 32] {
        self.calibration_report_sha256
    }

    pub fn probe_selection_sha256(&self) -> [u8; 32] {
        self.probe_selection_sha256
    }

    pub fn source_commit(&self) -> &str {
        &self.source_commit
    }

    pub fn source_tree(&self) -> &str {
        &self.source_tree
    }

    pub fn source_tracked_files_sha256(&self) -> [u8; 32] {
        self.source_tracked_files_sha256
    }

    pub fn writer_version(&self) -> &str {
        &self.writer_version
    }

    pub fn toolchain_identity(&self) -> &str {
        &self.toolchain_identity
    }

    pub fn readback_evidence_id(&self) -> &str {
        &self.readback_evidence_id
    }

    pub fn prefix_proof_evidence_id(&self) -> &str {
        &self.prefix_proof_evidence_id
    }

    pub fn effective_backend(&self) -> &str {
        &self.effective_backend
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ArtifactFileEvidenceInput {
    pub file_bytes: u64,
    pub payload_bytes: u64,
    pub record_count: u64,
    pub file_sha256: [u8; 32],
    pub file_sha512: [u8; 64],
    pub payload_sha256: [u8; 32],
    pub sequence_sha256: [u8; 32],
}

#[derive(Clone, Copy, Debug)]
pub struct ArtifactPackageManifestInput<'a> {
    pub context: &'a PublicationContext,
    pub vocab_size: u32,
    pub merge_count: u64,
    pub vocab: ArtifactFileEvidenceInput,
    pub merges: ArtifactFileEvidenceInput,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ArtifactPackageManifestV1 {
    wire: ManifestWire,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    schema: String,
    format: ManifestFormat,
    provenance: ManifestProvenance,
    counts: ManifestCounts,
    artifacts: [ManifestArtifact; 2],
    readback: ManifestReadback,
}

impl ArtifactPackageManifestV1 {
    pub fn try_from_input(input: ArtifactPackageManifestInput<'_>) -> Result<Self, ArtifactError> {
        validate_counts(input.vocab_size, input.merge_count)?;
        validate_artifact_input(
            "vocab",
            input.vocab,
            u64::from(input.vocab_size),
            ArtifactRole::Vocab,
        )?;
        validate_artifact_input(
            "merges",
            input.merges,
            input.merge_count,
            ArtifactRole::Merges,
        )?;
        if input.vocab.sequence_sha256 != input.merges.sequence_sha256
            || input.merges.payload_sha256 != input.merges.sequence_sha256
        {
            return Err(invalid_manifest("sequence_sha256"));
        }

        let manifest = Self {
            wire: ManifestWire {
                schema: PACKAGE_MANIFEST_SCHEMA.to_owned(),
                format: ManifestFormat {
                    name: FORMAT_NAME.to_owned(),
                    major: V3_FORMAT_MAJOR,
                    minor: V3_FORMAT_MINOR,
                    header_bytes: u16::try_from(V3_HEADER_BYTES).map_err(|_| {
                        ArtifactError::ArithmeticOverflow {
                            operation: "manifest header size conversion",
                        }
                    })?,
                },
                provenance: ManifestProvenance::from_context(input.context),
                counts: ManifestCounts {
                    base_vocab_count: BPE_TOKEN_START,
                    vocab_size: input.vocab_size,
                    merge_count: input.merge_count,
                },
                artifacts: [
                    ManifestArtifact::from_input(
                        VOCAB_FILENAME,
                        ManifestArtifactKind::Vocab,
                        input.vocab,
                    ),
                    ManifestArtifact::from_input(
                        MERGES_FILENAME,
                        ManifestArtifactKind::Merges,
                        input.merges,
                    ),
                ],
                readback: ManifestReadback {
                    explicit_format: FORMAT_NAME.to_owned(),
                    paired_validation: true,
                    source_tokenizer_equal: true,
                },
            },
        };
        manifest.validate()?;
        Ok(manifest)
    }

    pub fn parse_canonical_json(bytes: &[u8]) -> Result<Self, ArtifactError> {
        if bytes.len() > MAX_PACKAGE_MANIFEST_BYTES {
            return Err(ArtifactError::ResourceLimitExceeded {
                resource: "package_manifest_bytes",
                limit: u64::try_from(MAX_PACKAGE_MANIFEST_BYTES).map_err(|_| {
                    ArtifactError::ArithmeticOverflow {
                        operation: "manifest size limit conversion",
                    }
                })?,
                actual: u64::try_from(bytes.len()).map_err(|_| {
                    ArtifactError::ArithmeticOverflow {
                        operation: "manifest input length conversion",
                    }
                })?,
            });
        }
        let wire: ManifestWire =
            serde_json::from_slice(bytes).map_err(|_| invalid_manifest("canonical_json"))?;
        let manifest = Self { wire };
        manifest.validate()?;
        if manifest.to_canonical_json()?.as_slice() != bytes {
            return Err(invalid_manifest("canonical_json"));
        }
        Ok(manifest)
    }

    pub fn to_canonical_json(&self) -> Result<Vec<u8>, ArtifactError> {
        self.validate()?;
        let mut bytes = serde_json::to_vec(&self.wire).map_err(|_| invalid_manifest("json"))?;
        let final_bytes = bytes
            .len()
            .checked_add(1)
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "manifest terminal newline length",
            })?;
        let manifest_limit_u64 = u64::try_from(MAX_PACKAGE_MANIFEST_BYTES).map_err(|_| {
            ArtifactError::ArithmeticOverflow {
                operation: "manifest size limit conversion",
            }
        })?;
        let final_bytes_u64 =
            u64::try_from(final_bytes).map_err(|_| ArtifactError::ArithmeticOverflow {
                operation: "manifest serialized length conversion",
            })?;
        if final_bytes > MAX_PACKAGE_MANIFEST_BYTES {
            return Err(ArtifactError::ResourceLimitExceeded {
                resource: "package_manifest_serialization",
                limit: manifest_limit_u64,
                actual: final_bytes_u64,
            });
        }
        bytes
            .try_reserve_exact(1)
            .map_err(|_| ArtifactError::ResourceLimitExceeded {
                resource: "package_manifest_serialization",
                limit: manifest_limit_u64,
                actual: final_bytes_u64,
            })?;
        bytes.push(b'\n');
        Ok(bytes)
    }

    pub fn vocab_size(&self) -> u32 {
        self.wire.counts.vocab_size
    }

    pub fn merge_count(&self) -> u64 {
        self.wire.counts.merge_count
    }

    pub fn run_id(&self) -> &str {
        &self.wire.provenance.run_id
    }

    fn validate(&self) -> Result<(), ArtifactError> {
        let wire = &self.wire;
        if wire.schema != PACKAGE_MANIFEST_SCHEMA {
            return Err(invalid_manifest("schema"));
        }
        if wire.format.name != FORMAT_NAME
            || wire.format.major != V3_FORMAT_MAJOR
            || wire.format.minor != V3_FORMAT_MINOR
            || usize::from(wire.format.header_bytes) != V3_HEADER_BYTES
        {
            return Err(invalid_manifest("format"));
        }
        wire.provenance.validate()?;
        if wire.counts.base_vocab_count != BPE_TOKEN_START {
            return Err(invalid_manifest("base_vocab_count"));
        }
        validate_counts(wire.counts.vocab_size, wire.counts.merge_count)?;
        validate_manifest_artifact(
            &wire.artifacts[0],
            VOCAB_FILENAME,
            ManifestArtifactKind::Vocab,
            u64::from(wire.counts.vocab_size),
            ArtifactRole::Vocab,
        )?;
        validate_manifest_artifact(
            &wire.artifacts[1],
            MERGES_FILENAME,
            ManifestArtifactKind::Merges,
            wire.counts.merge_count,
            ArtifactRole::Merges,
        )?;
        if wire.artifacts[0].sequence_sha256 != wire.artifacts[1].sequence_sha256
            || wire.artifacts[1].payload_sha256 != wire.artifacts[1].sequence_sha256
        {
            return Err(invalid_manifest("sequence_sha256"));
        }
        if wire.readback.explicit_format != FORMAT_NAME
            || !wire.readback.paired_validation
            || !wire.readback.source_tokenizer_equal
        {
            return Err(invalid_manifest("readback"));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestFormat {
    name: String,
    major: u16,
    minor: u16,
    header_bytes: u16,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestProvenance {
    run_id: String,
    checkpoint_id: String,
    parent_checkpoint_id: Option<String>,
    deterministic_config_sha256: String,
    corpus_manifest_sha256: String,
    calibration_report_sha256: String,
    probe_selection_sha256: String,
    source_commit: String,
    source_tree: String,
    source_tracked_files_sha256: String,
    writer_version: String,
    toolchain_identity: String,
    readback_evidence_id: String,
    prefix_proof_evidence_id: String,
    effective_backend: String,
}

impl ManifestProvenance {
    fn from_context(context: &PublicationContext) -> Self {
        Self {
            run_id: context.run_id.clone(),
            checkpoint_id: context.checkpoint_id.clone(),
            parent_checkpoint_id: context.parent_checkpoint_id.clone(),
            deterministic_config_sha256: hex_upper(&context.deterministic_config_sha256),
            corpus_manifest_sha256: hex_upper(&context.corpus_manifest_sha256),
            calibration_report_sha256: hex_upper(&context.calibration_report_sha256),
            probe_selection_sha256: hex_upper(&context.probe_selection_sha256),
            source_commit: context.source_commit.clone(),
            source_tree: context.source_tree.clone(),
            source_tracked_files_sha256: hex_upper(&context.source_tracked_files_sha256),
            writer_version: context.writer_version.clone(),
            toolchain_identity: context.toolchain_identity.clone(),
            readback_evidence_id: context.readback_evidence_id.clone(),
            prefix_proof_evidence_id: context.prefix_proof_evidence_id.clone(),
            effective_backend: context.effective_backend.clone(),
        }
    }

    fn validate(&self) -> Result<(), ArtifactError> {
        validate_id_manifest("run_id", &self.run_id)?;
        validate_id_manifest("checkpoint_id", &self.checkpoint_id)?;
        if let Some(parent) = &self.parent_checkpoint_id {
            validate_id_manifest("parent_checkpoint_id", parent)?;
            if parent == &self.checkpoint_id {
                return Err(invalid_manifest("parent_checkpoint_id"));
            }
        }
        validate_upper_hex_manifest(
            "deterministic_config_sha256",
            &self.deterministic_config_sha256,
            SHA256_BYTES,
        )?;
        validate_upper_hex_manifest(
            "corpus_manifest_sha256",
            &self.corpus_manifest_sha256,
            SHA256_BYTES,
        )?;
        validate_upper_hex_manifest(
            "calibration_report_sha256",
            &self.calibration_report_sha256,
            SHA256_BYTES,
        )?;
        validate_upper_hex_manifest(
            "probe_selection_sha256",
            &self.probe_selection_sha256,
            SHA256_BYTES,
        )?;
        validate_git_object_id_manifest("source_commit", &self.source_commit)?;
        validate_git_object_id_manifest("source_tree", &self.source_tree)?;
        validate_upper_hex_manifest(
            "source_tracked_files_sha256",
            &self.source_tracked_files_sha256,
            SHA256_BYTES,
        )?;
        validate_label_manifest("writer_version", &self.writer_version)?;
        validate_label_manifest("toolchain_identity", &self.toolchain_identity)?;
        validate_id_manifest("readback_evidence_id", &self.readback_evidence_id)?;
        validate_id_manifest("prefix_proof_evidence_id", &self.prefix_proof_evidence_id)?;
        validate_id_manifest("effective_backend", &self.effective_backend)
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestCounts {
    base_vocab_count: u32,
    vocab_size: u32,
    merge_count: u64,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
enum ManifestArtifactKind {
    Vocab,
    Merges,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestArtifact {
    relative_filename: String,
    artifact_kind: ManifestArtifactKind,
    file_bytes: u64,
    payload_bytes: u64,
    record_count: u64,
    file_sha256: String,
    file_sha512: String,
    payload_sha256: String,
    sequence_sha256: String,
}

impl ManifestArtifact {
    fn from_input(
        relative_filename: &str,
        artifact_kind: ManifestArtifactKind,
        input: ArtifactFileEvidenceInput,
    ) -> Self {
        Self {
            relative_filename: relative_filename.to_owned(),
            artifact_kind,
            file_bytes: input.file_bytes,
            payload_bytes: input.payload_bytes,
            record_count: input.record_count,
            file_sha256: hex_upper(&input.file_sha256),
            file_sha512: hex_upper(&input.file_sha512),
            payload_sha256: hex_upper(&input.payload_sha256),
            sequence_sha256: hex_upper(&input.sequence_sha256),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestReadback {
    explicit_format: String,
    paired_validation: bool,
    source_tokenizer_equal: bool,
}

#[derive(Clone, Copy)]
enum ArtifactRole {
    Vocab,
    Merges,
}

fn validate_counts(vocab_size: u32, merge_count: u64) -> Result<(), ArtifactError> {
    if !(BPE_TOKEN_START..=MAX_VOCAB_SIZE).contains(&vocab_size)
        || merge_count != u64::from(vocab_size - BPE_TOKEN_START)
    {
        return Err(invalid_manifest("counts"));
    }
    Ok(())
}

fn validate_artifact_input(
    field: &'static str,
    input: ArtifactFileEvidenceInput,
    expected_records: u64,
    role: ArtifactRole,
) -> Result<(), ArtifactError> {
    if input.record_count != expected_records {
        return Err(invalid_manifest(field));
    }
    let expected_file_bytes = V3_HEADER_BYTES_U64.checked_add(input.payload_bytes).ok_or(
        ArtifactError::ArithmeticOverflow {
            operation: "manifest artifact file length",
        },
    )?;
    if input.file_bytes != expected_file_bytes {
        return Err(invalid_manifest(field));
    }
    match role {
        ArtifactRole::Vocab => {
            let minimum_payload = input
                .record_count
                .checked_mul(V3_VOCAB_LENGTH_PREFIX_BYTES)
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "manifest vocab minimum payload",
                })?;
            if input.payload_bytes < minimum_payload {
                return Err(invalid_manifest(field));
            }
        }
        ArtifactRole::Merges => {
            let expected_payload = input
                .record_count
                .checked_mul(V3_MERGE_RECORD_BYTES)
                .ok_or(ArtifactError::ArithmeticOverflow {
                    operation: "manifest merge payload",
                })?;
            if input.payload_bytes != expected_payload {
                return Err(invalid_manifest(field));
            }
        }
    }
    for digest in [
        input.file_sha256.as_slice(),
        input.file_sha512.as_slice(),
        input.payload_sha256.as_slice(),
        input.sequence_sha256.as_slice(),
    ] {
        if digest.iter().all(|byte| *byte == 0) {
            return Err(invalid_manifest(field));
        }
    }
    Ok(())
}

fn validate_manifest_artifact(
    artifact: &ManifestArtifact,
    filename: &str,
    kind: ManifestArtifactKind,
    expected_records: u64,
    role: ArtifactRole,
) -> Result<(), ArtifactError> {
    if artifact.relative_filename != filename
        || artifact.artifact_kind != kind
        || artifact.record_count != expected_records
    {
        return Err(invalid_manifest("artifacts"));
    }
    let input = manifest_artifact_evidence(artifact)?;
    validate_artifact_input("artifacts", input, expected_records, role)
}

fn manifest_artifact_evidence(
    artifact: &ManifestArtifact,
) -> Result<ArtifactFileEvidenceInput, ArtifactError> {
    Ok(ArtifactFileEvidenceInput {
        file_bytes: artifact.file_bytes,
        payload_bytes: artifact.payload_bytes,
        record_count: artifact.record_count,
        file_sha256: decode_upper_hex::<32>("file_sha256", &artifact.file_sha256)?,
        file_sha512: decode_upper_hex::<64>("file_sha512", &artifact.file_sha512)?,
        payload_sha256: decode_upper_hex::<32>("payload_sha256", &artifact.payload_sha256)?,
        sequence_sha256: decode_upper_hex::<32>("sequence_sha256", &artifact.sequence_sha256)?,
    })
}

fn validate_id(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    if value.is_empty()
        || value.len() > MAX_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(invalid_context(field));
    }
    Ok(())
}

fn validate_label(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    if value.is_empty()
        || value.len() > MAX_LABEL_BYTES
        || value.trim() != value
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_graphic() || byte == b' ')
        || value.contains(['/', '\\', '"', ':'])
    {
        return Err(invalid_context(field));
    }
    Ok(())
}

fn validate_git_object_id(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    if !matches!(value.len(), 40 | 64)
        || value.bytes().all(|byte| byte == b'0')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'a'..=b'f'))
    {
        return Err(invalid_context(field));
    }
    Ok(())
}

fn validate_digest(field: &'static str, digest: &[u8; 32]) -> Result<(), ArtifactError> {
    if digest.iter().all(|byte| *byte == 0) {
        return Err(invalid_context(field));
    }
    Ok(())
}

fn validate_id_manifest(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    validate_id(field, value).map_err(|_| invalid_manifest(field))
}

fn validate_label_manifest(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    validate_label(field, value).map_err(|_| invalid_manifest(field))
}

fn validate_git_object_id_manifest(field: &'static str, value: &str) -> Result<(), ArtifactError> {
    validate_git_object_id(field, value).map_err(|_| invalid_manifest(field))
}

fn validate_upper_hex_manifest(
    field: &'static str,
    value: &str,
    expected_bytes: usize,
) -> Result<(), ArtifactError> {
    let expected_hex_bytes =
        expected_bytes
            .checked_mul(2)
            .ok_or(ArtifactError::ArithmeticOverflow {
                operation: "manifest digest hex length",
            })?;
    if value.len() != expected_hex_bytes
        || value.bytes().all(|byte| byte == b'0')
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || matches!(byte, b'A'..=b'F'))
    {
        return Err(invalid_manifest(field));
    }
    Ok(())
}

fn decode_upper_hex<const N: usize>(
    field: &'static str,
    value: &str,
) -> Result<[u8; N], ArtifactError> {
    validate_upper_hex_manifest(field, value, N)?;
    let mut decoded = [0u8; N];
    for (output, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        *output = (hex_nibble(pair[0])? << 4) | hex_nibble(pair[1])?;
    }
    Ok(decoded)
}

fn hex_nibble(value: u8) -> Result<u8, ArtifactError> {
    match value {
        b'0'..=b'9' => Ok(value - b'0'),
        b'A'..=b'F' => Ok(value - b'A' + 10),
        _ => Err(invalid_manifest("digest_hex")),
    }
}

fn hex_upper(bytes: &[u8]) -> String {
    const HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(HEX[usize::from(byte >> 4)]));
        output.push(char::from(HEX[usize::from(byte & 0x0F)]));
    }
    output
}

const fn invalid_context(field: &'static str) -> ArtifactError {
    ArtifactError::InvalidPublicationContext { field }
}

const fn invalid_manifest(field: &'static str) -> ArtifactError {
    ArtifactError::InvalidPackageManifest { field }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::artifact::{
        ArtifactFormat, ArtifactKind, ArtifactLimits, inspect_artifact, load_tokenizer_package,
    };
    use sha2::{Digest, Sha256, Sha512};
    use std::fs;
    use std::io::{Seek, SeekFrom};
    use std::sync::atomic::{AtomicU64, Ordering};

    const SOURCE_COMMIT: &str = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa";
    const SOURCE_TREE: &str = "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb";

    fn unique_test_directory(label: &str) -> std::path::PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ashira_v3_publication_{}_{}_{}",
            std::process::id(),
            ordinal,
            label
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn artifact_limits() -> ArtifactLimits {
        ArtifactLimits {
            max_file_bytes: 10_000_000,
            max_total_vocab_bytes: 20_000_000,
            max_token_bytes: 1_000_000,
        }
    }

    fn canonical_v2_tokenizer() -> Tokenizer {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("runs/full_32768");
        load_tokenizer_package(
            &root.join(VOCAB_FILENAME),
            &root.join(MERGES_FILENAME),
            ArtifactFormat::V2U16,
            &artifact_limits(),
        )
        .unwrap()
    }

    fn base_tokenizer() -> Tokenizer {
        crate::TokenizerTrainer::new().freeze().unwrap()
    }

    struct IndependentBoundaryVector {
        name: &'static str,
        merge_count: u32,
        locked_one_merge: bool,
        vocab_bytes: usize,
        vocab_sha256: &'static str,
        vocab_sha512: &'static str,
        merges_bytes: usize,
        merges_sha256: &'static str,
        merges_sha512: &'static str,
    }

    struct IndependentArtifactFiles {
        vocab: Vec<u8>,
        merges: Vec<u8>,
    }

    const BOUNDARY_VECTORS: [IndependentBoundaryVector; 3] = [
        IndependentBoundaryVector {
            name: "empty",
            merge_count: 0,
            locked_one_merge: false,
            vocab_bytes: 1_657,
            vocab_sha256: "E292C14845AE1E00CA7741F2A2EE3D5397B23C1DEFB0803B045CF9BDFB6BAC7A",
            vocab_sha512: "BFCEF726CB4A02D5469EA4814553892A063AF1B760993093EE9D0D497FE2950BB0BA09D0E0D694D889507F99A17FB9C4B24B9ED2B8754F30918DE45322B237D6",
            merges_bytes: 128,
            merges_sha256: "EC29DC6DC65F6B1E6D39860912AAB209E374CD8C273BC2EBF5C90C78150CCC29",
            merges_sha512: "E2719D712FDCA1B17218E7EB25AE56AE7306A847E6BB1C30DB7F913DA678BADBB0EC7622793DEAB7872A2150C31923C51E54E42883E16BFAB312B3EA34182CB3",
        },
        IndependentBoundaryVector {
            name: "one",
            merge_count: 1,
            locked_one_merge: true,
            vocab_bytes: 1_663,
            vocab_sha256: "346E4C5DCE48638FCFA97F190B352EDA1508E02509A005E7D068440843525A40",
            vocab_sha512: "B95FF63801F57F89EF867FE7E29B575C747CDDEBD7F76A7772D3DEE3FC91FEF3855C38C755E9F8AE3ABEC6D024F0AE59367A9F87BB853C0D3F3459516F433639",
            merges_bytes: 140,
            merges_sha256: "A961B5548778197EC03633D590DB97B56EDA8150FFC113AB1F3A59EC89ACA163",
            merges_sha512: "8B060DE45E21AF8E6855B308F8274B4655F9190A5DE4CF8E86C7220006081A675F3A290813FB0FDE318B5DD00A8FCACBC45038118A6F17678C01357157688E3E",
        },
        IndependentBoundaryVector {
            name: "cross_u16",
            merge_count: 65_261,
            locked_one_merge: false,
            vocab_bytes: 393_223,
            vocab_sha256: "061C481B9A8EADA4C68B7487DBFB74739FEE3ADBC72859AA344FD30D3BA4999D",
            vocab_sha512: "ADF90D9DA9F40758E683B653DFDCF04600EBBE7CA7A09049CDD86E2C362D49DE5A1BA38DBB72FA0BFD1E711EE42008F8707E356CF11148D1B63201D993C410D6",
            merges_bytes: 783_260,
            merges_sha256: "FF5D9A11E001BCE072B9192E3801C798813AB72A0272D270FE582D96E8359644",
            merges_sha512: "4C44CC44F81108F1329B16055B62D4CD63DD6FB6E6F998689B7CAAE7E130E535A015C5131FEA5EDACFCE748A52D4C53D9BCCBF66C03BCE361E900975E415F86D",
        },
    ];

    fn independent_boundary_artifacts(
        merge_count: u32,
        locked_one_merge: bool,
    ) -> IndependentArtifactFiles {
        let specials: [&[u8]; 20] = [
            b"<PAD>",
            b"<UNK>",
            b"<BOS>",
            b"<EOS>",
            b"<kareem_narration>",
            b"<dylan_thinking>",
            b"<DYLAN>",
            b"<DYLAN_ADVERSARIAL>",
            b"<BLU>",
            b"<ECHO>",
            b"<RESONANCE>",
            b"<AI>",
            b"<PHIL>",
            b"<SYM>",
            b"<REFLECTION>",
            b"<CAIROS>",
            b"[[/ANCHOR]]",
            b"[[/CSA]]",
            b"<science_doc>",
            b"",
        ];
        let mut vocab_payload = Vec::new();
        for token in specials {
            push_independent_vocab_record(&mut vocab_payload, token);
        }
        for byte in u8::MIN..=u8::MAX {
            push_independent_vocab_record(&mut vocab_payload, &[byte]);
        }

        let mut merge_payload = Vec::new();
        for ordinal in 0..merge_count {
            let (a, b) = if locked_one_merge {
                (52u32, 136u32)
            } else {
                (20 + ordinal / 256, 20 + ordinal % 256)
            };
            let merged = 276 + ordinal;
            push_independent_u32(&mut merge_payload, a);
            push_independent_u32(&mut merge_payload, b);
            push_independent_u32(&mut merge_payload, merged);
            let token = [u8::try_from(a - 20).unwrap(), u8::try_from(b - 20).unwrap()];
            push_independent_vocab_record(&mut vocab_payload, &token);
        }

        let sequence_sha256 = digest_sha256(&merge_payload);
        let vocab_size = 276 + merge_count;
        IndependentArtifactFiles {
            vocab: independent_v3_file(
                1,
                0,
                u64::from(vocab_size),
                vocab_size,
                u64::from(merge_count),
                &vocab_payload,
                sequence_sha256,
            ),
            merges: independent_v3_file(
                2,
                12,
                u64::from(merge_count),
                vocab_size,
                u64::from(merge_count),
                &merge_payload,
                sequence_sha256,
            ),
        }
    }

    fn independent_v3_file(
        kind: u8,
        fixed_record_bytes: u8,
        record_count: u64,
        vocab_size: u32,
        merge_count: u64,
        payload: &[u8],
        sequence_sha256: [u8; 32],
    ) -> Vec<u8> {
        let payload_bytes = u64::try_from(payload.len()).unwrap();
        let payload_sha256 = digest_sha256(payload);
        let mut header = [0u8; 128];
        header[0..8].copy_from_slice(b"ASHIRA3\0");
        header[8..10].copy_from_slice(&3u16.to_le_bytes());
        header[10..12].copy_from_slice(&0u16.to_le_bytes());
        header[12] = kind;
        header[13] = 1;
        header[14] = 4;
        header[15] = fixed_record_bytes;
        header[16..18].copy_from_slice(&128u16.to_le_bytes());
        header[24..32].copy_from_slice(&record_count.to_le_bytes());
        header[32..40].copy_from_slice(&payload_bytes.to_le_bytes());
        header[40..44].copy_from_slice(&276u32.to_le_bytes());
        header[44..48].copy_from_slice(&vocab_size.to_le_bytes());
        header[48..56].copy_from_slice(&merge_count.to_le_bytes());
        header[64..96].copy_from_slice(&payload_sha256);
        header[96..128].copy_from_slice(&sequence_sha256);
        let mut file = Vec::with_capacity(128 + payload.len());
        file.extend_from_slice(&header);
        file.extend_from_slice(payload);
        file
    }

    fn push_independent_vocab_record(output: &mut Vec<u8>, token: &[u8]) {
        push_independent_u32(output, u32::try_from(token.len()).unwrap());
        output.extend_from_slice(token);
    }

    fn push_independent_u32(output: &mut Vec<u8>, value: u32) {
        output.extend_from_slice(&value.to_le_bytes());
    }

    fn digest_sha256(bytes: &[u8]) -> [u8; 32] {
        let digest = Sha256::digest(bytes);
        let mut output = [0u8; 32];
        output.copy_from_slice(&digest);
        output
    }

    fn write_independent_test_file(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .unwrap();
        file.write_all(bytes).unwrap();
        file.flush().unwrap();
        file.sync_all().unwrap();
    }

    fn context_input() -> PublicationContextInput<'static> {
        PublicationContextInput {
            run_id: "bounded-run-a",
            checkpoint_id: "checkpoint-000001",
            parent_checkpoint_id: Some("checkpoint-base"),
            deterministic_config_sha256: [0x11; 32],
            corpus_manifest_sha256: [0x22; 32],
            calibration_report_sha256: [0x33; 32],
            probe_selection_sha256: [0x44; 32],
            source_commit: SOURCE_COMMIT,
            source_tree: SOURCE_TREE,
            source_tracked_files_sha256: [0x55; 32],
            writer_version: "ashira-tokenizer-v3 0.1.0",
            toolchain_identity: "rustc 1.94.0 (4a4ef493e 2026-03-02)",
            readback_evidence_id: "bounded-readback-v1",
            prefix_proof_evidence_id: "bounded-prefix-proof-v1",
            effective_backend: "cpu",
        }
    }

    fn valid_context() -> PublicationContext {
        PublicationContext::try_from_input(context_input()).unwrap()
    }

    fn manifest_input(context: &PublicationContext) -> ArtifactPackageManifestInput<'_> {
        let sequence = [0xA1; 32];
        ArtifactPackageManifestInput {
            context,
            vocab_size: 277,
            merge_count: 1,
            vocab: ArtifactFileEvidenceInput {
                file_bytes: 2_128,
                payload_bytes: 2_000,
                record_count: 277,
                file_sha256: [0xA2; 32],
                file_sha512: [0xA3; 64],
                payload_sha256: [0xAB; 32],
                sequence_sha256: sequence,
            },
            merges: ArtifactFileEvidenceInput {
                file_bytes: 140,
                payload_bytes: 12,
                record_count: 1,
                file_sha256: [0xA4; 32],
                file_sha512: [0xA5; 64],
                payload_sha256: sequence,
                sequence_sha256: sequence,
            },
        }
    }

    #[test]
    fn publication_context_accepts_only_explicit_bounded_values() {
        let context = valid_context();
        assert_eq!(context.run_id(), "bounded-run-a");
        assert_eq!(context.checkpoint_id(), "checkpoint-000001");
        assert_eq!(context.parent_checkpoint_id(), Some("checkpoint-base"));
        assert_eq!(context.deterministic_config_sha256(), [0x11; 32]);
        assert_eq!(context.corpus_manifest_sha256(), [0x22; 32]);
        assert_eq!(context.calibration_report_sha256(), [0x33; 32]);
        assert_eq!(context.probe_selection_sha256(), [0x44; 32]);
        assert_eq!(context.source_commit(), SOURCE_COMMIT);
        assert_eq!(context.source_tree(), SOURCE_TREE);
        assert_eq!(context.source_tracked_files_sha256(), [0x55; 32]);
        assert_eq!(context.writer_version(), "ashira-tokenizer-v3 0.1.0");
        assert_eq!(
            context.toolchain_identity(),
            "rustc 1.94.0 (4a4ef493e 2026-03-02)"
        );
        assert_eq!(context.readback_evidence_id(), "bounded-readback-v1");
        assert_eq!(
            context.prefix_proof_evidence_id(),
            "bounded-prefix-proof-v1"
        );
        assert_eq!(context.effective_backend(), "cpu");
    }

    #[test]
    fn publication_context_rejects_hidden_or_fabricated_authority() {
        let mut input = context_input();
        input.run_id = "../escape";
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );

        let mut input = context_input();
        input.deterministic_config_sha256 = [0; 32];
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );

        let mut input = context_input();
        input.source_commit = "AAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAAA";
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );

        let mut input = context_input();
        input.source_commit = "0000000000000000000000000000000000000000";
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );

        let mut input = context_input();
        input.toolchain_identity = r"C:\toolchain\rustc";
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );

        let mut input = context_input();
        input.parent_checkpoint_id = Some(input.checkpoint_id);
        assert_eq!(
            PublicationContext::try_from_input(input)
                .unwrap_err()
                .class(),
            "InvalidPublicationContext"
        );
    }

    #[test]
    fn package_manifest_is_canonical_round_trippable_and_ordered() {
        let context = valid_context();
        let manifest = ArtifactPackageManifestV1::try_from_input(manifest_input(&context)).unwrap();
        let bytes_a = manifest.to_canonical_json().unwrap();
        let bytes_b = manifest.to_canonical_json().unwrap();
        assert_eq!(bytes_a, bytes_b);
        assert_eq!(bytes_a.len(), 2_245);
        assert_eq!(
            hex_upper(&Sha256::digest(&bytes_a)),
            "1C0B117DA9B0BB1EAD64392F807B0FDBEAD11002B2628BB80AD83BBFA9C95E99"
        );
        assert_eq!(bytes_a.last(), Some(&b'\n'));
        assert!(!bytes_a[..bytes_a.len() - 1].contains(&b'\n'));
        assert!(!bytes_a.contains(&b'\r'));
        assert_eq!(
            ArtifactPackageManifestV1::parse_canonical_json(&bytes_a).unwrap(),
            manifest
        );
        assert_eq!(manifest.vocab_size(), 277);
        assert_eq!(manifest.merge_count(), 1);
        assert_eq!(manifest.run_id(), "bounded-run-a");

        let text = std::str::from_utf8(&bytes_a).unwrap();
        let vocab_position = text.find("\"relative_filename\":\"vocab.bin\"").unwrap();
        let merge_position = text.find("\"relative_filename\":\"merges.bin\"").unwrap();
        assert!(vocab_position < merge_position);
        assert!(!text.contains("target/"));
        assert!(!text.contains("C:\\"));
    }

    #[test]
    fn package_manifest_rejects_noncanonical_or_inconsistent_bytes() {
        let context = valid_context();
        let manifest = ArtifactPackageManifestV1::try_from_input(manifest_input(&context)).unwrap();
        let canonical = manifest.to_canonical_json().unwrap();

        let oversized = vec![b' '; MAX_PACKAGE_MANIFEST_BYTES + 1];
        assert_eq!(
            ArtifactPackageManifestV1::parse_canonical_json(&oversized)
                .unwrap_err()
                .class(),
            "ResourceLimitExceeded"
        );

        let mut whitespace = canonical.clone();
        whitespace.insert(1, b' ');
        assert_eq!(
            ArtifactPackageManifestV1::parse_canonical_json(&whitespace)
                .unwrap_err()
                .class(),
            "InvalidPackageManifest"
        );

        let mut unknown = canonical[..canonical.len() - 2].to_vec();
        unknown.extend_from_slice(b",\"unexpected\":true}\n");
        assert_eq!(
            ArtifactPackageManifestV1::parse_canonical_json(&unknown)
                .unwrap_err()
                .class(),
            "InvalidPackageManifest"
        );

        let mut lowercase = canonical.clone();
        let digest = lowercase
            .windows(4)
            .position(|window| window == b"ABAB")
            .unwrap();
        lowercase[digest] = b'a';
        assert_eq!(
            ArtifactPackageManifestV1::parse_canonical_json(&lowercase)
                .unwrap_err()
                .class(),
            "InvalidPackageManifest"
        );

        let mut bad_sequence = manifest_input(&context);
        bad_sequence.vocab.sequence_sha256 = [0xB1; 32];
        assert_eq!(
            ArtifactPackageManifestV1::try_from_input(bad_sequence)
                .unwrap_err()
                .class(),
            "InvalidPackageManifest"
        );

        let mut bad_length = manifest_input(&context);
        bad_length.merges.file_bytes = 141;
        assert_eq!(
            ArtifactPackageManifestV1::try_from_input(bad_length)
                .unwrap_err()
                .class(),
            "InvalidPackageManifest"
        );
    }

    #[test]
    fn streaming_artifact_core_is_v3_strict_complete_hashed_and_create_new() {
        let tokenizer = canonical_v2_tokenizer();
        let staging = unique_test_directory("streaming_core");
        let measured = measure_v3_artifacts(&tokenizer).unwrap();
        let written = WrittenArtifactPair {
            vocab: write_vocab_file_create_new(
                &tokenizer,
                &staging.join(VOCAB_FILENAME),
                &measured.vocab,
            )
            .unwrap(),
            merges: write_merge_file_create_new(
                &tokenizer,
                &staging.join(MERGES_FILENAME),
                &measured.merges,
            )
            .unwrap(),
        };
        let vocab_path = staging.join(VOCAB_FILENAME);
        let merges_path = staging.join(MERGES_FILENAME);

        let vocab = inspect_artifact(
            &vocab_path,
            ArtifactFormat::V3U32,
            ArtifactKind::Vocab,
            &artifact_limits(),
        )
        .unwrap();
        let merges = inspect_artifact(
            &merges_path,
            ArtifactFormat::V3U32,
            ArtifactKind::Merges,
            &artifact_limits(),
        )
        .unwrap();
        assert_eq!(vocab.vocab_size, 32_768);
        assert_eq!(vocab.merge_count, 32_492);
        assert_eq!(vocab.payload_sha256, Some(written.vocab.payload_sha256));
        assert_eq!(vocab.sequence_sha256, Some(written.vocab.sequence_sha256));
        assert_eq!(merges.payload_sha256, Some(written.merges.payload_sha256));
        assert_eq!(merges.sequence_sha256, Some(written.merges.sequence_sha256));
        assert_eq!(
            written.vocab.sequence_sha256,
            written.merges.sequence_sha256
        );
        assert_eq!(
            written.merges.payload_sha256,
            written.merges.sequence_sha256
        );

        let vocab_bytes = fs::read(&vocab_path).unwrap();
        let merge_bytes = fs::read(&merges_path).unwrap();
        assert_eq!(
            written.vocab.file_sha256.as_slice(),
            Sha256::digest(&vocab_bytes).as_slice()
        );
        assert_eq!(
            written.vocab.file_sha512.as_slice(),
            Sha512::digest(&vocab_bytes).as_slice()
        );
        assert_eq!(
            written.merges.file_sha256.as_slice(),
            Sha256::digest(&merge_bytes).as_slice()
        );
        assert_eq!(
            written.merges.file_sha512.as_slice(),
            Sha512::digest(&merge_bytes).as_slice()
        );

        let round_trip = load_tokenizer_package(
            &vocab_path,
            &merges_path,
            ArtifactFormat::V3U32,
            &artifact_limits(),
        )
        .unwrap();
        assert_eq!(round_trip, tokenizer);

        let collision = match write_vocab_file_create_new(&tokenizer, &vocab_path, &measured.vocab)
        {
            Ok(_) => panic!("create-new writer overwrote an existing vocab artifact"),
            Err(error) => error,
        };
        assert_eq!(collision.class(), "ExistingDestination");
        assert_eq!(fs::read(&vocab_path).unwrap(), vocab_bytes);
        assert_eq!(fs::read(&merges_path).unwrap(), merge_bytes);
    }

    #[test]
    fn package_publication_is_manifest_last_strict_and_path_independent() {
        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("package_success");
        let destination_a = parent.join("checkpoint-a");
        let destination_b = parent.join("checkpoint-b");

        let package_a = write_v3_package(&tokenizer, &destination_a, &context).unwrap();
        let package_b = write_v3_package(&tokenizer, &destination_b, &context).unwrap();
        assert_eq!(
            package_a.destination(),
            fs::canonicalize(&destination_a).unwrap()
        );
        assert_eq!(
            package_b.destination(),
            fs::canonicalize(&destination_b).unwrap()
        );
        assert_eq!(package_a.vocab_size(), 32_768);
        assert_eq!(package_a.merge_count(), 32_492);
        assert!(package_a.durability().artifact_files_synced());
        assert!(package_a.durability().manifest_file_synced());

        for relative in [VOCAB_FILENAME, MERGES_FILENAME, PACKAGE_MANIFEST_FILENAME] {
            assert_eq!(
                fs::read(destination_a.join(relative)).unwrap(),
                fs::read(destination_b.join(relative)).unwrap()
            );
        }
        assert_eq!(package_a.vocab_evidence(), package_b.vocab_evidence());
        assert_eq!(package_a.merges_evidence(), package_b.merges_evidence());
        assert_eq!(package_a.manifest_bytes(), package_b.manifest_bytes());
        assert_eq!(package_a.manifest_sha256(), package_b.manifest_sha256());
        assert_eq!(package_a.manifest_sha512(), package_b.manifest_sha512());

        let manifest_bytes = fs::read(destination_a.join(PACKAGE_MANIFEST_FILENAME)).unwrap();
        assert_eq!(
            package_a.manifest_sha256().as_slice(),
            Sha256::digest(&manifest_bytes).as_slice()
        );
        assert_eq!(
            package_a.manifest_sha512().as_slice(),
            Sha512::digest(&manifest_bytes).as_slice()
        );
        let manifest = ArtifactPackageManifestV1::parse_canonical_json(&manifest_bytes).unwrap();
        assert_eq!(manifest.vocab_size(), 32_768);
        assert_eq!(manifest.merge_count(), 32_492);
        assert_eq!(manifest.run_id(), context.run_id());

        let round_trip = load_tokenizer_package(
            &destination_a.join(VOCAB_FILENAME),
            &destination_a.join(MERGES_FILENAME),
            ArtifactFormat::V3U32,
            &artifact_limits(),
        )
        .unwrap();
        assert_eq!(round_trip, tokenizer);
    }

    #[test]
    fn manifest_bound_loader_accepts_only_directory_or_exact_manifest() {
        let tokenizer = base_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("manifest_bound_load");
        let destination = parent.join("checkpoint");
        write_v3_package(&tokenizer, &destination, &context).unwrap();

        assert_eq!(
            load_v3_tokenizer_package(&destination, &artifact_limits()).unwrap(),
            tokenizer
        );
        assert_eq!(
            load_v3_tokenizer_package(
                &destination.join(PACKAGE_MANIFEST_FILENAME),
                &artifact_limits(),
            )
            .unwrap(),
            tokenizer
        );

        let headerless = parent.join("headerless.bin");
        fs::write(&headerless, b"legacy headerless bytes").unwrap();
        assert_eq!(
            load_v3_tokenizer_package(&headerless, &artifact_limits())
                .unwrap_err()
                .class(),
            "InvalidPublicationPath"
        );
        assert_eq!(
            load_v3_tokenizer_package(&destination.join(VOCAB_FILENAME), &artifact_limits())
                .unwrap_err()
                .class(),
            "InvalidPublicationPath"
        );
    }

    #[test]
    fn manifest_bound_loader_rejects_complete_file_hash_mismatch() {
        let tokenizer = base_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("manifest_hash_mismatch");
        let destination = parent.join("checkpoint");
        write_v3_package(&tokenizer, &destination, &context).unwrap();
        let vocab_path = destination.join(VOCAB_FILENAME);
        let mut bytes = fs::read(&vocab_path).unwrap();
        let final_byte = bytes.last_mut().unwrap();
        *final_byte ^= 0x01;
        fs::write(&vocab_path, bytes).unwrap();

        assert_eq!(
            load_v3_tokenizer_package(&destination, &artifact_limits())
                .unwrap_err()
                .class(),
            "DurabilityFailure"
        );
    }

    #[cfg(windows)]
    #[test]
    fn manifest_bound_loader_rejects_directory_reparse_selection() {
        use std::os::windows::fs::symlink_dir;

        let tokenizer = base_tokenizer();
        let context = valid_context();
        let root = unique_test_directory("manifest_load_reparse");
        let destination = root.join("checkpoint");
        write_v3_package(&tokenizer, &destination, &context).unwrap();
        let linked = root.join("linked-checkpoint");
        symlink_dir(&destination, &linked).unwrap();

        assert_eq!(
            load_v3_tokenizer_package(&linked, &artifact_limits())
                .unwrap_err()
                .class(),
            "InvalidPublicationPath"
        );
    }

    #[test]
    fn existing_final_destination_fails_before_staging_and_preserves_sentinel() {
        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let destination = unique_test_directory("existing_destination");
        let sentinel = destination.join("operator-owned.txt");
        let mut sentinel_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&sentinel)
            .unwrap();
        sentinel_file.write_all(b"preserve").unwrap();
        sentinel_file.sync_all().unwrap();
        let parent = destination.parent().unwrap();
        let before = staging_directories(parent).len();

        let error = write_v3_package(&tokenizer, &destination, &context).unwrap_err();
        assert_eq!(error.class(), "ExistingDestination");
        assert_eq!(fs::read(&sentinel).unwrap(), b"preserve");
        assert_eq!(staging_directories(parent).len(), before);
    }

    #[test]
    fn injected_pre_rename_failures_never_publish_final_directory() {
        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("phase_failures");
        let phases = [
            PublicationPhase::StagingCreated,
            PublicationPhase::VocabWritten,
            PublicationPhase::MergesWritten,
            PublicationPhase::ReadbackCompleted,
            PublicationPhase::CompleteHashesVerified,
            PublicationPhase::ManifestWritten,
            PublicationPhase::ManifestReadbackCompleted,
            PublicationPhase::StagingDirectorySynced,
        ];

        for (ordinal, target_phase) in phases.into_iter().enumerate() {
            let destination = parent.join(format!("blocked-{ordinal}"));
            let before = staging_directories(&parent);
            let error =
                write_v3_package_with_hook(&tokenizer, &destination, &context, |phase, _, _| {
                    if phase == target_phase {
                        Err(injected_failure("publication phase interruption"))
                    } else {
                        Ok(())
                    }
                })
                .unwrap_err();
            assert_eq!(error.class(), "DurabilityFailure");
            assert!(!destination.exists());
            let after = staging_directories(&parent);
            assert_eq!(after.len(), before.len() + 1);
            let staging = after.iter().find(|path| !before.contains(path)).unwrap();
            let manifest_exists = staging.join(PACKAGE_MANIFEST_FILENAME).exists();
            assert_eq!(
                manifest_exists,
                matches!(
                    target_phase,
                    PublicationPhase::ManifestWritten
                        | PublicationPhase::ManifestReadbackCompleted
                        | PublicationPhase::StagingDirectorySynced
                )
            );
        }
    }

    #[test]
    fn staged_corruption_blocks_manifest_and_final_publication() {
        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("corruption");
        let destination = parent.join("corrupted-checkpoint");
        let error =
            write_v3_package_with_hook(&tokenizer, &destination, &context, |phase, staging, _| {
                if phase == PublicationPhase::MergesWritten {
                    let mut file = OpenOptions::new()
                        .read(true)
                        .write(true)
                        .open(staging.join(VOCAB_FILENAME))?;
                    file.seek(SeekFrom::Start(V3_HEADER_BYTES_U64))?;
                    let mut byte = [0u8; 1];
                    file.read_exact(&mut byte)?;
                    byte[0] ^= 1;
                    file.seek(SeekFrom::Start(V3_HEADER_BYTES_U64))?;
                    file.write_all(&byte)?;
                    file.sync_all()?;
                }
                Ok(())
            })
            .unwrap_err();
        assert_eq!(error.class(), "DurabilityFailure");
        assert!(!destination.exists());
        let staging = staging_directories(&parent);
        assert_eq!(staging.len(), 1);
        assert!(!staging[0].join(PACKAGE_MANIFEST_FILENAME).exists());
    }

    #[test]
    fn rename_race_does_not_overwrite_late_destination() {
        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let parent = unique_test_directory("rename_race");
        let destination = parent.join("raced-checkpoint");
        let sentinel = destination.join("late-owner.txt");
        let error = write_v3_package_with_hook(
            &tokenizer,
            &destination,
            &context,
            |phase, _, final_destination| {
                if phase == PublicationPhase::StagingDirectorySynced {
                    fs::create_dir(final_destination)?;
                    let mut file = OpenOptions::new()
                        .write(true)
                        .create_new(true)
                        .open(final_destination.join("late-owner.txt"))?;
                    file.write_all(b"late owner")?;
                    file.sync_all()?;
                }
                Ok(())
            },
        )
        .unwrap_err();
        assert_eq!(error.class(), "ExistingDestination");
        assert_eq!(fs::read(&sentinel).unwrap(), b"late owner");
        assert_eq!(staging_directories(&parent).len(), 1);
    }

    #[test]
    fn independent_complete_file_vectors_match_empty_one_and_cross_u16_publication() {
        let context = valid_context();
        for vector in &BOUNDARY_VECTORS {
            let independent =
                independent_boundary_artifacts(vector.merge_count, vector.locked_one_merge);
            assert_eq!(independent.vocab.len(), vector.vocab_bytes);
            assert_eq!(
                hex_upper(&Sha256::digest(&independent.vocab)),
                vector.vocab_sha256
            );
            assert_eq!(
                hex_upper(&Sha512::digest(&independent.vocab)),
                vector.vocab_sha512
            );
            assert_eq!(independent.merges.len(), vector.merges_bytes);
            assert_eq!(
                hex_upper(&Sha256::digest(&independent.merges)),
                vector.merges_sha256
            );
            assert_eq!(
                hex_upper(&Sha512::digest(&independent.merges)),
                vector.merges_sha512
            );

            let root = unique_test_directory(vector.name);
            write_independent_test_file(&root.join(VOCAB_FILENAME), &independent.vocab);
            write_independent_test_file(&root.join(MERGES_FILENAME), &independent.merges);
            let tokenizer = load_tokenizer_package(
                &root.join(VOCAB_FILENAME),
                &root.join(MERGES_FILENAME),
                ArtifactFormat::V3U32,
                &artifact_limits(),
            )
            .unwrap();
            let expected_vocab_size = usize::try_from(276 + vector.merge_count).unwrap();
            assert_eq!(tokenizer.vocab_size(), expected_vocab_size);
            assert_eq!(
                tokenizer.merge_count(),
                usize::try_from(vector.merge_count).unwrap()
            );
            if vector.name == "one" {
                assert_eq!(tokenizer.token_bytes(276), Some(b" t".as_slice()));
            }
            if vector.name == "cross_u16" {
                assert_eq!(tokenizer.token_bytes(65_536), Some([254, 236].as_slice()));
                assert_eq!(tokenizer.merge_at(65_260).unwrap().merged, 65_536);
            }

            let destination = root.join("published");
            let package = write_v3_package(&tokenizer, &destination, &context).unwrap();
            assert_eq!(package.vocab_size(), 276 + vector.merge_count);
            assert_eq!(package.merge_count(), u64::from(vector.merge_count));
            assert_eq!(
                fs::read(destination.join(VOCAB_FILENAME)).unwrap(),
                independent.vocab
            );
            assert_eq!(
                fs::read(destination.join(MERGES_FILENAME)).unwrap(),
                independent.merges
            );
        }
    }

    #[cfg(windows)]
    #[test]
    fn runtime_directory_symlink_parent_fails_before_staging() {
        use std::os::windows::fs::symlink_dir;

        let tokenizer = canonical_v2_tokenizer();
        let context = valid_context();
        let root = unique_test_directory("runtime_reparse");
        let real_parent = root.join("real-parent");
        let linked_parent = root.join("linked-parent");
        fs::create_dir(&real_parent).unwrap();
        symlink_dir(&real_parent, &linked_parent).unwrap();
        let destination = linked_parent.join("blocked-package");

        let error = write_v3_package(&tokenizer, &destination, &context).unwrap_err();
        assert_eq!(error.class(), "InvalidPublicationPath");
        assert!(!real_parent.join("blocked-package").exists());
        assert!(staging_directories(&real_parent).is_empty());
    }

    fn staging_directories(parent: &Path) -> Vec<std::path::PathBuf> {
        let mut paths: Vec<_> = fs::read_dir(parent)
            .unwrap()
            .map(|entry| entry.unwrap().path())
            .filter(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(|name| name.starts_with(".ashira-v3-staging-"))
            })
            .collect();
        paths.sort();
        paths
    }

    fn injected_failure(operation: &'static str) -> ArtifactError {
        ArtifactError::DurabilityFailure {
            operation,
            source: std::io::Error::new(std::io::ErrorKind::Interrupted, operation),
        }
    }
}
