use crate::demo::{compare_demo_runs, run_demo_pipeline};
use crate::{ArtifactLimits, CodecLimits, EncodedTokensV1, load_v3_tokenizer_package};
use sha2::{Digest, Sha256};
use std::ffi::OsString;
use std::fs::{self, File, Metadata, OpenOptions};
use std::io::{BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const D4_MAX_ARTIFACT_FILE_BYTES: u64 = 512 * 1024 * 1024;
pub const D4_MAX_TOTAL_VOCAB_BYTES: u64 = 256 * 1024 * 1024;
pub const D4_MAX_TOKEN_BYTES: u32 = 16 * 1024 * 1024;
pub const D4_MAX_INPUT_BYTES: u64 = 8 * 1024 * 1024;
pub const D4_MAX_TOKEN_COUNT: u64 = 8 * 1024 * 1024;
pub const D4_MAX_DECODED_BYTES: u64 = 32 * 1024 * 1024;
pub const D4_MAX_ENCODED_JSON_BYTES: u64 = 128 * 1024 * 1024;

const EXIT_USAGE: i32 = 2;
const EXIT_INPUT: i32 = 3;
const EXIT_CODEC: i32 = 4;
const EXIT_OUTPUT: i32 = 5;
const MAX_STAGING_ATTEMPTS: u64 = 1_024;

static NEXT_STAGING_ORDINAL: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Eq, PartialEq)]
struct CliLimits {
    artifact: ArtifactLimits,
    codec: CodecLimits,
}

impl CliLimits {
    fn build_week_demo() -> Self {
        Self {
            artifact: ArtifactLimits {
                max_file_bytes: D4_MAX_ARTIFACT_FILE_BYTES,
                max_total_vocab_bytes: D4_MAX_TOTAL_VOCAB_BYTES,
                max_token_bytes: D4_MAX_TOKEN_BYTES,
            },
            codec: CodecLimits {
                max_input_bytes: D4_MAX_INPUT_BYTES,
                max_token_count: D4_MAX_TOKEN_COUNT,
                max_decoded_bytes: D4_MAX_DECODED_BYTES,
                max_encoded_json_bytes: D4_MAX_ENCODED_JSON_BYTES,
            },
        }
    }
}

#[derive(Debug)]
enum CliCommand {
    Encode {
        package: PathBuf,
        text_file: PathBuf,
        output: PathBuf,
    },
    Decode {
        package: PathBuf,
        encoded: PathBuf,
        output: PathBuf,
    },
    DemoPipeline {
        manifest: PathBuf,
        run_root: PathBuf,
        vocab_size: usize,
    },
    DemoCompare {
        run_a: PathBuf,
        run_b: PathBuf,
    },
    Help,
}

#[derive(Debug)]
struct CommandFailure {
    exit_code: i32,
    class: &'static str,
    message: String,
}

impl CommandFailure {
    fn usage(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_USAGE,
            class: "Usage",
            message: message.into(),
        }
    }

    fn input(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_INPUT,
            class,
            message: message.into(),
        }
    }

    fn codec(message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_CODEC,
            class: "Codec",
            message: message.into(),
        }
    }

    fn output(class: &'static str, message: impl Into<String>) -> Self {
        Self {
            exit_code: EXIT_OUTPUT,
            class,
            message: message.into(),
        }
    }
}

#[derive(Clone, Copy)]
enum IoClass {
    Input,
    Output,
}

impl IoClass {
    fn failure(self, class: &'static str, message: impl Into<String>) -> CommandFailure {
        match self {
            Self::Input => CommandFailure::input(class, message),
            Self::Output => CommandFailure::output(class, message),
        }
    }
}

struct OutputDestination {
    parent: PathBuf,
    final_path: PathBuf,
    filename: OsString,
}

struct PublishedOutput {
    bytes: u64,
    sha256: [u8; 32],
}

struct FileCommandSuccess {
    action: &'static str,
    input_bytes: u64,
    token_count: u64,
    output: PublishedOutput,
}

enum CommandSuccess {
    File(FileCommandSuccess),
    Demo(crate::demo::DemoPipelineResult),
    Compare(crate::demo::DemoCompareResult),
}

pub fn run_cli(args: Vec<OsString>, stdout: &mut dyn Write, stderr: &mut dyn Write) -> i32 {
    run_cli_with_limits(args, stdout, stderr, &CliLimits::build_week_demo())
}

fn run_cli_with_limits(
    args: Vec<OsString>,
    stdout: &mut dyn Write,
    stderr: &mut dyn Write,
    limits: &CliLimits,
) -> i32 {
    match parse_command(&args) {
        Ok(CliCommand::Help) => match stdout.write_all(usage_bytes()) {
            Ok(()) => 0,
            Err(_) => EXIT_OUTPUT,
        },
        Ok(command) => match execute_command(command, limits) {
            Ok(CommandSuccess::File(success)) => {
                let digest = hex_upper(&success.output.sha256);
                match writeln!(
                    stdout,
                    "PASS {} input_bytes={} token_count={} output_bytes={} output_sha256={digest}",
                    success.action, success.input_bytes, success.token_count, success.output.bytes,
                ) {
                    Ok(()) => 0,
                    Err(_) => EXIT_OUTPUT,
                }
            }
            Ok(CommandSuccess::Demo(success)) => match writeln!(
                stdout,
                "PASS demo-pipeline label=demo_wikitext_only vocab_size={} merge_count={} token_count={} package_manifest_sha256={} deterministic_core_sha256={} round_trip_sha256={}",
                success.vocab_size(),
                success.merge_count(),
                success.token_count(),
                hex_upper(&success.package_manifest_sha256()),
                hex_upper(&success.deterministic_core_sha256()),
                hex_upper(&success.round_trip_sha256()),
            ) {
                Ok(()) => 0,
                Err(_) => EXIT_OUTPUT,
            },
            Ok(CommandSuccess::Compare(success)) => match writeln!(
                stdout,
                "PASS demo-compare label=demo_wikitext_only files={} bytes={} vocab_size={} merge_count={} token_count={} package_manifest_sha256={} run_tree_sha256={} deterministic_core_sha256={} source_commit={}",
                success.file_count(),
                success.total_bytes(),
                success.vocab_size(),
                success.merge_count(),
                success.token_count(),
                hex_upper(&success.package_manifest_sha256()),
                hex_upper(&success.run_tree_sha256()),
                hex_upper(&success.deterministic_core_sha256()),
                success.source_commit(),
            ) {
                Ok(()) => 0,
                Err(_) => EXIT_OUTPUT,
            },
            Err(failure) => {
                let _ = writeln!(stderr, "ERROR [{}]: {}", failure.class, failure.message);
                failure.exit_code
            }
        },
        Err(failure) => {
            let _ = writeln!(stderr, "ERROR [{}]: {}", failure.class, failure.message);
            let _ = stderr.write_all(usage_bytes());
            failure.exit_code
        }
    }
}

fn usage_bytes() -> &'static [u8] {
    b"Usage:\n  ashira encode --package <run_root_or_manifest> --text-file <input.txt> --out <encoded.json>\n  ashira decode --package <run_root_or_manifest> --encoded <encoded.json> --out <decoded.txt>\n  ashira demo-pipeline --manifest <demo_manifest.json> --run-root <new_run_root> --vocab-size <276..4096>\n  ashira demo-compare --run-a <run_a_root> --run-b <run_b_root>\n"
}

fn parse_command(args: &[OsString]) -> Result<CliCommand, CommandFailure> {
    let operands = args.get(1..).unwrap_or_default();
    let Some(subcommand) = operands.first() else {
        return Err(CommandFailure::usage("missing subcommand"));
    };
    if subcommand == "--help" || subcommand == "-h" {
        if operands.len() != 1 {
            return Err(CommandFailure::usage("--help accepts no operands"));
        }
        return Ok(CliCommand::Help);
    }
    match subcommand.to_str() {
        Some("encode") => parse_encode(&operands[1..]),
        Some("decode") => parse_decode(&operands[1..]),
        Some("demo-pipeline") => parse_demo_pipeline(&operands[1..]),
        Some("demo-compare") => parse_demo_compare(&operands[1..]),
        Some(other) => Err(CommandFailure::usage(format!("unknown subcommand {other}"))),
        None => Err(CommandFailure::usage("subcommand must be UTF-8")),
    }
}

fn parse_demo_compare(args: &[OsString]) -> Result<CliCommand, CommandFailure> {
    let mut run_a = None;
    let mut run_b = None;
    parse_options(args, |flag, value| match flag {
        "--run-a" => set_once(&mut run_a, value, flag),
        "--run-b" => set_once(&mut run_b, value, flag),
        _ => Err(CommandFailure::usage(format!(
            "unknown demo-compare option {flag}"
        ))),
    })?;
    Ok(CliCommand::DemoCompare {
        run_a: required_path(run_a, "--run-a")?,
        run_b: required_path(run_b, "--run-b")?,
    })
}

fn parse_demo_pipeline(args: &[OsString]) -> Result<CliCommand, CommandFailure> {
    let mut manifest = None;
    let mut run_root = None;
    let mut vocab_size = None;
    parse_options(args, |flag, value| match flag {
        "--manifest" => set_once(&mut manifest, value, flag),
        "--run-root" => set_once(&mut run_root, value, flag),
        "--vocab-size" => set_once(&mut vocab_size, value, flag),
        _ => Err(CommandFailure::usage(format!(
            "unknown demo-pipeline option {flag}"
        ))),
    })?;
    let vocab_size = required_decimal(vocab_size, "--vocab-size")?;
    Ok(CliCommand::DemoPipeline {
        manifest: required_path(manifest, "--manifest")?,
        run_root: required_path(run_root, "--run-root")?,
        vocab_size,
    })
}

fn parse_encode(args: &[OsString]) -> Result<CliCommand, CommandFailure> {
    let mut package = None;
    let mut text_file = None;
    let mut output = None;
    parse_options(args, |flag, value| match flag {
        "--package" => set_once(&mut package, value, flag),
        "--text-file" => set_once(&mut text_file, value, flag),
        "--out" => set_once(&mut output, value, flag),
        _ => Err(CommandFailure::usage(format!(
            "unknown encode option {flag}"
        ))),
    })?;
    Ok(CliCommand::Encode {
        package: required_path(package, "--package")?,
        text_file: required_path(text_file, "--text-file")?,
        output: required_path(output, "--out")?,
    })
}

fn parse_decode(args: &[OsString]) -> Result<CliCommand, CommandFailure> {
    let mut package = None;
    let mut encoded = None;
    let mut output = None;
    parse_options(args, |flag, value| match flag {
        "--package" => set_once(&mut package, value, flag),
        "--encoded" => set_once(&mut encoded, value, flag),
        "--out" => set_once(&mut output, value, flag),
        _ => Err(CommandFailure::usage(format!(
            "unknown decode option {flag}"
        ))),
    })?;
    Ok(CliCommand::Decode {
        package: required_path(package, "--package")?,
        encoded: required_path(encoded, "--encoded")?,
        output: required_path(output, "--out")?,
    })
}

fn parse_options(
    args: &[OsString],
    mut accept: impl FnMut(&str, OsString) -> Result<(), CommandFailure>,
) -> Result<(), CommandFailure> {
    let mut index = 0usize;
    while index < args.len() {
        let flag = args[index]
            .to_str()
            .ok_or_else(|| CommandFailure::usage("option names must be UTF-8"))?;
        let value = args
            .get(index + 1)
            .ok_or_else(|| CommandFailure::usage(format!("{flag} requires a value")))?
            .clone();
        accept(flag, value)?;
        index = index
            .checked_add(2)
            .ok_or_else(|| CommandFailure::usage("argument index overflow"))?;
    }
    Ok(())
}

fn set_once(
    target: &mut Option<OsString>,
    value: OsString,
    flag: &str,
) -> Result<(), CommandFailure> {
    if target.replace(value).is_some() {
        return Err(CommandFailure::usage(format!("duplicate option {flag}")));
    }
    Ok(())
}

fn required_path(value: Option<OsString>, flag: &str) -> Result<PathBuf, CommandFailure> {
    let value = value.ok_or_else(|| CommandFailure::usage(format!("missing {flag}")))?;
    if value.is_empty() {
        return Err(CommandFailure::usage(format!("{flag} cannot be empty")));
    }
    Ok(PathBuf::from(value))
}

fn required_decimal(value: Option<OsString>, flag: &str) -> Result<usize, CommandFailure> {
    let value = value.ok_or_else(|| CommandFailure::usage(format!("missing {flag}")))?;
    let value = value
        .to_str()
        .ok_or_else(|| CommandFailure::usage(format!("{flag} must be UTF-8 decimal")))?;
    if value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(CommandFailure::usage(format!(
            "{flag} must be unsigned decimal"
        )));
    }
    value
        .parse::<usize>()
        .map_err(|_| CommandFailure::usage(format!("{flag} is out of range")))
}

fn execute_command(
    command: CliCommand,
    limits: &CliLimits,
) -> Result<CommandSuccess, CommandFailure> {
    match command {
        CliCommand::Encode {
            package,
            text_file,
            output,
        } => execute_encode(&package, &text_file, &output, limits).map(CommandSuccess::File),
        CliCommand::Decode {
            package,
            encoded,
            output,
        } => execute_decode(&package, &encoded, &output, limits).map(CommandSuccess::File),
        CliCommand::DemoPipeline {
            manifest,
            run_root,
            vocab_size,
        } => run_demo_pipeline(&manifest, &run_root, vocab_size)
            .map(CommandSuccess::Demo)
            .map_err(|error| CommandFailure {
                exit_code: error.exit_code(),
                class: error.class(),
                message: error.to_string(),
            }),
        CliCommand::DemoCompare { run_a, run_b } => compare_demo_runs(&run_a, &run_b)
            .map(CommandSuccess::Compare)
            .map_err(|error| CommandFailure {
                exit_code: error.exit_code(),
                class: error.class(),
                message: error.to_string(),
            }),
        CliCommand::Help => Err(CommandFailure::usage("internal help dispatch")),
    }
}

fn execute_encode(
    package: &Path,
    text_file: &Path,
    output: &Path,
    limits: &CliLimits,
) -> Result<FileCommandSuccess, CommandFailure> {
    let destination = validate_output_destination(output)?;
    let tokenizer = load_v3_tokenizer_package(package, &limits.artifact).map_err(|error| {
        CommandFailure::input("Package", format!("V3 package load failed: {error}"))
    })?;
    let input = read_bounded_regular_file(
        text_file,
        limits.codec.max_input_bytes,
        "text input",
        IoClass::Input,
    )?;
    let input_bytes = usize_to_u64(input.len(), "text input length", IoClass::Input)?;
    let document = EncodedTokensV1::encode(&tokenizer, &input, &limits.codec)
        .map_err(|error| CommandFailure::codec(error.to_string()))?;
    let token_count = usize_to_u64(
        document.token_ids().len(),
        "encoded token count",
        IoClass::Input,
    )?;
    let json = document
        .to_canonical_json(&limits.codec)
        .map_err(|error| CommandFailure::codec(error.to_string()))?;
    let published = publish_output_atomic(&destination, &json)?;
    Ok(FileCommandSuccess {
        action: "encode",
        input_bytes,
        token_count,
        output: published,
    })
}

fn execute_decode(
    package: &Path,
    encoded: &Path,
    output: &Path,
    limits: &CliLimits,
) -> Result<FileCommandSuccess, CommandFailure> {
    let destination = validate_output_destination(output)?;
    let json = read_bounded_regular_file(
        encoded,
        limits.codec.max_encoded_json_bytes,
        "encoded JSON",
        IoClass::Input,
    )?;
    let input_bytes = usize_to_u64(json.len(), "encoded JSON length", IoClass::Input)?;
    let document = EncodedTokensV1::parse_json(&json, &limits.codec)
        .map_err(|error| CommandFailure::codec(error.to_string()))?;
    let token_count = usize_to_u64(
        document.token_ids().len(),
        "encoded token count",
        IoClass::Input,
    )?;
    let tokenizer = load_v3_tokenizer_package(package, &limits.artifact).map_err(|error| {
        CommandFailure::input("Package", format!("V3 package load failed: {error}"))
    })?;
    let decoded = document
        .decode(&tokenizer, &limits.codec)
        .map_err(|error| CommandFailure::codec(error.to_string()))?;
    let published = publish_output_atomic(&destination, &decoded)?;
    Ok(FileCommandSuccess {
        action: "decode",
        input_bytes,
        token_count,
        output: published,
    })
}

fn read_bounded_regular_file(
    path: &Path,
    limit: u64,
    resource: &'static str,
    io_class: IoClass,
) -> Result<Vec<u8>, CommandFailure> {
    let absolute = absolute_path(path, io_class)?;
    ensure_no_link_like_ancestors(&absolute, io_class)?;
    let canonical = fs::canonicalize(&absolute).map_err(|error| {
        io_class.failure(
            "Io",
            format!("cannot resolve {resource} {}: {error}", path.display()),
        )
    })?;
    ensure_no_link_like_ancestors(&canonical, io_class)?;
    ensure_regular_unlinked_file(&canonical, resource, io_class)?;

    let mut file = File::open(&canonical).map_err(|error| {
        io_class.failure(
            "Io",
            format!("cannot open {resource} {}: {error}", path.display()),
        )
    })?;
    let before = file.metadata().map_err(|error| {
        io_class.failure("Io", format!("cannot inspect open {resource}: {error}"))
    })?;
    if !before.is_file() || metadata_is_link_like(&before) {
        return Err(io_class.failure("InvalidPath", format!("{resource} is not a regular file")));
    }
    enforce_file_limit(before.len(), limit, resource, io_class)?;
    let capacity = usize::try_from(before.len()).map_err(|_| {
        io_class.failure(
            "ResourceLimit",
            format!("{resource} length is not representable on this platform"),
        )
    })?;
    let mut bytes = Vec::new();
    bytes.try_reserve_exact(capacity).map_err(|_| {
        io_class.failure(
            "ResourceLimit",
            format!("cannot allocate bounded {resource} buffer"),
        )
    })?;
    let take_limit = limit.checked_add(1).ok_or_else(|| {
        io_class.failure("ResourceLimit", format!("{resource} read limit overflow"))
    })?;
    Read::by_ref(&mut file)
        .take(take_limit)
        .read_to_end(&mut bytes)
        .map_err(|error| io_class.failure("Io", format!("cannot read {resource}: {error}")))?;
    let actual = usize_to_u64(bytes.len(), resource, io_class)?;
    enforce_file_limit(actual, limit, resource, io_class)?;
    let after = file.metadata().map_err(|error| {
        io_class.failure("Io", format!("cannot re-inspect open {resource}: {error}"))
    })?;
    if before.len() != actual || after.len() != actual {
        return Err(io_class.failure(
            "ConcurrentMutation",
            format!("{resource} length changed during bounded read"),
        ));
    }
    ensure_regular_unlinked_file(&canonical, resource, io_class)?;
    let canonical_after = fs::canonicalize(&absolute).map_err(|error| {
        io_class.failure(
            "ConcurrentMutation",
            format!("cannot re-resolve {resource}: {error}"),
        )
    })?;
    if canonical_after != canonical {
        return Err(io_class.failure(
            "ConcurrentMutation",
            format!("{resource} path changed during bounded read"),
        ));
    }
    Ok(bytes)
}

fn validate_output_destination(path: &Path) -> Result<OutputDestination, CommandFailure> {
    if path.as_os_str().is_empty() {
        return Err(CommandFailure::output(
            "InvalidPath",
            "output path cannot be empty",
        ));
    }
    let filename = path
        .file_name()
        .filter(|name| !name.is_empty() && *name != "." && *name != "..")
        .ok_or_else(|| CommandFailure::output("InvalidPath", "output filename is invalid"))?
        .to_os_string();
    let parent_input = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    let parent_absolute = absolute_path(parent_input, IoClass::Output)?;
    ensure_no_link_like_ancestors(&parent_absolute, IoClass::Output)?;
    let parent = fs::canonicalize(&parent_absolute).map_err(|error| {
        CommandFailure::output(
            "InvalidPath",
            format!("cannot resolve output parent: {error}"),
        )
    })?;
    ensure_no_link_like_ancestors(&parent, IoClass::Output)?;
    let metadata = fs::symlink_metadata(&parent).map_err(|error| {
        CommandFailure::output(
            "InvalidPath",
            format!("cannot inspect output parent: {error}"),
        )
    })?;
    if metadata_is_link_like(&metadata) || !metadata.is_dir() {
        return Err(CommandFailure::output(
            "InvalidPath",
            "output parent is not a regular directory",
        ));
    }
    let final_path = parent.join(&filename);
    ensure_destination_absent(&final_path)?;
    Ok(OutputDestination {
        parent,
        final_path,
        filename,
    })
}

fn publish_output_atomic(
    destination: &OutputDestination,
    bytes: &[u8],
) -> Result<PublishedOutput, CommandFailure> {
    ensure_no_link_like_ancestors(&destination.parent, IoClass::Output)?;
    ensure_destination_absent(&destination.final_path)?;
    let (staging_path, staging_file) = create_staging_file(destination)?;
    let mut writer = BufWriter::new(staging_file);
    writer.write_all(bytes).map_err(|error| {
        CommandFailure::output("Io", format!("staged output write failed: {error}"))
    })?;
    writer.flush().map_err(|error| {
        CommandFailure::output("Durability", format!("staged output flush failed: {error}"))
    })?;
    writer.get_ref().sync_all().map_err(|error| {
        CommandFailure::output(
            "Durability",
            format!("staged output sync_all failed: {error}"),
        )
    })?;
    drop(writer);

    let expected_len = usize_to_u64(bytes.len(), "output bytes", IoClass::Output)?;
    let readback = read_bounded_regular_file(
        &staging_path,
        expected_len,
        "staged output",
        IoClass::Output,
    )?;
    if readback != bytes {
        return Err(CommandFailure::output(
            "Durability",
            "staged output readback mismatch",
        ));
    }
    ensure_destination_absent(&destination.final_path)?;
    rename_staging_no_replace(&staging_path, &destination.final_path)?;

    let final_readback = read_bounded_regular_file(
        &destination.final_path,
        expected_len,
        "published output",
        IoClass::Output,
    )?;
    if final_readback != bytes {
        return Err(CommandFailure::output(
            "Durability",
            "published output readback mismatch",
        ));
    }
    Ok(PublishedOutput {
        bytes: expected_len,
        sha256: finalize_sha256(Sha256::digest(bytes)),
    })
}

fn create_staging_file(destination: &OutputDestination) -> Result<(PathBuf, File), CommandFailure> {
    for _ in 0..MAX_STAGING_ATTEMPTS {
        let ordinal = NEXT_STAGING_ORDINAL.fetch_add(1, Ordering::Relaxed);
        let mut name = OsString::from(".");
        name.push(&destination.filename);
        name.push(format!(
            ".ashira-d4-staging-{}-{ordinal:016X}",
            std::process::id()
        ));
        let staging = destination.parent.join(name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&staging)
        {
            Ok(file) => {
                let metadata = fs::symlink_metadata(&staging).map_err(|error| {
                    CommandFailure::output(
                        "InvalidPath",
                        format!("cannot inspect staged output: {error}"),
                    )
                })?;
                if metadata_is_link_like(&metadata) || !metadata.is_file() {
                    return Err(CommandFailure::output(
                        "InvalidPath",
                        "staged output is not a regular file",
                    ));
                }
                return Ok((staging, file));
            }
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
            Err(error) => {
                return Err(CommandFailure::output(
                    "Io",
                    format!("cannot create staged output: {error}"),
                ));
            }
        }
    }
    Err(CommandFailure::output(
        "ResourceLimit",
        format!("exhausted {MAX_STAGING_ATTEMPTS} staged-output attempts"),
    ))
}

fn ensure_destination_absent(path: &Path) -> Result<(), CommandFailure> {
    match fs::symlink_metadata(path) {
        Ok(_) => Err(CommandFailure::output(
            "ExistingOutput",
            format!("output already exists: {}", path.display()),
        )),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(CommandFailure::output(
            "Io",
            format!("cannot inspect output destination: {error}"),
        )),
    }
}

fn absolute_path(path: &Path, io_class: IoClass) -> Result<PathBuf, CommandFailure> {
    if path.as_os_str().is_empty() {
        return Err(io_class.failure("InvalidPath", "path cannot be empty"));
    }
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        std::env::current_dir()
            .map(|current| current.join(path))
            .map_err(|error| {
                io_class.failure("Io", format!("cannot resolve current directory: {error}"))
            })
    }
}

fn ensure_no_link_like_ancestors(path: &Path, io_class: IoClass) -> Result<(), CommandFailure> {
    for ancestor in path
        .ancestors()
        .filter(|entry| !entry.as_os_str().is_empty())
    {
        let metadata = fs::symlink_metadata(ancestor).map_err(|error| {
            io_class.failure(
                "InvalidPath",
                format!(
                    "cannot inspect path ancestor {}: {error}",
                    ancestor.display()
                ),
            )
        })?;
        if metadata_is_link_like(&metadata) {
            return Err(io_class.failure(
                "InvalidPath",
                format!("link or reparse path rejected: {}", ancestor.display()),
            ));
        }
    }
    Ok(())
}

fn ensure_regular_unlinked_file(
    path: &Path,
    resource: &'static str,
    io_class: IoClass,
) -> Result<Metadata, CommandFailure> {
    let metadata = fs::symlink_metadata(path).map_err(|error| {
        io_class.failure(
            "InvalidPath",
            format!("cannot inspect {resource} {}: {error}", path.display()),
        )
    })?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(io_class.failure(
            "InvalidPath",
            format!("{resource} is not a regular unlinked file"),
        ));
    }
    Ok(metadata)
}

fn enforce_file_limit(
    actual: u64,
    limit: u64,
    resource: &'static str,
    io_class: IoClass,
) -> Result<(), CommandFailure> {
    if actual > limit {
        return Err(io_class.failure(
            "ResourceLimit",
            format!("{resource} exceeds limit: {actual} > {limit}"),
        ));
    }
    Ok(())
}

fn usize_to_u64(
    value: usize,
    resource: &'static str,
    io_class: IoClass,
) -> Result<u64, CommandFailure> {
    u64::try_from(value).map_err(|_| {
        io_class.failure(
            "ResourceLimit",
            format!("{resource} is not representable as u64"),
        )
    })
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
fn rename_staging_no_replace(staging: &Path, destination: &Path) -> Result<(), CommandFailure> {
    ensure_destination_absent(destination)?;
    match fs::rename(staging, destination) {
        Ok(()) => Ok(()),
        Err(error) if fs::symlink_metadata(destination).is_ok() => Err(CommandFailure::output(
            "ExistingOutput",
            format!("output appeared during publication: {error}"),
        )),
        Err(error) => Err(CommandFailure::output(
            "Durability",
            format!("same-parent output rename failed: {error}"),
        )),
    }
}

#[cfg(not(windows))]
fn rename_staging_no_replace(_staging: &Path, _destination: &Path) -> Result<(), CommandFailure> {
    Err(CommandFailure::output(
        "Unsupported",
        "non-overwrite atomic file rename is not proven on this platform",
    ))
}

fn finalize_sha256(digest: impl AsRef<[u8]>) -> [u8; 32] {
    let mut output = [0u8; 32];
    output.copy_from_slice(digest.as_ref());
    output
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{PublicationContext, PublicationContextInput, TokenizerTrainer, write_v3_package};
    use std::ffi::OsStr;
    use std::sync::atomic::{AtomicU64, Ordering};

    fn unique_test_directory(label: &str) -> PathBuf {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let ordinal = NEXT.fetch_add(1, Ordering::Relaxed);
        let path = std::env::temp_dir().join(format!(
            "ashira_v3_cli_{}_{}_{}",
            std::process::id(),
            ordinal,
            label
        ));
        fs::create_dir(&path).unwrap();
        path
    }

    fn test_context() -> PublicationContext {
        PublicationContext::try_from_input(PublicationContextInput {
            run_id: "d4-cli-test",
            checkpoint_id: "checkpoint-000001",
            parent_checkpoint_id: None,
            deterministic_config_sha256: [0x11; 32],
            corpus_manifest_sha256: [0x22; 32],
            calibration_report_sha256: [0x33; 32],
            probe_selection_sha256: [0x44; 32],
            source_commit: "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
            source_tree: "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
            source_tracked_files_sha256: [0x55; 32],
            writer_version: "ashira-tokenizer-v3 0.1.0",
            toolchain_identity: "rustc 1.94.0",
            readback_evidence_id: "d4-readback-v1",
            prefix_proof_evidence_id: "d4-prefix-v1",
            effective_backend: "cpu",
        })
        .unwrap()
    }

    fn test_package(root: &Path) -> PathBuf {
        let package = root.join("package");
        let tokenizer = TokenizerTrainer::new().freeze().unwrap();
        write_v3_package(&tokenizer, &package, &test_context()).unwrap();
        package
    }

    fn command_args(parts: &[&OsStr]) -> Vec<OsString> {
        std::iter::once(OsString::from("ashira"))
            .chain(parts.iter().map(|part| (*part).to_os_string()))
            .collect()
    }

    fn run(args: Vec<OsString>) -> (i32, String, String) {
        run_with_limits(args, &CliLimits::build_week_demo())
    }

    fn run_with_limits(args: Vec<OsString>, limits: &CliLimits) -> (i32, String, String) {
        let mut stdout = Vec::new();
        let mut stderr = Vec::new();
        let code = run_cli_with_limits(args, &mut stdout, &mut stderr, limits);
        (
            code,
            String::from_utf8(stdout).unwrap(),
            String::from_utf8(stderr).unwrap(),
        )
    }

    #[test]
    fn command_round_trip_preserves_bounded_non_utf8_and_line_bytes() {
        let root = unique_test_directory("round_trip");
        let package = test_package(&root);
        let input = root.join("input.bin");
        let encoded = root.join("encoded.json");
        let encoded_b = root.join("encoded-b.json");
        let decoded = root.join("decoded.bin");
        let bytes = b"alpha\r\n\xFFbeta\n<kareem_narration>\r";
        fs::write(&input, bytes).unwrap();

        let encode_args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            package.as_os_str(),
            OsStr::new("--text-file"),
            input.as_os_str(),
            OsStr::new("--out"),
            encoded.as_os_str(),
        ]);
        let (code, stdout, stderr) = run(encode_args);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.starts_with("PASS encode "));

        let encode_b_args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            package.as_os_str(),
            OsStr::new("--text-file"),
            input.as_os_str(),
            OsStr::new("--out"),
            encoded_b.as_os_str(),
        ]);
        let (code, stdout_b, stderr) = run(encode_b_args);
        assert_eq!(code, 0, "{stderr}");
        assert_eq!(stdout_b, stdout);
        assert_eq!(fs::read(&encoded_b).unwrap(), fs::read(&encoded).unwrap());

        let decode_args = command_args(&[
            OsStr::new("decode"),
            OsStr::new("--package"),
            package.join("package_manifest.json").as_os_str(),
            OsStr::new("--encoded"),
            encoded.as_os_str(),
            OsStr::new("--out"),
            decoded.as_os_str(),
        ]);
        let (code, stdout, stderr) = run(decode_args);
        assert_eq!(code, 0, "{stderr}");
        assert!(stdout.starts_with("PASS decode "));
        assert_eq!(fs::read(&decoded).unwrap(), bytes);
        assert!(!fs::read_dir(&root).unwrap().any(|entry| {
            entry
                .unwrap()
                .file_name()
                .to_string_lossy()
                .contains(".ashira-d4-staging-")
        }));
    }

    #[test]
    fn decode_rejects_malformed_invalid_and_above_u32_operational_ids() {
        let root = unique_test_directory("bad_json_ids");
        let package = test_package(&root);
        let input = root.join("input.bin");
        let valid = root.join("valid.json");
        fs::write(&input, b"x").unwrap();
        let encode_args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            package.as_os_str(),
            OsStr::new("--text-file"),
            input.as_os_str(),
            OsStr::new("--out"),
            valid.as_os_str(),
        ]);
        assert_eq!(run(encode_args).0, 0);
        let canonical = String::from_utf8(fs::read(&valid).unwrap()).unwrap();

        let cases = [
            ("malformed", "{\n".to_owned()),
            (
                "invalid_vocab",
                canonical.replace("\"token_ids\":[140]", "\"token_ids\":[276]"),
            ),
            (
                "above_operational",
                canonical.replace("\"token_ids\":[140]", "\"token_ids\":[131072]"),
            ),
        ];
        for (label, document) in cases {
            let encoded = root.join(format!("{label}.json"));
            let output = root.join(format!("{label}.out"));
            fs::write(&encoded, document).unwrap();
            let args = command_args(&[
                OsStr::new("decode"),
                OsStr::new("--package"),
                package.as_os_str(),
                OsStr::new("--encoded"),
                encoded.as_os_str(),
                OsStr::new("--out"),
                output.as_os_str(),
            ]);
            let (code, _, stderr) = run(args);
            assert_eq!(code, EXIT_CODEC, "{label}: {stderr}");
            assert!(!output.exists());
        }
    }

    #[test]
    fn existing_output_fails_before_package_or_input_reads_and_is_preserved() {
        let root = unique_test_directory("existing_output");
        let output = root.join("owned.json");
        fs::write(&output, b"operator-owned").unwrap();
        let args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            root.join("missing-package").as_os_str(),
            OsStr::new("--text-file"),
            root.join("missing-input").as_os_str(),
            OsStr::new("--out"),
            output.as_os_str(),
        ]);
        let (code, _, stderr) = run(args);
        assert_eq!(code, EXIT_OUTPUT);
        assert!(stderr.contains("ExistingOutput"));
        assert_eq!(fs::read(&output).unwrap(), b"operator-owned");
    }

    #[test]
    fn command_rejects_headerless_package_without_exposing_output() {
        let root = unique_test_directory("headerless_package");
        let package = root.join("legacy.bin");
        let input = root.join("input.bin");
        let output = root.join("encoded.json");
        fs::write(&package, b"headerless legacy artifact").unwrap();
        fs::write(&input, b"input").unwrap();
        let args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            package.as_os_str(),
            OsStr::new("--text-file"),
            input.as_os_str(),
            OsStr::new("--out"),
            output.as_os_str(),
        ]);
        let (code, _, stderr) = run(args);
        assert_eq!(code, EXIT_INPUT);
        assert!(stderr.contains("Package"));
        assert!(!output.exists());
    }

    #[test]
    fn bounded_input_fails_before_output_staging() {
        let root = unique_test_directory("bounded_input");
        let package = test_package(&root);
        let input = root.join("too-large.bin");
        let output = root.join("encoded.json");
        fs::write(&input, b"four").unwrap();
        let mut limits = CliLimits::build_week_demo();
        limits.codec.max_input_bytes = 3;
        let args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            package.as_os_str(),
            OsStr::new("--text-file"),
            input.as_os_str(),
            OsStr::new("--out"),
            output.as_os_str(),
        ]);
        let (code, _, stderr) = run_with_limits(args, &limits);
        assert_eq!(code, EXIT_INPUT, "{stderr}");
        assert!(stderr.contains("ResourceLimit"));
        assert!(!output.exists());
    }

    #[test]
    fn parser_rejects_missing_duplicate_unknown_and_legacy_training_forms() {
        let cases = [
            command_args(&[OsStr::new("encode")]),
            command_args(&[
                OsStr::new("encode"),
                OsStr::new("--package"),
                OsStr::new("a"),
                OsStr::new("--package"),
                OsStr::new("b"),
            ]),
            command_args(&[OsStr::new("decode"), OsStr::new("--unknown")]),
            command_args(&[OsStr::new("--corpus"), OsStr::new("legacy")]),
            command_args(&[
                OsStr::new("demo-pipeline"),
                OsStr::new("--manifest"),
                OsStr::new("demo.json"),
                OsStr::new("--run-root"),
                OsStr::new("run-a"),
            ]),
            command_args(&[
                OsStr::new("demo-pipeline"),
                OsStr::new("--manifest"),
                OsStr::new("demo.json"),
                OsStr::new("--run-root"),
                OsStr::new("run-a"),
                OsStr::new("--vocab-size"),
                OsStr::new("+512"),
            ]),
            command_args(&[
                OsStr::new("demo-compare"),
                OsStr::new("--run-a"),
                OsStr::new("run-a"),
            ]),
            command_args(&[
                OsStr::new("demo-compare"),
                OsStr::new("--run-a"),
                OsStr::new("run-a"),
                OsStr::new("--run-a"),
                OsStr::new("run-b"),
            ]),
        ];
        for args in cases {
            let (code, _, stderr) = run(args);
            assert_eq!(code, EXIT_USAGE);
            assert!(stderr.contains("Usage:"));
        }
    }

    #[test]
    fn parser_accepts_exact_demo_pipeline_form() {
        let parsed = parse_command(&command_args(&[
            OsStr::new("demo-pipeline"),
            OsStr::new("--manifest"),
            OsStr::new("demo/demo_wikitext_manifest.json"),
            OsStr::new("--run-root"),
            OsStr::new("../demo_runs/run_a"),
            OsStr::new("--vocab-size"),
            OsStr::new("512"),
        ]))
        .unwrap();
        let CliCommand::DemoPipeline {
            manifest,
            run_root,
            vocab_size,
        } = parsed
        else {
            panic!("expected demo-pipeline command");
        };
        assert_eq!(manifest, Path::new("demo/demo_wikitext_manifest.json"));
        assert_eq!(run_root, Path::new("../demo_runs/run_a"));
        assert_eq!(vocab_size, 512);
    }

    #[test]
    fn parser_accepts_exact_demo_compare_form() {
        let parsed = parse_command(&command_args(&[
            OsStr::new("demo-compare"),
            OsStr::new("--run-a"),
            OsStr::new("../demo_runs/run_a"),
            OsStr::new("--run-b"),
            OsStr::new("../demo_runs/run_b"),
        ]))
        .unwrap();
        let CliCommand::DemoCompare { run_a, run_b } = parsed else {
            panic!("expected demo-compare command");
        };
        assert_eq!(run_a, Path::new("../demo_runs/run_a"));
        assert_eq!(run_b, Path::new("../demo_runs/run_b"));
    }

    #[cfg(windows)]
    #[test]
    fn output_reparse_parent_is_rejected_before_staging() {
        use std::os::windows::fs::symlink_dir;

        let root = unique_test_directory("output_reparse");
        let real = root.join("real");
        fs::create_dir(&real).unwrap();
        let linked = root.join("linked");
        symlink_dir(&real, &linked).unwrap();
        let args = command_args(&[
            OsStr::new("encode"),
            OsStr::new("--package"),
            root.join("missing-package").as_os_str(),
            OsStr::new("--text-file"),
            root.join("missing-input").as_os_str(),
            OsStr::new("--out"),
            linked.join("encoded.json").as_os_str(),
        ]);
        let (code, _, stderr) = run(args);
        assert_eq!(code, EXIT_OUTPUT);
        assert!(stderr.contains("InvalidPath"));
        assert!(!real.join("encoded.json").exists());
    }

    #[cfg(windows)]
    #[test]
    fn no_replace_rename_preserves_late_output_and_staging_bytes() {
        let root = unique_test_directory("rename_race");
        let staging = root.join("staging.tmp");
        let output = root.join("output.json");
        fs::write(&staging, b"candidate").unwrap();
        fs::write(&output, b"late-owner").unwrap();

        let error = rename_staging_no_replace(&staging, &output).unwrap_err();
        assert_eq!(error.class, "ExistingOutput");
        assert_eq!(fs::read(&output).unwrap(), b"late-owner");
        assert_eq!(fs::read(&staging).unwrap(), b"candidate");
    }
}
