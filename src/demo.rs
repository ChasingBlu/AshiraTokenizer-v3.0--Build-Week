use crate::{
    ArtifactFileEvidenceInput, ArtifactFormat, ArtifactKind, ArtifactLimits, CodecLimits,
    DEMO_WIKITEXT_MANIFEST_LABEL, DEMO_WIKITEXT_MANIFEST_SCHEMA, EncodedTokensV1, FamilyWeights,
    ManifestAdmissionProfile, ManifestRoot, PublicationContext, PublicationContextInput,
    TokenizerTrainer, TrainConfig, admit_demo_wikitext_manifest, inspect_artifact,
    load_v3_tokenizer_package, validate_vocab_target, write_v3_package,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::error::Error;
use std::ffi::OsString;
use std::fmt;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Component, Path, PathBuf};
use std::process::Command;
use std::sync::atomic::{AtomicU64, Ordering};

pub const DEMO_MAX_VOCAB_SIZE: usize = 4_096;
pub const DEMO_RUN_MANIFEST_SCHEMA: &str = "ashira_v3_demo_run_manifest_v1";

const DEMO_CORPUS_ROOT_ID: &str = "demo_wikitext";
const DEMO_CORPUS_DIRECTORY: &str = "corpus";
const DEMO_PACKAGE_DIRECTORY: &str = "package";
const DEMO_RUN_MANIFEST_FILENAME: &str = "demo_run_manifest.json";
const DEMO_FINAL_VALIDATION_FILENAME: &str = "demo_final_validation.json";
const DEMO_CONFIG_FILENAME: &str = "demo_training_config.json";
const DEMO_CALIBRATION_FILENAME: &str = "demo_calibration_assertion.json";
const DEMO_PROBE_FILENAME: &str = "demo_probe_assertion.json";
const DEMO_INPUT_FILENAME: &str = "demo_input.txt";
const DEMO_ENCODED_FILENAME: &str = "demo_encoded.json";
const DEMO_DECODED_FILENAME: &str = "demo_decoded.txt";
const DEMO_MIN_FREQUENCY: u32 = 2;
const DEMO_FAMILY_WEIGHT_SCALED: i64 = 1_000;
const MAX_STAGING_ATTEMPTS: u64 = 1_024;
const MAX_TRACKED_FILES: usize = 100_000;
const MAX_TRACKED_FILE_BYTES: u64 = 512 * 1024 * 1024;
const MAX_TRACKED_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEMO_COMPARE_MAX_TOTAL_BYTES: u64 = 2 * 1024 * 1024 * 1024;
const DEMO_INPUT: &[u8] = include_bytes!("../demo/demo_input.txt");

const DEMO_COMPARISON_FILES: [(&str, u64); 11] = [
    (DEMO_CALIBRATION_FILENAME, 1024 * 1024),
    (DEMO_DECODED_FILENAME, 32 * 1024 * 1024),
    (DEMO_ENCODED_FILENAME, 128 * 1024 * 1024),
    (DEMO_FINAL_VALIDATION_FILENAME, 1024 * 1024),
    (DEMO_INPUT_FILENAME, 8 * 1024 * 1024),
    (DEMO_PROBE_FILENAME, 1024 * 1024),
    (DEMO_RUN_MANIFEST_FILENAME, 1024 * 1024),
    (DEMO_CONFIG_FILENAME, 1024 * 1024),
    ("package/merges.bin", 512 * 1024 * 1024),
    ("package/package_manifest.json", 1024 * 1024),
    ("package/vocab.bin", 512 * 1024 * 1024),
];

static NEXT_STAGING_ORDINAL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum DemoFailureKind {
    Input,
    Training,
    Publication,
}

#[derive(Debug)]
pub struct DemoError {
    kind: DemoFailureKind,
    class: &'static str,
    message: String,
}

impl DemoError {
    fn input(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: DemoFailureKind::Input,
            class,
            message: message.into(),
        }
    }

    fn training(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: DemoFailureKind::Training,
            class,
            message: message.into(),
        }
    }

    fn publication(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            kind: DemoFailureKind::Publication,
            class,
            message: message.into(),
        }
    }

    pub const fn class(&self) -> &'static str {
        self.class
    }

    pub const fn exit_code(&self) -> i32 {
        match self.kind {
            DemoFailureKind::Input => 3,
            DemoFailureKind::Training => 4,
            DemoFailureKind::Publication => 5,
        }
    }
}

impl fmt::Display for DemoError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for DemoError {}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemoPipelineResult {
    vocab_size: u32,
    merge_count: u64,
    token_count: u64,
    package_manifest_sha256: [u8; 32],
    deterministic_core_sha256: [u8; 32],
    round_trip_sha256: [u8; 32],
}

impl DemoPipelineResult {
    pub const fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    pub const fn merge_count(&self) -> u64 {
        self.merge_count
    }

    pub const fn token_count(&self) -> u64 {
        self.token_count
    }

    pub const fn package_manifest_sha256(&self) -> [u8; 32] {
        self.package_manifest_sha256
    }

    pub const fn deterministic_core_sha256(&self) -> [u8; 32] {
        self.deterministic_core_sha256
    }

    pub const fn round_trip_sha256(&self) -> [u8; 32] {
        self.round_trip_sha256
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct DemoCompareResult {
    file_count: usize,
    total_bytes: u64,
    vocab_size: u32,
    merge_count: u64,
    token_count: u64,
    package_manifest_sha256: [u8; 32],
    run_tree_sha256: [u8; 32],
    deterministic_core_sha256: [u8; 32],
    source_commit: String,
}

impl DemoCompareResult {
    pub const fn file_count(&self) -> usize {
        self.file_count
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub const fn vocab_size(&self) -> u32 {
        self.vocab_size
    }

    pub const fn merge_count(&self) -> u64 {
        self.merge_count
    }

    pub const fn token_count(&self) -> u64 {
        self.token_count
    }

    pub const fn package_manifest_sha256(&self) -> [u8; 32] {
        self.package_manifest_sha256
    }

    pub const fn run_tree_sha256(&self) -> [u8; 32] {
        self.run_tree_sha256
    }

    pub const fn deterministic_core_sha256(&self) -> [u8; 32] {
        self.deterministic_core_sha256
    }

    pub fn source_commit(&self) -> &str {
        &self.source_commit
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
struct SourceAuthority {
    commit: String,
    tree: String,
    tracked_manifest_sha256: [u8; 32],
    toolchain_identity: String,
    live: bool,
}

struct RunDestination {
    parent: PathBuf,
    final_path: PathBuf,
    filename: OsString,
}

#[derive(Serialize)]
struct DemoConfigWire<'a> {
    schema: &'a str,
    manifest_schema: &'a str,
    manifest_label: &'a str,
    vocab_size: u32,
    min_frequency: u32,
    deterministic: bool,
    family_weight_scaled: i64,
    effective_backend: &'a str,
    demo_input_sha256: String,
}

#[derive(Serialize)]
struct GateAssertionWire<'a> {
    schema: &'a str,
    gate: &'a str,
    status: &'a str,
    reason: &'a str,
}

#[derive(Serialize)]
struct DemoRunManifestWire {
    schema: &'static str,
    status: &'static str,
    authority: DemoAuthorityWire,
    source: DemoSourceWire,
    corpus: DemoCorpusWire,
    config: DemoConfigEvidenceWire,
    training: DemoTrainingWire,
    package: DemoPackageWire,
    round_trip: DemoRoundTripWire,
    assertions: DemoAssertionsWire,
    validation: DemoValidationWire,
}

#[derive(Serialize)]
struct DemoAuthorityWire {
    profile: &'static str,
    label: &'static str,
    balanced_composite_claimed: bool,
    final_v3_training_authority_claimed: bool,
    other_family_gates_closed: bool,
}

#[derive(Serialize)]
struct DemoSourceWire {
    commit: String,
    tree: String,
    tracked_manifest_schema: &'static str,
    tracked_manifest_sha256: String,
    writer_version: &'static str,
    toolchain_identity: String,
    effective_backend: &'static str,
}

#[derive(Serialize)]
struct DemoCorpusWire {
    manifest_schema: &'static str,
    manifest_label: &'static str,
    manifest_bytes: u64,
    manifest_sha256: String,
    manifest_sha512: String,
    admitted_files: usize,
    admitted_bytes: u64,
}

#[derive(Serialize)]
struct DemoConfigEvidenceWire {
    file: &'static str,
    bytes: u64,
    sha256: String,
}

#[derive(Serialize)]
struct DemoTrainingWire {
    input_files: usize,
    loaded_sequences: usize,
    skipped_lines: usize,
    loaded_tokens: usize,
    learned_merges: usize,
    final_vocab: usize,
    trainer_fnv64: String,
}

#[derive(Serialize)]
struct DemoPackageWire {
    directory: &'static str,
    vocab_size: u32,
    merge_count: u64,
    vocab: DemoArtifactWire,
    merges: DemoArtifactWire,
    manifest_bytes: u64,
    manifest_sha256: String,
    manifest_sha512: String,
}

#[derive(Serialize)]
struct DemoArtifactWire {
    file_bytes: u64,
    payload_bytes: u64,
    record_count: u64,
    file_sha256: String,
    file_sha512: String,
    payload_sha256: String,
    sequence_sha256: String,
}

#[derive(Serialize)]
struct DemoRoundTripWire {
    input_file: &'static str,
    input_bytes: u64,
    input_sha256: String,
    encoded_file: &'static str,
    encoded_bytes: u64,
    encoded_sha256: String,
    token_count: u64,
    decoded_file: &'static str,
    decoded_bytes: u64,
    decoded_sha256: String,
    byte_equal: bool,
}

#[derive(Serialize)]
struct DemoAssertionsWire {
    calibration_file: &'static str,
    calibration_sha256: String,
    probe_file: &'static str,
    probe_sha256: String,
    external_attribution_license_status: &'static str,
}

#[derive(Serialize)]
struct DemoValidationWire {
    writer_strict_readback: bool,
    manifest_bound_load_before_publication: bool,
    canonical_encoded_json_reparse: bool,
    byte_round_trip: bool,
    final_package_reload_attestation: &'static str,
    file_sync_all: bool,
    directory_sync_claimed: bool,
    publication_visibility: &'static str,
}

#[derive(Serialize)]
struct DemoFinalValidationWire {
    schema: &'static str,
    status: &'static str,
    run_manifest_sha256: String,
    package_manifest_sha256: String,
    round_trip_sha256: String,
    final_package_reload: bool,
    final_report_readback: bool,
    final_run_files_readback: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoConfigReadback {
    schema: String,
    manifest_schema: String,
    manifest_label: String,
    vocab_size: u32,
    min_frequency: u32,
    deterministic: bool,
    family_weight_scaled: i64,
    effective_backend: String,
    demo_input_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct GateAssertionReadback {
    schema: String,
    gate: String,
    status: String,
    reason: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoRunManifestReadback {
    schema: String,
    status: String,
    authority: DemoAuthorityReadback,
    source: DemoSourceReadback,
    corpus: DemoCorpusReadback,
    config: DemoConfigEvidenceReadback,
    training: DemoTrainingReadback,
    package: DemoPackageReadback,
    round_trip: DemoRoundTripReadback,
    assertions: DemoAssertionsReadback,
    validation: DemoValidationReadback,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoAuthorityReadback {
    profile: String,
    label: String,
    balanced_composite_claimed: bool,
    final_v3_training_authority_claimed: bool,
    other_family_gates_closed: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoSourceReadback {
    commit: String,
    tree: String,
    tracked_manifest_schema: String,
    tracked_manifest_sha256: String,
    writer_version: String,
    toolchain_identity: String,
    effective_backend: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoCorpusReadback {
    manifest_schema: String,
    manifest_label: String,
    manifest_bytes: u64,
    manifest_sha256: String,
    manifest_sha512: String,
    admitted_files: usize,
    admitted_bytes: u64,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoConfigEvidenceReadback {
    file: String,
    bytes: u64,
    sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoTrainingReadback {
    input_files: usize,
    loaded_sequences: usize,
    skipped_lines: usize,
    loaded_tokens: usize,
    learned_merges: usize,
    final_vocab: usize,
    trainer_fnv64: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoPackageReadback {
    directory: String,
    vocab_size: u32,
    merge_count: u64,
    vocab: DemoArtifactReadback,
    merges: DemoArtifactReadback,
    manifest_bytes: u64,
    manifest_sha256: String,
    manifest_sha512: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoArtifactReadback {
    file_bytes: u64,
    payload_bytes: u64,
    record_count: u64,
    file_sha256: String,
    file_sha512: String,
    payload_sha256: String,
    sequence_sha256: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoRoundTripReadback {
    input_file: String,
    input_bytes: u64,
    input_sha256: String,
    encoded_file: String,
    encoded_bytes: u64,
    encoded_sha256: String,
    token_count: u64,
    decoded_file: String,
    decoded_bytes: u64,
    decoded_sha256: String,
    byte_equal: bool,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoAssertionsReadback {
    calibration_file: String,
    calibration_sha256: String,
    probe_file: String,
    probe_sha256: String,
    external_attribution_license_status: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoValidationReadback {
    writer_strict_readback: bool,
    manifest_bound_load_before_publication: bool,
    canonical_encoded_json_reparse: bool,
    byte_round_trip: bool,
    final_package_reload_attestation: String,
    file_sync_all: bool,
    directory_sync_claimed: bool,
    publication_visibility: String,
}

#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct DemoFinalValidationReadback {
    schema: String,
    status: String,
    run_manifest_sha256: String,
    package_manifest_sha256: String,
    round_trip_sha256: String,
    final_package_reload: bool,
    final_report_readback: bool,
    final_run_files_readback: bool,
}

struct DemoRunFile {
    relative: &'static str,
    bytes: Vec<u8>,
}

struct ValidatedDemoRun {
    files: Vec<DemoRunFile>,
    result: DemoCompareResult,
}

pub fn compare_demo_runs(run_a: &Path, run_b: &Path) -> Result<DemoCompareResult, DemoError> {
    let root_a = canonical_demo_run_root(run_a, "Run A")?;
    let root_b = canonical_demo_run_root(run_b, "Run B")?;
    if root_a == root_b {
        return Err(DemoError::input(
            "DemoCompare",
            "Run A and Run B resolve to the same directory",
        ));
    }

    let validated_a = validate_demo_run(&root_a, "Run A")?;
    let validated_b = validate_demo_run(&root_b, "Run B")?;
    for (file_a, file_b) in validated_a.files.iter().zip(&validated_b.files) {
        if file_a.relative != file_b.relative || file_a.bytes != file_b.bytes {
            return Err(DemoError::input(
                "DemoCompare",
                format!("A/B byte mismatch: {}", file_a.relative),
            ));
        }
    }
    if validated_a.result != validated_b.result {
        return Err(DemoError::input(
            "DemoCompare",
            "A/B semantic or aggregate identity mismatch",
        ));
    }
    Ok(validated_a.result)
}

fn validate_demo_run(root: &Path, label: &'static str) -> Result<ValidatedDemoRun, DemoError> {
    validate_demo_run_topology(root, label)?;
    let mut files = Vec::new();
    files
        .try_reserve_exact(DEMO_COMPARISON_FILES.len())
        .map_err(|_| {
            DemoError::input(
                "DemoCompare",
                format!("cannot allocate {label} file inventory"),
            )
        })?;
    let mut total_bytes = 0u64;
    let mut run_tree = Sha256::new();
    for (relative, limit) in DEMO_COMPARISON_FILES {
        let bytes = read_bounded_file(&root.join(relative), limit, "demo comparison file")?;
        let length = u64::try_from(bytes.len()).map_err(|_| {
            DemoError::input(
                "DemoCompare",
                format!("{label} file length overflow: {relative}"),
            )
        })?;
        total_bytes = total_bytes.checked_add(length).ok_or_else(|| {
            DemoError::input("DemoCompare", format!("{label} total byte overflow"))
        })?;
        if total_bytes > DEMO_COMPARE_MAX_TOTAL_BYTES {
            return Err(DemoError::input(
                "DemoCompare",
                format!("{label} total bytes exceed the comparison bound"),
            ));
        }
        let record = format!("{}  {length}  {relative}\n", hex_lower(&sha256(&bytes)));
        run_tree.update(record.as_bytes());
        files.push(DemoRunFile { relative, bytes });
    }

    let run_manifest_bytes = demo_run_file(&files, DEMO_RUN_MANIFEST_FILENAME)?;
    let run_manifest: DemoRunManifestReadback =
        parse_canonical_readback(run_manifest_bytes, "demo run manifest")?;
    validate_run_manifest_authority(&run_manifest, label)?;

    let config_bytes = demo_run_file(&files, DEMO_CONFIG_FILENAME)?;
    let config: DemoConfigReadback =
        parse_canonical_readback(config_bytes, "demo training configuration")?;
    validate_demo_config(&config, &run_manifest, config_bytes, label)?;

    let calibration_bytes = demo_run_file(&files, DEMO_CALIBRATION_FILENAME)?;
    let calibration: GateAssertionReadback =
        parse_canonical_readback(calibration_bytes, "demo calibration assertion")?;
    validate_gate_assertion(
        &calibration,
        "calibration",
        calibration_bytes,
        &run_manifest.assertions.calibration_sha256,
        label,
    )?;
    let probe_bytes = demo_run_file(&files, DEMO_PROBE_FILENAME)?;
    let probe: GateAssertionReadback =
        parse_canonical_readback(probe_bytes, "demo probe assertion")?;
    validate_gate_assertion(
        &probe,
        "probe_selection",
        probe_bytes,
        &run_manifest.assertions.probe_sha256,
        label,
    )?;

    let package_manifest_bytes = demo_run_file(&files, "package/package_manifest.json")?;
    let package_path = root.join(DEMO_PACKAGE_DIRECTORY);
    let artifact_limits = demo_artifact_limits();
    let tokenizer =
        load_v3_tokenizer_package(&package_path, &artifact_limits).map_err(|error| {
            DemoError::input(
                "DemoCompare",
                format!("{label} strict package load failed: {error}"),
            )
        })?;
    validate_package_evidence(
        root,
        &files,
        &run_manifest,
        package_manifest_bytes,
        tokenizer.vocab_size(),
        tokenizer.merge_count(),
        label,
    )?;

    let input = demo_run_file(&files, DEMO_INPUT_FILENAME)?;
    let encoded_bytes = demo_run_file(&files, DEMO_ENCODED_FILENAME)?;
    let decoded_file = demo_run_file(&files, DEMO_DECODED_FILENAME)?;
    let codec_limits = demo_codec_limits();
    let encoded = EncodedTokensV1::parse_json(encoded_bytes, &codec_limits).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("{label} encoded document parse failed: {error}"),
        )
    })?;
    let canonical_encoded = encoded.to_canonical_json(&codec_limits).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("{label} encoded document serialization failed: {error}"),
        )
    })?;
    let decoded = encoded.decode(&tokenizer, &codec_limits).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("{label} encoded document decode failed: {error}"),
        )
    })?;
    if canonical_encoded != encoded_bytes
        || input != DEMO_INPUT
        || decoded != input
        || decoded_file != input
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} canonical encoding or fixed byte round trip is invalid"),
        ));
    }
    validate_round_trip_evidence(
        &run_manifest,
        input,
        encoded_bytes,
        decoded_file,
        encoded.token_ids().len(),
        label,
    )?;

    let final_validation_bytes = demo_run_file(&files, DEMO_FINAL_VALIDATION_FILENAME)?;
    let final_validation: DemoFinalValidationReadback =
        parse_canonical_readback(final_validation_bytes, "demo final validation")?;
    let package_manifest_sha256 = sha256(package_manifest_bytes);
    let round_trip_sha256 = sha256(decoded_file);
    if final_validation.schema != "ashira_v3_demo_final_validation_v1"
        || final_validation.status != "PASS"
        || final_validation.run_manifest_sha256 != hex_upper(&sha256(run_manifest_bytes))
        || final_validation.package_manifest_sha256 != hex_upper(&package_manifest_sha256)
        || final_validation.round_trip_sha256 != hex_upper(&round_trip_sha256)
        || !final_validation.final_package_reload
        || !final_validation.final_report_readback
        || !final_validation.final_run_files_readback
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} final validation attestation is invalid"),
        ));
    }

    let mut deterministic_core = Sha256::new();
    deterministic_core.update(b"ashira_v3_demo_deterministic_core_v1\0");
    deterministic_core.update(run_manifest_bytes);
    deterministic_core.update(final_validation_bytes);

    let vocab_size = u32::try_from(tokenizer.vocab_size()).map_err(|_| {
        DemoError::input("DemoCompare", format!("{label} vocabulary count overflow"))
    })?;
    let merge_count = u64::try_from(tokenizer.merge_count())
        .map_err(|_| DemoError::input("DemoCompare", format!("{label} merge count overflow")))?;
    let token_count = u64::try_from(encoded.token_ids().len())
        .map_err(|_| DemoError::input("DemoCompare", format!("{label} token count overflow")))?;
    Ok(ValidatedDemoRun {
        files,
        result: DemoCompareResult {
            file_count: DEMO_COMPARISON_FILES.len(),
            total_bytes,
            vocab_size,
            merge_count,
            token_count,
            package_manifest_sha256,
            run_tree_sha256: finalize_sha256(run_tree.finalize()),
            deterministic_core_sha256: finalize_sha256(deterministic_core.finalize()),
            source_commit: run_manifest.source.commit,
        },
    })
}

fn canonical_demo_run_root(path: &Path, label: &'static str) -> Result<PathBuf, DemoError> {
    let absolute = absolute_path(path)?;
    ensure_no_link_like_ancestors(&absolute)?;
    let canonical = fs::canonicalize(&absolute).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("cannot resolve {label} root {}: {error}", path.display()),
        )
    })?;
    ensure_no_link_like_ancestors(&canonical)?;
    let metadata = fs::symlink_metadata(&canonical).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("cannot inspect {label} root: {error}"),
        )
    })?;
    if metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} root is not a non-link directory"),
        ));
    }
    Ok(canonical)
}

fn validate_demo_run_topology(root: &Path, label: &'static str) -> Result<(), DemoError> {
    validate_exact_directory_entries(
        root,
        &[
            DEMO_CALIBRATION_FILENAME,
            DEMO_DECODED_FILENAME,
            DEMO_ENCODED_FILENAME,
            DEMO_FINAL_VALIDATION_FILENAME,
            DEMO_INPUT_FILENAME,
            DEMO_PACKAGE_DIRECTORY,
            DEMO_PROBE_FILENAME,
            DEMO_RUN_MANIFEST_FILENAME,
            DEMO_CONFIG_FILENAME,
        ],
        Some(DEMO_PACKAGE_DIRECTORY),
        label,
    )?;
    validate_exact_directory_entries(
        &root.join(DEMO_PACKAGE_DIRECTORY),
        &["merges.bin", "package_manifest.json", "vocab.bin"],
        None,
        label,
    )
}

fn validate_exact_directory_entries(
    directory: &Path,
    expected: &[&str],
    expected_directory: Option<&str>,
    label: &'static str,
) -> Result<(), DemoError> {
    ensure_no_link_like_ancestors(directory)?;
    let mut actual = Vec::new();
    actual.try_reserve_exact(expected.len()).map_err(|_| {
        DemoError::input("DemoCompare", format!("cannot allocate {label} topology"))
    })?;
    let entries = fs::read_dir(directory).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("cannot enumerate {label} directory: {error}"),
        )
    })?;
    for entry in entries {
        let entry = entry.map_err(|error| {
            DemoError::input(
                "DemoCompare",
                format!("cannot enumerate {label} entry: {error}"),
            )
        })?;
        if actual.len() >= expected.len() {
            return Err(DemoError::input(
                "DemoCompare",
                format!("{label} contains unexpected entries"),
            ));
        }
        let name = entry.file_name().into_string().map_err(|_| {
            DemoError::input("DemoCompare", format!("{label} contains a non-UTF-8 entry"))
        })?;
        let metadata = fs::symlink_metadata(entry.path()).map_err(|error| {
            DemoError::input(
                "DemoCompare",
                format!("cannot inspect {label} entry {name}: {error}"),
            )
        })?;
        let should_be_directory = expected_directory == Some(name.as_str());
        if metadata_is_link_like(&metadata)
            || (should_be_directory && !metadata.is_dir())
            || (!should_be_directory && !metadata.is_file())
        {
            return Err(DemoError::input(
                "DemoCompare",
                format!("{label} entry has invalid type: {name}"),
            ));
        }
        actual.push(name);
    }
    actual.sort_unstable();
    let mut expected_sorted = expected.to_vec();
    expected_sorted.sort_unstable();
    if actual.iter().map(String::as_str).collect::<Vec<_>>() != expected_sorted {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} does not contain the exact governed 11-file topology"),
        ));
    }
    Ok(())
}

fn demo_run_file<'a>(files: &'a [DemoRunFile], relative: &str) -> Result<&'a [u8], DemoError> {
    files
        .iter()
        .find(|file| file.relative == relative)
        .map(|file| file.bytes.as_slice())
        .ok_or_else(|| {
            DemoError::input(
                "DemoCompare",
                format!("comparison inventory is missing {relative}"),
            )
        })
}

fn parse_canonical_readback<T>(bytes: &[u8], label: &'static str) -> Result<T, DemoError>
where
    T: serde::de::DeserializeOwned + Serialize,
{
    let value: T = serde_json::from_slice(bytes).map_err(|error| {
        DemoError::input("DemoCompare", format!("cannot parse {label}: {error}"))
    })?;
    let mut canonical = serde_json::to_vec(&value).map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("cannot reserialize {label}: {error}"),
        )
    })?;
    canonical.push(b'\n');
    if canonical != bytes {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} is not canonical JSON"),
        ));
    }
    Ok(value)
}

fn validate_run_manifest_authority(
    manifest: &DemoRunManifestReadback,
    label: &'static str,
) -> Result<(), DemoError> {
    if manifest.schema != DEMO_RUN_MANIFEST_SCHEMA
        || manifest.status != "PREPUBLICATION_VALIDATED"
        || manifest.authority.profile != "DemoWikitextOnly"
        || manifest.authority.label != DEMO_WIKITEXT_MANIFEST_LABEL
        || manifest.authority.balanced_composite_claimed
        || manifest.authority.final_v3_training_authority_claimed
        || manifest.authority.other_family_gates_closed
        || manifest.source.tracked_manifest_schema != "ashira_v3_source_tracked_manifest_v1"
        || manifest.source.writer_version != "ashira-tokenizer-v3 0.1.0 demo-pipeline-v1"
        || manifest.source.effective_backend != "cpu"
        || manifest.source.toolchain_identity.is_empty()
        || !is_exact_hex(&manifest.source.commit, 40, false)
        || !is_exact_hex(&manifest.source.tree, 40, false)
        || !is_exact_hex(&manifest.source.tracked_manifest_sha256, 64, true)
        || manifest.corpus.manifest_schema != DEMO_WIKITEXT_MANIFEST_SCHEMA
        || manifest.corpus.manifest_label != DEMO_WIKITEXT_MANIFEST_LABEL
        || manifest.corpus.manifest_bytes == 0
        || !is_exact_hex(&manifest.corpus.manifest_sha256, 64, true)
        || !is_exact_hex(&manifest.corpus.manifest_sha512, 128, true)
        || manifest.corpus.admitted_files != 1
        || manifest.corpus.admitted_bytes == 0
        || manifest.training.input_files != 1
        || manifest.training.loaded_sequences == 0
        || manifest.training.loaded_tokens == 0
        || !is_exact_hex(&manifest.training.trainer_fnv64, 16, false)
        || manifest.package.directory != DEMO_PACKAGE_DIRECTORY
        || manifest.assertions.calibration_file != DEMO_CALIBRATION_FILENAME
        || manifest.assertions.probe_file != DEMO_PROBE_FILENAME
        || manifest.assertions.external_attribution_license_status
            != "required_before_public_submission_packaging"
        || !manifest.validation.writer_strict_readback
        || !manifest.validation.manifest_bound_load_before_publication
        || !manifest.validation.canonical_encoded_json_reparse
        || !manifest.validation.byte_round_trip
        || manifest.validation.final_package_reload_attestation != DEMO_FINAL_VALIDATION_FILENAME
        || !manifest.validation.file_sync_all
        || manifest.validation.directory_sync_claimed
        || manifest.validation.publication_visibility
            != "pre_rename_invisible_post_rename_visible_no_rollback_claim"
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} run manifest authority or validation boundary is invalid"),
        ));
    }
    Ok(())
}

fn validate_demo_config(
    config: &DemoConfigReadback,
    manifest: &DemoRunManifestReadback,
    bytes: &[u8],
    label: &'static str,
) -> Result<(), DemoError> {
    if config.schema != "ashira_v3_demo_training_config_v1"
        || config.manifest_schema != DEMO_WIKITEXT_MANIFEST_SCHEMA
        || config.manifest_label != DEMO_WIKITEXT_MANIFEST_LABEL
        || config.vocab_size != manifest.package.vocab_size
        || config.min_frequency != DEMO_MIN_FREQUENCY
        || !config.deterministic
        || config.family_weight_scaled != DEMO_FAMILY_WEIGHT_SCALED
        || config.effective_backend != "cpu"
        || config.demo_input_sha256 != hex_upper(&sha256(DEMO_INPUT))
        || manifest.config.file != DEMO_CONFIG_FILENAME
        || manifest.config.bytes != bytes.len() as u64
        || manifest.config.sha256 != hex_upper(&sha256(bytes))
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} deterministic training configuration evidence is invalid"),
        ));
    }
    Ok(())
}

fn validate_gate_assertion(
    assertion: &GateAssertionReadback,
    gate: &str,
    bytes: &[u8],
    expected_sha256: &str,
    label: &'static str,
) -> Result<(), DemoError> {
    if assertion.schema != "ashira_v3_demo_gate_assertion_v1"
        || assertion.gate != gate
        || assertion.status != "not_applicable"
        || assertion.reason != "single_family_demo_wikitext_only"
        || expected_sha256 != hex_upper(&sha256(bytes))
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} {gate} assertion evidence is invalid"),
        ));
    }
    Ok(())
}

fn validate_package_evidence(
    root: &Path,
    files: &[DemoRunFile],
    manifest: &DemoRunManifestReadback,
    package_manifest_bytes: &[u8],
    vocab_size: usize,
    merge_count: usize,
    label: &'static str,
) -> Result<(), DemoError> {
    let package = &manifest.package;
    if package.vocab_size as usize != vocab_size
        || package.merge_count as usize != merge_count
        || manifest.training.final_vocab != vocab_size
        || manifest.training.learned_merges != merge_count
        || package.manifest_bytes != package_manifest_bytes.len() as u64
        || package.manifest_sha256 != hex_upper(&sha256(package_manifest_bytes))
        || package.manifest_sha512 != hex_upper(&sha512(package_manifest_bytes))
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} package count or manifest evidence is invalid"),
        ));
    }

    let package_json: serde_json::Value =
        serde_json::from_slice(package_manifest_bytes).map_err(|error| {
            DemoError::input(
                "DemoCompare",
                format!("{label} package manifest parse failed: {error}"),
            )
        })?;
    for (pointer, expected) in [
        (
            "/provenance/deterministic_config_sha256",
            manifest.config.sha256.as_str(),
        ),
        (
            "/provenance/corpus_manifest_sha256",
            manifest.corpus.manifest_sha256.as_str(),
        ),
        (
            "/provenance/calibration_report_sha256",
            manifest.assertions.calibration_sha256.as_str(),
        ),
        (
            "/provenance/probe_selection_sha256",
            manifest.assertions.probe_sha256.as_str(),
        ),
        ("/provenance/source_commit", manifest.source.commit.as_str()),
        ("/provenance/source_tree", manifest.source.tree.as_str()),
        (
            "/provenance/source_tracked_files_sha256",
            manifest.source.tracked_manifest_sha256.as_str(),
        ),
        (
            "/provenance/writer_version",
            manifest.source.writer_version.as_str(),
        ),
        (
            "/provenance/toolchain_identity",
            manifest.source.toolchain_identity.as_str(),
        ),
        (
            "/provenance/effective_backend",
            manifest.source.effective_backend.as_str(),
        ),
    ] {
        if package_json
            .pointer(pointer)
            .and_then(serde_json::Value::as_str)
            != Some(expected)
        {
            return Err(DemoError::input(
                "DemoCompare",
                format!("{label} package provenance mismatch at {pointer}"),
            ));
        }
    }

    let limits = demo_artifact_limits();
    let vocab_metadata = inspect_artifact(
        &root.join("package/vocab.bin"),
        ArtifactFormat::V3U32,
        ArtifactKind::Vocab,
        &limits,
    )
    .map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("{label} vocab artifact inspection failed: {error}"),
        )
    })?;
    let merge_metadata = inspect_artifact(
        &root.join("package/merges.bin"),
        ArtifactFormat::V3U32,
        ArtifactKind::Merges,
        &limits,
    )
    .map_err(|error| {
        DemoError::input(
            "DemoCompare",
            format!("{label} merge artifact inspection failed: {error}"),
        )
    })?;
    validate_artifact_evidence(
        &package.vocab,
        &vocab_metadata,
        demo_run_file(files, "package/vocab.bin")?,
        u64::from(package.vocab_size),
        label,
        "vocab",
    )?;
    validate_artifact_evidence(
        &package.merges,
        &merge_metadata,
        demo_run_file(files, "package/merges.bin")?,
        package.merge_count,
        label,
        "merges",
    )
}

fn validate_artifact_evidence(
    evidence: &DemoArtifactReadback,
    metadata: &crate::ArtifactMetadata,
    bytes: &[u8],
    expected_records: u64,
    label: &'static str,
    artifact: &str,
) -> Result<(), DemoError> {
    if evidence.file_bytes != bytes.len() as u64
        || evidence.file_bytes != metadata.file_bytes
        || evidence.payload_bytes != metadata.payload_bytes
        || evidence.record_count != expected_records
        || evidence.record_count != metadata.record_count
        || evidence.file_sha256 != hex_upper(&sha256(bytes))
        || evidence.file_sha512 != hex_upper(&sha512(bytes))
        || metadata
            .payload_sha256
            .map(|digest| hex_upper(&digest))
            .as_deref()
            != Some(evidence.payload_sha256.as_str())
        || metadata
            .sequence_sha256
            .map(|digest| hex_upper(&digest))
            .as_deref()
            != Some(evidence.sequence_sha256.as_str())
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} {artifact} complete-file or payload evidence is invalid"),
        ));
    }
    Ok(())
}

fn validate_round_trip_evidence(
    manifest: &DemoRunManifestReadback,
    input: &[u8],
    encoded: &[u8],
    decoded: &[u8],
    token_count: usize,
    label: &'static str,
) -> Result<(), DemoError> {
    let round_trip = &manifest.round_trip;
    if round_trip.input_file != DEMO_INPUT_FILENAME
        || round_trip.input_bytes != input.len() as u64
        || round_trip.input_sha256 != hex_upper(&sha256(input))
        || round_trip.encoded_file != DEMO_ENCODED_FILENAME
        || round_trip.encoded_bytes != encoded.len() as u64
        || round_trip.encoded_sha256 != hex_upper(&sha256(encoded))
        || round_trip.token_count != token_count as u64
        || round_trip.decoded_file != DEMO_DECODED_FILENAME
        || round_trip.decoded_bytes != decoded.len() as u64
        || round_trip.decoded_sha256 != hex_upper(&sha256(decoded))
        || !round_trip.byte_equal
    {
        return Err(DemoError::input(
            "DemoCompare",
            format!("{label} round-trip evidence is invalid"),
        ));
    }
    Ok(())
}

fn is_exact_hex(value: &str, length: usize, uppercase: bool) -> bool {
    value.len() == length
        && value.bytes().all(|byte| {
            byte.is_ascii_digit()
                || if uppercase {
                    (b'A'..=b'F').contains(&byte)
                } else {
                    (b'a'..=b'f').contains(&byte)
                }
        })
}

pub fn run_demo_pipeline(
    manifest_path: &Path,
    run_root: &Path,
    vocab_size: usize,
) -> Result<DemoPipelineResult, DemoError> {
    let destination = validate_run_destination(run_root)?;
    let source = resolve_clean_source_authority()?;
    execute_demo_pipeline(manifest_path, destination, vocab_size, source)
}

fn execute_demo_pipeline(
    manifest_path: &Path,
    destination: RunDestination,
    vocab_size: usize,
    source: SourceAuthority,
) -> Result<DemoPipelineResult, DemoError> {
    let vocab_size_u32 = validate_vocab_target(vocab_size)
        .map_err(|error| DemoError::input("VocabSize", error.to_string()))?;
    if vocab_size > DEMO_MAX_VOCAB_SIZE {
        return Err(DemoError::input(
            "VocabSize",
            format!(
                "demo vocabulary target {vocab_size} exceeds bounded maximum {DEMO_MAX_VOCAB_SIZE}"
            ),
        ));
    }
    ensure_absent(&destination.final_path)?;

    let manifest_parent = manifest_path.parent().ok_or_else(|| {
        DemoError::input("ManifestPath", "demo manifest must have a parent directory")
    })?;
    let corpus_root = manifest_parent.join(DEMO_CORPUS_DIRECTORY);
    let admitted = admit_demo_wikitext_manifest(
        manifest_path,
        &[ManifestRoot {
            id: DEMO_CORPUS_ROOT_ID,
            path: &corpus_root,
        }],
    )
    .map_err(|error| DemoError::input("ManifestAdmission", error.to_string()))?;
    if admitted.profile() != ManifestAdmissionProfile::DemoWikitextOnly
        || admitted.files().len() != 1
    {
        return Err(DemoError::input(
            "ManifestAuthority",
            "demo admission did not return the exact WikiText-only profile",
        ));
    }

    let manifest_bytes = read_bounded_file(manifest_path, 1024 * 1024, "demo manifest")?;
    let manifest_sha256 = sha256(&manifest_bytes);
    let manifest_sha512 = sha512(&manifest_bytes);
    if manifest_sha256 != admitted.manifest_sha256()
        || manifest_sha512 != admitted.manifest_sha512()
    {
        return Err(DemoError::input(
            "ManifestMutation",
            "demo manifest changed after admission",
        ));
    }

    let config_wire = DemoConfigWire {
        schema: "ashira_v3_demo_training_config_v1",
        manifest_schema: DEMO_WIKITEXT_MANIFEST_SCHEMA,
        manifest_label: DEMO_WIKITEXT_MANIFEST_LABEL,
        vocab_size: vocab_size_u32,
        min_frequency: DEMO_MIN_FREQUENCY,
        deterministic: true,
        family_weight_scaled: DEMO_FAMILY_WEIGHT_SCALED,
        effective_backend: "cpu",
        demo_input_sha256: hex_upper(&sha256(DEMO_INPUT)),
    };
    let config_bytes = canonical_json(&config_wire, "demo training configuration")?;
    let config_sha256 = sha256(&config_bytes);
    let calibration_bytes = gate_assertion("calibration")?;
    let probe_bytes = gate_assertion("probe_selection")?;
    let calibration_sha256 = sha256(&calibration_bytes);
    let probe_sha256 = sha256(&probe_bytes);

    let weights = FamilyWeights::try_new(
        DEMO_FAMILY_WEIGHT_SCALED,
        DEMO_FAMILY_WEIGHT_SCALED,
        DEMO_FAMILY_WEIGHT_SCALED,
        DEMO_FAMILY_WEIGHT_SCALED,
    )
    .map_err(|error| DemoError::training("FamilyWeights", error.to_string()))?;
    let files = admitted.training_files(weights);
    let config = TrainConfig {
        vocab_size,
        min_frequency: DEMO_MIN_FREQUENCY,
        deterministic: true,
    };
    let mut trainer = TokenizerTrainer::new();
    let stats = trainer
        .train_weighted(&files, &config)
        .map_err(|error| DemoError::training("Training", error))?;
    let trainer_hash = trainer.compute_hash_hex();
    let tokenizer = trainer
        .freeze()
        .map_err(|error| DemoError::training("Freeze", error.to_string()))?;

    let context = PublicationContext::try_from_input(PublicationContextInput {
        run_id: "demo-wikitext-only-v1",
        checkpoint_id: "demo-checkpoint-v1",
        parent_checkpoint_id: None,
        deterministic_config_sha256: config_sha256,
        corpus_manifest_sha256: manifest_sha256,
        calibration_report_sha256: calibration_sha256,
        probe_selection_sha256: probe_sha256,
        source_commit: &source.commit,
        source_tree: &source.tree,
        source_tracked_files_sha256: source.tracked_manifest_sha256,
        writer_version: "ashira-tokenizer-v3 0.1.0 demo-pipeline-v1",
        toolchain_identity: &source.toolchain_identity,
        readback_evidence_id: "demo-runtime-readback-v1",
        prefix_proof_evidence_id: "demo-prefix-not-applicable-v1",
        effective_backend: "cpu",
    })
    .map_err(|error| DemoError::publication("PublicationContext", error.to_string()))?;

    if source.live {
        let repeated = resolve_clean_source_authority()?;
        if repeated != source {
            return Err(DemoError::input(
                "SourceAuthority",
                "source authority changed between training and publication",
            ));
        }
    }

    ensure_absent(&destination.final_path)?;
    let staging = create_staging_directory(&destination)?;
    write_synced_create_new(&staging.join(DEMO_CONFIG_FILENAME), &config_bytes)?;
    write_synced_create_new(&staging.join(DEMO_CALIBRATION_FILENAME), &calibration_bytes)?;
    write_synced_create_new(&staging.join(DEMO_PROBE_FILENAME), &probe_bytes)?;
    write_synced_create_new(&staging.join(DEMO_INPUT_FILENAME), DEMO_INPUT)?;

    let package_path = staging.join(DEMO_PACKAGE_DIRECTORY);
    let published = write_v3_package(&tokenizer, &package_path, &context)
        .map_err(|error| DemoError::publication("PackagePublication", error.to_string()))?;
    let artifact_limits = demo_artifact_limits();
    let loaded = load_v3_tokenizer_package(&package_path, &artifact_limits)
        .map_err(|error| DemoError::publication("PackageValidation", error.to_string()))?;
    if loaded != tokenizer {
        return Err(DemoError::publication(
            "PackageValidation",
            "manifest-bound package load differs from frozen tokenizer",
        ));
    }

    let codec_limits = demo_codec_limits();
    let encoded = EncodedTokensV1::encode(&loaded, DEMO_INPUT, &codec_limits)
        .map_err(|error| DemoError::training("Encode", error.to_string()))?;
    let token_count = u64::try_from(encoded.token_ids().len())
        .map_err(|_| DemoError::training("Encode", "token count conversion overflow"))?;
    let encoded_bytes = encoded
        .to_canonical_json(&codec_limits)
        .map_err(|error| DemoError::training("Encode", error.to_string()))?;
    let reparsed = EncodedTokensV1::parse_json(&encoded_bytes, &codec_limits)
        .map_err(|error| DemoError::training("EncodedReadback", error.to_string()))?;
    let decoded = reparsed
        .decode(&loaded, &codec_limits)
        .map_err(|error| DemoError::training("Decode", error.to_string()))?;
    if decoded != DEMO_INPUT {
        return Err(DemoError::training(
            "RoundTrip",
            "decoded demo bytes differ from the fixed input",
        ));
    }
    write_synced_create_new(&staging.join(DEMO_ENCODED_FILENAME), &encoded_bytes)?;
    write_synced_create_new(&staging.join(DEMO_DECODED_FILENAME), &decoded)?;

    let report = DemoRunManifestWire {
        schema: DEMO_RUN_MANIFEST_SCHEMA,
        status: "PREPUBLICATION_VALIDATED",
        authority: DemoAuthorityWire {
            profile: "DemoWikitextOnly",
            label: DEMO_WIKITEXT_MANIFEST_LABEL,
            balanced_composite_claimed: false,
            final_v3_training_authority_claimed: false,
            other_family_gates_closed: false,
        },
        source: DemoSourceWire {
            commit: source.commit.clone(),
            tree: source.tree.clone(),
            tracked_manifest_schema: "ashira_v3_source_tracked_manifest_v1",
            tracked_manifest_sha256: hex_upper(&source.tracked_manifest_sha256),
            writer_version: "ashira-tokenizer-v3 0.1.0 demo-pipeline-v1",
            toolchain_identity: source.toolchain_identity.clone(),
            effective_backend: "cpu",
        },
        corpus: DemoCorpusWire {
            manifest_schema: DEMO_WIKITEXT_MANIFEST_SCHEMA,
            manifest_label: DEMO_WIKITEXT_MANIFEST_LABEL,
            manifest_bytes: u64::try_from(manifest_bytes.len()).map_err(|_| {
                DemoError::publication("ManifestEvidence", "manifest length conversion overflow")
            })?,
            manifest_sha256: hex_upper(&manifest_sha256),
            manifest_sha512: hex_upper(&manifest_sha512),
            admitted_files: admitted.files().len(),
            admitted_bytes: admitted.total_bytes(),
        },
        config: DemoConfigEvidenceWire {
            file: DEMO_CONFIG_FILENAME,
            bytes: u64::try_from(config_bytes.len()).map_err(|_| {
                DemoError::publication("ConfigEvidence", "config length conversion overflow")
            })?,
            sha256: hex_upper(&config_sha256),
        },
        training: DemoTrainingWire {
            input_files: stats.input_files,
            loaded_sequences: stats.loaded_sequences,
            skipped_lines: stats.skipped_lines,
            loaded_tokens: stats.loaded_tokens,
            learned_merges: stats.learned_merges,
            final_vocab: stats.final_vocab,
            trainer_fnv64: trainer_hash,
        },
        package: DemoPackageWire {
            directory: DEMO_PACKAGE_DIRECTORY,
            vocab_size: published.vocab_size(),
            merge_count: published.merge_count(),
            vocab: artifact_wire(published.vocab_evidence()),
            merges: artifact_wire(published.merges_evidence()),
            manifest_bytes: published.manifest_bytes(),
            manifest_sha256: hex_upper(&published.manifest_sha256()),
            manifest_sha512: hex_upper(&published.manifest_sha512()),
        },
        round_trip: DemoRoundTripWire {
            input_file: DEMO_INPUT_FILENAME,
            input_bytes: u64::try_from(DEMO_INPUT.len()).map_err(|_| {
                DemoError::publication("RoundTripEvidence", "input length conversion overflow")
            })?,
            input_sha256: hex_upper(&sha256(DEMO_INPUT)),
            encoded_file: DEMO_ENCODED_FILENAME,
            encoded_bytes: u64::try_from(encoded_bytes.len()).map_err(|_| {
                DemoError::publication("RoundTripEvidence", "encoded length conversion overflow")
            })?,
            encoded_sha256: hex_upper(&sha256(&encoded_bytes)),
            token_count,
            decoded_file: DEMO_DECODED_FILENAME,
            decoded_bytes: u64::try_from(decoded.len()).map_err(|_| {
                DemoError::publication("RoundTripEvidence", "decoded length conversion overflow")
            })?,
            decoded_sha256: hex_upper(&sha256(&decoded)),
            byte_equal: true,
        },
        assertions: DemoAssertionsWire {
            calibration_file: DEMO_CALIBRATION_FILENAME,
            calibration_sha256: hex_upper(&calibration_sha256),
            probe_file: DEMO_PROBE_FILENAME,
            probe_sha256: hex_upper(&probe_sha256),
            external_attribution_license_status: "required_before_public_submission_packaging",
        },
        validation: DemoValidationWire {
            writer_strict_readback: true,
            manifest_bound_load_before_publication: true,
            canonical_encoded_json_reparse: true,
            byte_round_trip: true,
            final_package_reload_attestation: DEMO_FINAL_VALIDATION_FILENAME,
            file_sync_all: true,
            directory_sync_claimed: false,
            publication_visibility: "pre_rename_invisible_post_rename_visible_no_rollback_claim",
        },
    };
    let report_bytes = canonical_json(&report, "demo run manifest")?;
    write_synced_create_new(&staging.join(DEMO_RUN_MANIFEST_FILENAME), &report_bytes)?;
    let report_readback = read_bounded_file(
        &staging.join(DEMO_RUN_MANIFEST_FILENAME),
        1024 * 1024,
        "staged demo run manifest",
    )?;
    if report_readback != report_bytes {
        return Err(DemoError::publication(
            "RunReadback",
            "staged demo run manifest differs after sync",
        ));
    }

    ensure_absent(&destination.final_path)?;
    rename_staging_no_replace(&staging, &destination.final_path)?;

    let final_report = read_bounded_file(
        &destination.final_path.join(DEMO_RUN_MANIFEST_FILENAME),
        1024 * 1024,
        "published demo run manifest",
    )?;
    if final_report != report_bytes {
        return Err(DemoError::publication(
            "FinalReadback",
            "published demo run manifest differs after rename",
        ));
    }
    let final_loaded = load_v3_tokenizer_package(
        &destination.final_path.join(DEMO_PACKAGE_DIRECTORY),
        &artifact_limits,
    )
    .map_err(|error| DemoError::publication("FinalPackageValidation", error.to_string()))?;
    if final_loaded != loaded {
        return Err(DemoError::publication(
            "FinalPackageValidation",
            "final package reload differs from staged package",
        ));
    }
    for (filename, expected, label) in [
        (
            DEMO_CONFIG_FILENAME,
            config_bytes.as_slice(),
            "published demo training config",
        ),
        (
            DEMO_CALIBRATION_FILENAME,
            calibration_bytes.as_slice(),
            "published demo calibration assertion",
        ),
        (
            DEMO_PROBE_FILENAME,
            probe_bytes.as_slice(),
            "published demo probe assertion",
        ),
        (DEMO_INPUT_FILENAME, DEMO_INPUT, "published demo input"),
        (
            DEMO_ENCODED_FILENAME,
            encoded_bytes.as_slice(),
            "published demo encoded JSON",
        ),
        (
            DEMO_DECODED_FILENAME,
            decoded.as_slice(),
            "published demo decoded bytes",
        ),
    ] {
        let limit = u64::try_from(expected.len()).map_err(|_| {
            DemoError::publication("FinalReadback", "final file length conversion overflow")
        })?;
        let actual = read_bounded_file(&destination.final_path.join(filename), limit, label)?;
        if actual != expected {
            return Err(DemoError::publication(
                "FinalReadback",
                format!("published run file differs after rename: {filename}"),
            ));
        }
    }

    let report_sha256 = sha256(&report_bytes);
    let round_trip_sha256 = sha256(&decoded);
    let final_validation_bytes = canonical_json(
        &DemoFinalValidationWire {
            schema: "ashira_v3_demo_final_validation_v1",
            status: "PASS",
            run_manifest_sha256: hex_upper(&report_sha256),
            package_manifest_sha256: hex_upper(&published.manifest_sha256()),
            round_trip_sha256: hex_upper(&round_trip_sha256),
            final_package_reload: true,
            final_report_readback: true,
            final_run_files_readback: true,
        },
        "demo final validation",
    )?;
    let final_validation_path = destination.final_path.join(DEMO_FINAL_VALIDATION_FILENAME);
    write_synced_create_new(&final_validation_path, &final_validation_bytes)?;
    let final_validation_readback = read_bounded_file(
        &final_validation_path,
        1024 * 1024,
        "published demo final validation",
    )?;
    if final_validation_readback != final_validation_bytes {
        return Err(DemoError::publication(
            "FinalReadback",
            "published final validation differs after sync",
        ));
    }

    let mut deterministic_core = Sha256::new();
    deterministic_core.update(b"ashira_v3_demo_deterministic_core_v1\0");
    deterministic_core.update(&report_bytes);
    deterministic_core.update(&final_validation_bytes);

    Ok(DemoPipelineResult {
        vocab_size: published.vocab_size(),
        merge_count: published.merge_count(),
        token_count,
        package_manifest_sha256: published.manifest_sha256(),
        deterministic_core_sha256: finalize_sha256(deterministic_core.finalize()),
        round_trip_sha256,
    })
}

fn gate_assertion(gate: &'static str) -> Result<Vec<u8>, DemoError> {
    canonical_json(
        &GateAssertionWire {
            schema: "ashira_v3_demo_gate_assertion_v1",
            gate,
            status: "not_applicable",
            reason: "single_family_demo_wikitext_only",
        },
        "demo gate assertion",
    )
}

fn canonical_json(value: &impl Serialize, label: &'static str) -> Result<Vec<u8>, DemoError> {
    let mut bytes = serde_json::to_vec(value)
        .map_err(|error| DemoError::publication("CanonicalJson", format!("{label}: {error}")))?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn artifact_wire(evidence: ArtifactFileEvidenceInput) -> DemoArtifactWire {
    DemoArtifactWire {
        file_bytes: evidence.file_bytes,
        payload_bytes: evidence.payload_bytes,
        record_count: evidence.record_count,
        file_sha256: hex_upper(&evidence.file_sha256),
        file_sha512: hex_upper(&evidence.file_sha512),
        payload_sha256: hex_upper(&evidence.payload_sha256),
        sequence_sha256: hex_upper(&evidence.sequence_sha256),
    }
}

fn demo_artifact_limits() -> ArtifactLimits {
    ArtifactLimits {
        max_file_bytes: 512 * 1024 * 1024,
        max_total_vocab_bytes: 256 * 1024 * 1024,
        max_token_bytes: 16 * 1024 * 1024,
    }
}

fn demo_codec_limits() -> CodecLimits {
    CodecLimits {
        max_input_bytes: 8 * 1024 * 1024,
        max_token_count: 8 * 1024 * 1024,
        max_decoded_bytes: 32 * 1024 * 1024,
        max_encoded_json_bytes: 128 * 1024 * 1024,
    }
}

fn validate_run_destination(path: &Path) -> Result<RunDestination, DemoError> {
    let filename = path
        .file_name()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| DemoError::publication("RunPath", "run-root filename is invalid"))?
        .to_os_string();
    let parent_input = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_absolute = absolute_path(parent_input)?;
    ensure_no_link_like_ancestors(&parent_absolute)?;
    let parent = fs::canonicalize(&parent_absolute).map_err(|error| {
        DemoError::publication(
            "RunPath",
            format!("cannot resolve run-root parent: {error}"),
        )
    })?;
    ensure_no_link_like_ancestors(&parent)?;
    let metadata = fs::symlink_metadata(&parent).map_err(|error| {
        DemoError::publication(
            "RunPath",
            format!("cannot inspect run-root parent: {error}"),
        )
    })?;
    if metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(DemoError::publication(
            "RunPath",
            "run-root parent is not a non-link directory",
        ));
    }
    let final_path = parent.join(&filename);
    let source_root = fs::canonicalize(env!("CARGO_MANIFEST_DIR")).map_err(|error| {
        DemoError::input(
            "SourceAuthority",
            format!("cannot resolve compiled source root: {error}"),
        )
    })?;
    if final_path.starts_with(&source_root) {
        return Err(DemoError::publication(
            "RunPath",
            "run-root must be outside the Git source tree so A/B runs do not dirty authority",
        ));
    }
    ensure_absent(&final_path)?;
    Ok(RunDestination {
        parent,
        final_path,
        filename,
    })
}

fn create_staging_directory(destination: &RunDestination) -> Result<PathBuf, DemoError> {
    for _ in 0..MAX_STAGING_ATTEMPTS {
        let ordinal = NEXT_STAGING_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(&destination.filename);
        name.push(format!(
            ".ashira-demo-staging-{}-{ordinal:016X}",
            std::process::id()
        ));
        let staging = destination.parent.join(name);
        match fs::create_dir(&staging) {
            Ok(()) => return Ok(staging),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(DemoError::publication(
                    "RunStaging",
                    format!("cannot create run staging directory: {error}"),
                ));
            }
        }
    }
    Err(DemoError::publication(
        "RunStaging",
        "bounded run staging attempts exhausted",
    ))
}

fn ensure_absent(path: &Path) -> Result<(), DemoError> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(DemoError::publication(
            "ExistingRunRoot",
            format!("run-root already exists: {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(DemoError::publication(
            "RunPath",
            format!("cannot inspect run-root: {error}"),
        )),
    }
}

fn write_synced_create_new(path: &Path, bytes: &[u8]) -> Result<(), DemoError> {
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|error| {
            DemoError::publication(
                "RunWrite",
                format!("cannot create {}: {error}", path.display()),
            )
        })?;
    let mut writer = BufWriter::new(file);
    writer.write_all(bytes).map_err(|error| {
        DemoError::publication(
            "RunWrite",
            format!("cannot write {}: {error}", path.display()),
        )
    })?;
    writer.flush().map_err(|error| {
        DemoError::publication(
            "RunWrite",
            format!("cannot flush {}: {error}", path.display()),
        )
    })?;
    writer.get_ref().sync_all().map_err(|error| {
        DemoError::publication(
            "RunWrite",
            format!("cannot sync {}: {error}", path.display()),
        )
    })?;
    drop(writer);
    let readback = read_bounded_file(path, bytes.len() as u64, "new demo run file")?;
    if readback != bytes {
        return Err(DemoError::publication(
            "RunReadback",
            format!("new run file readback mismatch: {}", path.display()),
        ));
    }
    Ok(())
}

fn read_bounded_file(path: &Path, limit: u64, label: &'static str) -> Result<Vec<u8>, DemoError> {
    ensure_no_link_like_ancestors(path)?;
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        DemoError::input("FileRead", format!("cannot inspect {label}: {error}"))
    })?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() || metadata.len() > limit {
        return Err(DemoError::input(
            "FileRead",
            format!("{label} is not a bounded regular file"),
        ));
    }
    let mut file = File::open(path)
        .map_err(|error| DemoError::input("FileRead", format!("cannot open {label}: {error}")))?;
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| DemoError::input("FileRead", format!("cannot allocate bounded {label}")))?;
    Read::by_ref(&mut file)
        .take(limit.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|error| DemoError::input("FileRead", format!("cannot read {label}: {error}")))?;
    if bytes.len() as u64 != metadata.len() || bytes.len() as u64 > limit {
        return Err(DemoError::input(
            "FileRead",
            format!("{label} changed or exceeded its bound"),
        ));
    }
    Ok(bytes)
}

fn resolve_clean_source_authority() -> Result<SourceAuthority, DemoError> {
    let root = Path::new(env!("CARGO_MANIFEST_DIR"));
    let status = git_output(root, &["status", "--porcelain=v1", "--untracked-files=all"])?;
    if !status.is_empty() {
        return Err(DemoError::input(
            "SourceAuthority",
            "source worktree/index is dirty; commit the exact reviewed demo candidate first",
        ));
    }
    let commit = git_text(root, &["rev-parse", "HEAD"])?;
    let tree = git_text(root, &["rev-parse", "HEAD^{tree}"])?;
    let tracked = git_output(root, &["ls-files", "-z"])?;
    let tracked_manifest_sha256 = hash_tracked_manifest(root, &tracked)?;
    let toolchain_identity = command_text("rustc", &["--version"])?;
    Ok(SourceAuthority {
        commit,
        tree,
        tracked_manifest_sha256,
        toolchain_identity,
        live: true,
    })
}

fn git_output(root: &Path, args: &[&str]) -> Result<Vec<u8>, DemoError> {
    let output = Command::new("git")
        .arg("-C")
        .arg(root)
        .args(args)
        .output()
        .map_err(|error| {
            DemoError::input("SourceAuthority", format!("cannot execute git: {error}"))
        })?;
    if !output.status.success() {
        return Err(DemoError::input(
            "SourceAuthority",
            format!("git command failed with {}", output.status),
        ));
    }
    Ok(output.stdout)
}

fn git_text(root: &Path, args: &[&str]) -> Result<String, DemoError> {
    let bytes = git_output(root, args)?;
    let text = std::str::from_utf8(&bytes)
        .map_err(|_| DemoError::input("SourceAuthority", "git output is not UTF-8"))?
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(DemoError::input(
            "SourceAuthority",
            "git returned an empty authority value",
        ));
    }
    Ok(text)
}

fn command_text(program: &str, args: &[&str]) -> Result<String, DemoError> {
    let output = Command::new(program).args(args).output().map_err(|error| {
        DemoError::input(
            "ToolchainAuthority",
            format!("cannot execute {program}: {error}"),
        )
    })?;
    if !output.status.success() {
        return Err(DemoError::input(
            "ToolchainAuthority",
            format!("{program} failed with {}", output.status),
        ));
    }
    let text = std::str::from_utf8(&output.stdout)
        .map_err(|_| DemoError::input("ToolchainAuthority", "toolchain output is not UTF-8"))?
        .trim()
        .to_owned();
    if text.is_empty() {
        return Err(DemoError::input(
            "ToolchainAuthority",
            "toolchain identity is empty",
        ));
    }
    Ok(text)
}

fn hash_tracked_manifest(root: &Path, raw_paths: &[u8]) -> Result<[u8; 32], DemoError> {
    let mut paths: Vec<&[u8]> = raw_paths
        .split(|byte| *byte == 0)
        .filter(|path| !path.is_empty())
        .collect();
    if paths.is_empty() || paths.len() > MAX_TRACKED_FILES {
        return Err(DemoError::input(
            "SourceAuthority",
            "tracked file count is empty or exceeds its bound",
        ));
    }
    paths.sort_unstable();
    if paths.windows(2).any(|pair| pair[0] == pair[1]) {
        return Err(DemoError::input(
            "SourceAuthority",
            "tracked file list contains duplicates",
        ));
    }
    let mut aggregate = Sha256::new();
    let mut total_bytes = 0u64;
    for raw_path in paths {
        let relative = std::str::from_utf8(raw_path).map_err(|_| {
            DemoError::input("SourceAuthority", "tracked path is not normalized UTF-8")
        })?;
        validate_tracked_path(relative)?;
        let path = root.join(relative);
        ensure_no_link_like_ancestors(&path)?;
        let metadata = fs::symlink_metadata(&path).map_err(|error| {
            DemoError::input(
                "SourceAuthority",
                format!("cannot inspect tracked file {relative}: {error}"),
            )
        })?;
        if metadata_is_link_like(&metadata)
            || !metadata.is_file()
            || metadata.len() > MAX_TRACKED_FILE_BYTES
        {
            return Err(DemoError::input(
                "SourceAuthority",
                format!("tracked file is not a bounded regular file: {relative}"),
            ));
        }
        total_bytes = total_bytes
            .checked_add(metadata.len())
            .ok_or_else(|| DemoError::input("SourceAuthority", "tracked byte total overflow"))?;
        if total_bytes > MAX_TRACKED_TOTAL_BYTES {
            return Err(DemoError::input(
                "SourceAuthority",
                "tracked byte total exceeds its bound",
            ));
        }
        let bytes = fs::read(&path).map_err(|error| {
            DemoError::input(
                "SourceAuthority",
                format!("cannot read tracked file {relative}: {error}"),
            )
        })?;
        if bytes.len() as u64 != metadata.len() {
            return Err(DemoError::input(
                "SourceAuthority",
                format!("tracked file changed during read: {relative}"),
            ));
        }
        let record = format!(
            "{}  {}  {}\n",
            hex_lower(&sha256(&bytes)),
            metadata.len(),
            relative
        );
        aggregate.update(record.as_bytes());
    }
    Ok(finalize_sha256(aggregate.finalize()))
}

fn validate_tracked_path(path: &str) -> Result<(), DemoError> {
    let parsed = Path::new(path);
    if path.is_empty()
        || path.contains(['\\', '\0'])
        || parsed.is_absolute()
        || parsed
            .components()
            .any(|component| !matches!(component, Component::Normal(_)))
    {
        return Err(DemoError::input(
            "SourceAuthority",
            format!("tracked path is not normalized: {path}"),
        ));
    }
    Ok(())
}

fn absolute_path(path: &Path) -> Result<PathBuf, DemoError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|error| {
                DemoError::publication("RunPath", format!("cannot resolve current dir: {error}"))
            })?
            .join(path))
    }
}

fn ensure_no_link_like_ancestors(path: &Path) -> Result<(), DemoError> {
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        match fs::symlink_metadata(ancestor) {
            Ok(metadata) if metadata_is_link_like(&metadata) => {
                return Err(DemoError::input(
                    "LinkRejected",
                    format!("link/reparse path rejected: {}", ancestor.display()),
                ));
            }
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => {
                return Err(DemoError::input(
                    "PathInspection",
                    format!("cannot inspect {}: {error}", ancestor.display()),
                ));
            }
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

#[cfg(windows)]
fn rename_staging_no_replace(staging: &Path, destination: &Path) -> Result<(), DemoError> {
    ensure_absent(destination)?;
    match fs::rename(staging, destination) {
        Ok(()) => Ok(()),
        Err(error) if fs::symlink_metadata(destination).is_ok() => Err(DemoError::publication(
            "ExistingRunRoot",
            format!("run-root appeared during publication: {error}"),
        )),
        Err(error) => Err(DemoError::publication(
            "RunPublication",
            format!("same-parent run-root rename failed: {error}"),
        )),
    }
}

#[cfg(not(windows))]
fn rename_staging_no_replace(_staging: &Path, _destination: &Path) -> Result<(), DemoError> {
    Err(DemoError::publication(
        "Unsupported",
        "non-overwrite run-root directory rename is not proven on this platform",
    ))
}

fn sha256(bytes: &[u8]) -> [u8; 32] {
    finalize_sha256(Sha256::digest(bytes))
}

fn sha512(bytes: &[u8]) -> [u8; 64] {
    let digest = Sha512::digest(bytes);
    let mut output = [0u8; 64];
    output.copy_from_slice(&digest);
    output
}

fn finalize_sha256(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_ref());
    output
}

fn hex_upper(bytes: &[u8]) -> String {
    hex(bytes, b"0123456789ABCDEF")
}

fn hex_lower(bytes: &[u8]) -> String {
    hex(bytes, b"0123456789abcdef")
}

fn hex(bytes: &[u8], alphabet: &[u8; 16]) -> String {
    let mut output = String::with_capacity(bytes.len().saturating_mul(2));
    for byte in bytes {
        output.push(char::from(alphabet[usize::from(byte >> 4)]));
        output.push(char::from(alphabet[usize::from(byte & 0x0F)]));
    }
    output
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_root(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let root = std::env::temp_dir().join(format!(
            "ashira_v3_demo_{}_{}_{label}",
            std::process::id(),
            ordinal
        ));
        fs::create_dir(&root).unwrap();
        root
    }

    fn source_authority() -> SourceAuthority {
        SourceAuthority {
            commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_owned(),
            tree: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_owned(),
            tracked_manifest_sha256: [0x55; 32],
            toolchain_identity: "rustc 1.94.0 test".to_owned(),
            live: false,
        }
    }

    fn demo_fixture(root: &Path) -> PathBuf {
        let demo = root.join("demo");
        let corpus = demo.join(DEMO_CORPUS_DIRECTORY);
        fs::create_dir_all(&corpus).unwrap();
        let corpus_bytes = b"aa aa aa\ndeterministic demo words words\n";
        fs::write(corpus.join("wikitext.txt"), corpus_bytes).unwrap();
        let manifest = format!(
            "{{\"schema\":\"{DEMO_WIKITEXT_MANIFEST_SCHEMA}\",\"label\":\"{DEMO_WIKITEXT_MANIFEST_LABEL}\",\"entries\":[{{\"ordinal\":1,\"family\":\"wikitext\",\"root_id\":\"{DEMO_CORPUS_ROOT_ID}\",\"relative_path\":\"wikitext.txt\",\"enabled\":true,\"bytes\":{},\"sha256\":\"{}\",\"sha512\":\"{}\",\"encoding_policy\":\"utf8\"}}]}}\n",
            corpus_bytes.len(),
            hex_upper(&sha256(corpus_bytes)),
            hex_upper(&sha512(corpus_bytes)),
        );
        let manifest_path = demo.join("demo_wikitext_manifest.json");
        fs::write(&manifest_path, manifest).unwrap();
        manifest_path
    }

    fn run_snapshot(root: &Path) -> Vec<(&'static str, Vec<u8>)> {
        DEMO_COMPARISON_FILES
            .iter()
            .map(|(relative, _)| (*relative, fs::read(root.join(relative)).unwrap()))
            .collect()
    }

    #[test]
    fn two_clean_demo_roots_produce_identical_deterministic_core() {
        let root = unique_root("ab");
        let manifest = demo_fixture(&root);
        let run_a = root.join("run_a");
        let run_b = root.join("run_b");
        let result_a = execute_demo_pipeline(
            &manifest,
            validate_run_destination(&run_a).unwrap(),
            277,
            source_authority(),
        )
        .unwrap();
        let result_b = execute_demo_pipeline(
            &manifest,
            validate_run_destination(&run_b).unwrap(),
            277,
            source_authority(),
        )
        .unwrap();

        assert_eq!(result_a, result_b);
        assert_eq!(
            fs::read(run_a.join(DEMO_RUN_MANIFEST_FILENAME)).unwrap(),
            fs::read(run_b.join(DEMO_RUN_MANIFEST_FILENAME)).unwrap()
        );
        for relative in [
            "package/vocab.bin",
            "package/merges.bin",
            "package/package_manifest.json",
            DEMO_CONFIG_FILENAME,
            DEMO_CALIBRATION_FILENAME,
            DEMO_PROBE_FILENAME,
            DEMO_INPUT_FILENAME,
            DEMO_ENCODED_FILENAME,
            DEMO_DECODED_FILENAME,
            DEMO_FINAL_VALIDATION_FILENAME,
        ] {
            assert_eq!(
                fs::read(run_a.join(relative)).unwrap(),
                fs::read(run_b.join(relative)).unwrap(),
                "A/B mismatch for {relative}"
            );
        }
        assert_eq!(
            fs::read(run_a.join(DEMO_DECODED_FILENAME)).unwrap(),
            DEMO_INPUT
        );

        let before_a = run_snapshot(&run_a);
        let before_b = run_snapshot(&run_b);
        let comparison = compare_demo_runs(&run_a, &run_b).unwrap();
        assert_eq!(comparison.file_count(), 11);
        assert_eq!(comparison.vocab_size(), result_a.vocab_size());
        assert_eq!(comparison.merge_count(), result_a.merge_count());
        assert_eq!(comparison.token_count(), result_a.token_count());
        assert_eq!(
            comparison.package_manifest_sha256(),
            result_a.package_manifest_sha256()
        );
        assert_eq!(
            comparison.deterministic_core_sha256(),
            result_a.deterministic_core_sha256()
        );
        assert_eq!(comparison.source_commit(), source_authority().commit);
        assert_eq!(run_snapshot(&run_a), before_a);
        assert_eq!(run_snapshot(&run_b), before_b);
    }

    #[test]
    fn demo_comparison_rejects_same_root_mismatch_and_extra_entry() {
        let root = unique_root("compare_failures");
        let manifest = demo_fixture(&root);
        let run_a = root.join("run_a");
        let run_b = root.join("run_b");
        execute_demo_pipeline(
            &manifest,
            validate_run_destination(&run_a).unwrap(),
            277,
            source_authority(),
        )
        .unwrap();
        execute_demo_pipeline(
            &manifest,
            validate_run_destination(&run_b).unwrap(),
            277,
            source_authority(),
        )
        .unwrap();

        let same = compare_demo_runs(&run_a, &run_a).unwrap_err();
        assert_eq!(same.class(), "DemoCompare");

        fs::write(run_b.join(DEMO_DECODED_FILENAME), b"tampered").unwrap();
        let mismatch = compare_demo_runs(&run_a, &run_b).unwrap_err();
        assert_eq!(mismatch.class(), "DemoCompare");

        fs::write(run_a.join("unexpected.txt"), b"unexpected").unwrap();
        let extra = compare_demo_runs(&run_a, &run_b).unwrap_err();
        assert_eq!(extra.class(), "DemoCompare");
        assert!(extra.to_string().contains("unexpected entries"));
    }

    #[cfg(windows)]
    #[test]
    fn demo_comparison_rejects_reparse_root_before_inventory() {
        use std::os::windows::fs::symlink_dir;

        let root = unique_root("compare_reparse");
        let real_a = root.join("real_a");
        let real_b = root.join("real_b");
        fs::create_dir(&real_a).unwrap();
        fs::create_dir(&real_b).unwrap();
        let linked_a = root.join("linked_a");
        symlink_dir(&real_a, &linked_a).unwrap();

        let error = compare_demo_runs(&linked_a, &real_b).unwrap_err();
        assert_eq!(error.class(), "LinkRejected");
    }

    #[test]
    fn existing_run_root_fails_before_source_or_manifest_work() {
        let root = unique_root("existing");
        let run_root = root.join("owned");
        fs::create_dir(&run_root).unwrap();
        fs::write(run_root.join("sentinel.txt"), b"operator-owned").unwrap();
        let error = run_demo_pipeline(
            &root.join("missing-manifest.json"),
            &run_root,
            DEMO_MAX_VOCAB_SIZE + 1,
        )
        .unwrap_err();
        assert_eq!(error.class(), "ExistingRunRoot");
        assert_eq!(
            fs::read(run_root.join("sentinel.txt")).unwrap(),
            b"operator-owned"
        );
    }
}
