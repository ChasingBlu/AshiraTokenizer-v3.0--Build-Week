use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256, Sha512};
use std::collections::{BTreeMap, HashSet};
use std::error::Error;
use std::fmt;
use std::fs::{self, File, Metadata};
use std::io::{self, Read};
use std::path::{Component, Path, PathBuf};

pub const COMPOSITE_MANIFEST_SCHEMA: &str = "ashira_v3_composite_corpus_manifest_v1";
pub const DEMO_WIKITEXT_MANIFEST_SCHEMA: &str = "ashira_v3_demo_wikitext_manifest_v1";
pub const DEMO_WIKITEXT_MANIFEST_LABEL: &str = "demo_wikitext_only";
pub const MAX_COMPOSITE_MANIFEST_BYTES: usize = 1024 * 1024;
pub const MAX_COMPOSITE_MANIFEST_ENTRIES: usize = 16_384;
pub const MAX_COMPOSITE_MANIFEST_ROOTS: usize = 64;

const MAX_ROOT_ID_BYTES: usize = 128;
const MAX_RELATIVE_PATH_BYTES: usize = 4_096;
const HASH_BUFFER_BYTES: usize = 8_192;

#[derive(Clone, Copy, Debug, Deserialize, Eq, Hash, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum CorpusFamily {
    Identity,
    Scripture,
    Wikitext,
    Bookcorpus,
}

impl CorpusFamily {
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Identity => "identity",
            Self::Scripture => "scripture",
            Self::Wikitext => "wikitext",
            Self::Bookcorpus => "bookcorpus",
        }
    }

    const ALL: [Self; 4] = [
        Self::Identity,
        Self::Scripture,
        Self::Wikitext,
        Self::Bookcorpus,
    ];
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ManifestAdmissionProfile {
    CompositeFourFamily,
    DemoWikitextOnly,
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(rename_all = "lowercase")]
pub enum EncodingPolicy {
    Utf8,
    Bytes,
}

#[derive(Clone, Copy, Debug)]
pub struct ManifestRoot<'a> {
    pub id: &'a str,
    pub path: &'a Path,
}

#[derive(Clone, Copy, Debug)]
pub struct FamilyWeights {
    identity: i64,
    scripture: i64,
    wikitext: i64,
    bookcorpus: i64,
}

impl FamilyWeights {
    pub fn try_new(
        identity: i64,
        scripture: i64,
        wikitext: i64,
        bookcorpus: i64,
    ) -> Result<Self, ManifestError> {
        if [identity, scripture, wikitext, bookcorpus]
            .into_iter()
            .any(|weight| weight <= 0)
        {
            return Err(ManifestError::InvalidField {
                field: "family_weights",
            });
        }
        Ok(Self {
            identity,
            scripture,
            wikitext,
            bookcorpus,
        })
    }

    pub const fn for_family(self, family: CorpusFamily) -> i64 {
        match family {
            CorpusFamily::Identity => self.identity,
            CorpusFamily::Scripture => self.scripture,
            CorpusFamily::Wikitext => self.wikitext,
            CorpusFamily::Bookcorpus => self.bookcorpus,
        }
    }
}

#[derive(Debug)]
pub enum ManifestError {
    Io {
        operation: &'static str,
        source: io::Error,
    },
    Json(serde_json::Error),
    InvalidField {
        field: &'static str,
    },
    NonCanonicalManifest,
    PathRejected {
        path: String,
    },
    FileSetMismatch,
    FileMetadataMismatch {
        path: String,
        field: &'static str,
    },
    HashMismatch {
        path: String,
        algorithm: &'static str,
    },
    EncodingMismatch {
        path: String,
    },
    ResourceLimitExceeded {
        resource: &'static str,
        limit: u64,
        actual: u64,
    },
}

impl ManifestError {
    pub const fn class(&self) -> &'static str {
        match self {
            Self::Io { .. } => "Io",
            Self::Json(_) => "Json",
            Self::InvalidField { .. } => "InvalidField",
            Self::NonCanonicalManifest => "NonCanonicalManifest",
            Self::PathRejected { .. } => "PathRejected",
            Self::FileSetMismatch => "FileSetMismatch",
            Self::FileMetadataMismatch { .. } => "FileMetadataMismatch",
            Self::HashMismatch { .. } => "HashMismatch",
            Self::EncodingMismatch { .. } => "EncodingMismatch",
            Self::ResourceLimitExceeded { .. } => "ResourceLimitExceeded",
        }
    }
}

impl fmt::Display for ManifestError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { operation, source } => {
                write!(
                    formatter,
                    "manifest I/O failed during {operation}: {source}"
                )
            }
            Self::Json(error) => write!(formatter, "manifest JSON is invalid: {error}"),
            Self::InvalidField { field } => write!(formatter, "invalid manifest field {field}"),
            Self::NonCanonicalManifest => formatter.write_str("manifest JSON is not canonical"),
            Self::PathRejected { path } => write!(formatter, "manifest path rejected: {path}"),
            Self::FileSetMismatch => {
                formatter.write_str("manifest declarations do not equal root inventory")
            }
            Self::FileMetadataMismatch { path, field } => {
                write!(
                    formatter,
                    "manifest file metadata mismatch for {path}: {field}"
                )
            }
            Self::HashMismatch { path, algorithm } => {
                write!(formatter, "manifest file {algorithm} mismatch for {path}")
            }
            Self::EncodingMismatch { path } => {
                write!(formatter, "manifest UTF-8 policy rejected {path}")
            }
            Self::ResourceLimitExceeded {
                resource,
                limit,
                actual,
            } => write!(
                formatter,
                "manifest resource limit exceeded for {resource}: {actual} > {limit}"
            ),
        }
    }
}

impl Error for ManifestError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(error) => Some(error),
            _ => None,
        }
    }
}

#[derive(Clone, Debug)]
pub struct AdmittedFile {
    ordinal: u32,
    family: CorpusFamily,
    root_id: String,
    relative_path: String,
    absolute_path: PathBuf,
    bytes: u64,
    sha256: [u8; 32],
    sha512: [u8; 64],
    encoding_policy: EncodingPolicy,
    identity: FileIdentity,
}

impl AdmittedFile {
    pub const fn ordinal(&self) -> u32 {
        self.ordinal
    }

    pub const fn family(&self) -> CorpusFamily {
        self.family
    }

    pub fn root_id(&self) -> &str {
        &self.root_id
    }

    pub fn relative_path(&self) -> &str {
        &self.relative_path
    }

    pub fn path(&self) -> &Path {
        &self.absolute_path
    }

    pub const fn bytes(&self) -> u64 {
        self.bytes
    }

    pub const fn sha256(&self) -> [u8; 32] {
        self.sha256
    }

    pub const fn sha512(&self) -> [u8; 64] {
        self.sha512
    }

    pub const fn encoding_policy(&self) -> EncodingPolicy {
        self.encoding_policy
    }

    pub(crate) fn read_verified_bytes(&self) -> Result<Vec<u8>, ManifestError> {
        verify_declared_file(self, true)?.ok_or(ManifestError::FileMetadataMismatch {
            path: self.relative_path.clone(),
            field: "verified content collection",
        })
    }
}

#[derive(Clone, Debug)]
pub struct AdmittedCorpus {
    profile: ManifestAdmissionProfile,
    manifest_sha256: [u8; 32],
    manifest_sha512: [u8; 64],
    files: Vec<AdmittedFile>,
    total_bytes: u64,
}

impl AdmittedCorpus {
    pub const fn profile(&self) -> ManifestAdmissionProfile {
        self.profile
    }

    pub const fn manifest_sha256(&self) -> [u8; 32] {
        self.manifest_sha256
    }

    pub const fn manifest_sha512(&self) -> [u8; 64] {
        self.manifest_sha512
    }

    pub fn files(&self) -> &[AdmittedFile] {
        &self.files
    }

    pub const fn total_bytes(&self) -> u64 {
        self.total_bytes
    }

    pub fn training_files(&self, weights: FamilyWeights) -> Vec<crate::TrainingFile> {
        self.files
            .iter()
            .cloned()
            .map(|file| {
                let weight = weights.for_family(file.family());
                crate::TrainingFile::from_admitted(file, weight)
            })
            .collect()
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestWire {
    schema: String,
    #[serde(skip_serializing_if = "Option::is_none")]
    label: Option<String>,
    entries: Vec<ManifestEntryWire>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
struct ManifestEntryWire {
    ordinal: u32,
    family: CorpusFamily,
    root_id: String,
    relative_path: String,
    enabled: bool,
    bytes: u64,
    sha256: String,
    sha512: String,
    encoding_policy: EncodingPolicy,
}

#[derive(Clone, Debug)]
struct RootScope {
    id: String,
    canonical_path: PathBuf,
    filesystem_scope: u64,
}

#[derive(Clone, Debug)]
struct InventoryFile {
    absolute_path: PathBuf,
    bytes: u64,
    identity: FileIdentity,
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct FileIdentity {
    scope: u64,
    index: u64,
}

pub fn admit_corpus_manifest(
    manifest_path: &Path,
    roots: &[ManifestRoot<'_>],
) -> Result<AdmittedCorpus, ManifestError> {
    admit_manifest(
        manifest_path,
        roots,
        ManifestAdmissionProfile::CompositeFourFamily,
    )
}

pub fn admit_demo_wikitext_manifest(
    manifest_path: &Path,
    roots: &[ManifestRoot<'_>],
) -> Result<AdmittedCorpus, ManifestError> {
    admit_manifest(
        manifest_path,
        roots,
        ManifestAdmissionProfile::DemoWikitextOnly,
    )
}

fn admit_manifest(
    manifest_path: &Path,
    roots: &[ManifestRoot<'_>],
    profile: ManifestAdmissionProfile,
) -> Result<AdmittedCorpus, ManifestError> {
    let manifest_bytes = read_bounded_regular_file(
        manifest_path,
        MAX_COMPOSITE_MANIFEST_BYTES,
        "corpus manifest read",
    )?;
    let wire: ManifestWire =
        serde_json::from_slice(&manifest_bytes).map_err(ManifestError::Json)?;
    let mut canonical = serde_json::to_vec(&wire).map_err(ManifestError::Json)?;
    canonical.push(b'\n');
    if manifest_bytes != canonical {
        return Err(ManifestError::NonCanonicalManifest);
    }
    validate_manifest_wire(&wire, profile)?;
    let root_scopes = validate_roots(roots)?;
    let declared_root_ids: HashSet<&str> = wire
        .entries
        .iter()
        .map(|entry| entry.root_id.as_str())
        .collect();
    let supplied_root_ids: HashSet<&str> =
        root_scopes.iter().map(|root| root.id.as_str()).collect();
    if declared_root_ids != supplied_root_ids {
        return Err(ManifestError::InvalidField {
            field: "manifest root set",
        });
    }
    let inventory = inventory_roots(&root_scopes)?;

    let declared_keys: HashSet<(String, String)> = wire
        .entries
        .iter()
        .map(|entry| (entry.root_id.clone(), entry.relative_path.clone()))
        .collect();
    let inventory_keys: HashSet<(String, String)> = inventory.keys().cloned().collect();
    if declared_keys != inventory_keys {
        return Err(ManifestError::FileSetMismatch);
    }

    let mut enabled_files = Vec::new();
    enabled_files.try_reserve(wire.entries.len()).map_err(|_| {
        ManifestError::ResourceLimitExceeded {
            resource: "admitted_file_allocation",
            limit: MAX_COMPOSITE_MANIFEST_ENTRIES as u64,
            actual: wire.entries.len() as u64,
        }
    })?;
    let mut total_bytes = 0u64;
    for entry in &wire.entries {
        let key = (entry.root_id.clone(), entry.relative_path.clone());
        let inventoried = inventory.get(&key).ok_or(ManifestError::FileSetMismatch)?;
        let sha256 = decode_upper_hex::<32>("sha256", &entry.sha256)?;
        let sha512 = decode_upper_hex::<64>("sha512", &entry.sha512)?;
        let admitted = AdmittedFile {
            ordinal: entry.ordinal,
            family: entry.family,
            root_id: entry.root_id.clone(),
            relative_path: entry.relative_path.clone(),
            absolute_path: inventoried.absolute_path.clone(),
            bytes: entry.bytes,
            sha256,
            sha512,
            encoding_policy: entry.encoding_policy,
            identity: inventoried.identity,
        };
        if inventoried.bytes != admitted.bytes {
            return Err(ManifestError::FileMetadataMismatch {
                path: admitted.relative_path.clone(),
                field: "bytes",
            });
        }
        verify_declared_file(&admitted, false)?;
        if entry.enabled {
            total_bytes = total_bytes.checked_add(entry.bytes).ok_or(
                ManifestError::ResourceLimitExceeded {
                    resource: "admitted_total_bytes",
                    limit: u64::MAX,
                    actual: u64::MAX,
                },
            )?;
            enabled_files.push(admitted);
        }
    }

    Ok(AdmittedCorpus {
        profile,
        manifest_sha256: finalize_sha256(Sha256::new_with_prefix(&manifest_bytes)),
        manifest_sha512: finalize_sha512(Sha512::new_with_prefix(&manifest_bytes)),
        files: enabled_files,
        total_bytes,
    })
}

fn validate_manifest_wire(
    wire: &ManifestWire,
    profile: ManifestAdmissionProfile,
) -> Result<(), ManifestError> {
    match profile {
        ManifestAdmissionProfile::CompositeFourFamily => {
            if wire.schema != COMPOSITE_MANIFEST_SCHEMA {
                return Err(ManifestError::InvalidField { field: "schema" });
            }
            if wire.label.is_some() {
                return Err(ManifestError::InvalidField { field: "label" });
            }
        }
        ManifestAdmissionProfile::DemoWikitextOnly => {
            if wire.schema != DEMO_WIKITEXT_MANIFEST_SCHEMA {
                return Err(ManifestError::InvalidField { field: "schema" });
            }
            if wire.label.as_deref() != Some(DEMO_WIKITEXT_MANIFEST_LABEL) {
                return Err(ManifestError::InvalidField { field: "label" });
            }
        }
    }
    if wire.entries.is_empty() || wire.entries.len() > MAX_COMPOSITE_MANIFEST_ENTRIES {
        return Err(ManifestError::ResourceLimitExceeded {
            resource: "manifest_entries",
            limit: MAX_COMPOSITE_MANIFEST_ENTRIES as u64,
            actual: wire.entries.len() as u64,
        });
    }

    let mut previous_path: Option<&[u8]> = None;
    let mut enabled_families = HashSet::new();
    let mut enabled_family_bytes = [0u64; 4];
    let mut seen_roots = HashSet::new();
    let mut seen_paths = HashSet::new();
    for (index, entry) in wire.entries.iter().enumerate() {
        let expected_ordinal =
            u32::try_from(index + 1).map_err(|_| ManifestError::ResourceLimitExceeded {
                resource: "manifest_ordinal",
                limit: MAX_COMPOSITE_MANIFEST_ENTRIES as u64,
                actual: (index + 1) as u64,
            })?;
        if entry.ordinal != expected_ordinal {
            return Err(ManifestError::InvalidField { field: "ordinal" });
        }
        validate_root_id(&entry.root_id)?;
        validate_relative_path(&entry.relative_path)?;
        validate_upper_hex("sha256", &entry.sha256, 32)?;
        validate_upper_hex("sha512", &entry.sha512, 64)?;
        if previous_path.is_some_and(|path| path >= entry.relative_path.as_bytes()) {
            return Err(ManifestError::InvalidField {
                field: "entry ordering",
            });
        }
        previous_path = Some(entry.relative_path.as_bytes());
        if !seen_paths.insert(entry.relative_path.as_str()) {
            return Err(ManifestError::InvalidField {
                field: "relative_path uniqueness",
            });
        }
        seen_roots.insert(entry.root_id.as_str());
        if entry.enabled {
            enabled_families.insert(entry.family);
            let family_index = match entry.family {
                CorpusFamily::Identity => 0,
                CorpusFamily::Scripture => 1,
                CorpusFamily::Wikitext => 2,
                CorpusFamily::Bookcorpus => 3,
            };
            enabled_family_bytes[family_index] = enabled_family_bytes[family_index]
                .checked_add(entry.bytes)
                .ok_or(ManifestError::ResourceLimitExceeded {
                    resource: "enabled family bytes",
                    limit: u64::MAX,
                    actual: u64::MAX,
                })?;
        }
    }
    match profile {
        ManifestAdmissionProfile::CompositeFourFamily => {
            if CorpusFamily::ALL
                .into_iter()
                .any(|family| !enabled_families.contains(&family))
            {
                return Err(ManifestError::InvalidField {
                    field: "enabled family coverage",
                });
            }
            if enabled_family_bytes.contains(&0) {
                return Err(ManifestError::InvalidField {
                    field: "enabled family bytes",
                });
            }
        }
        ManifestAdmissionProfile::DemoWikitextOnly => {
            if wire.entries.len() != 1
                || wire.entries[0].family != CorpusFamily::Wikitext
                || !wire.entries[0].enabled
                || enabled_families.len() != 1
                || !enabled_families.contains(&CorpusFamily::Wikitext)
                || enabled_family_bytes[2] == 0
            {
                return Err(ManifestError::InvalidField {
                    field: "demo_wikitext_only coverage",
                });
            }
        }
    }
    if seen_roots.len() > MAX_COMPOSITE_MANIFEST_ROOTS {
        return Err(ManifestError::ResourceLimitExceeded {
            resource: "manifest_root_ids",
            limit: MAX_COMPOSITE_MANIFEST_ROOTS as u64,
            actual: seen_roots.len() as u64,
        });
    }
    Ok(())
}

fn validate_roots(roots: &[ManifestRoot<'_>]) -> Result<Vec<RootScope>, ManifestError> {
    if roots.is_empty() || roots.len() > MAX_COMPOSITE_MANIFEST_ROOTS {
        return Err(ManifestError::ResourceLimitExceeded {
            resource: "manifest_roots",
            limit: MAX_COMPOSITE_MANIFEST_ROOTS as u64,
            actual: roots.len() as u64,
        });
    }
    let mut scopes = Vec::new();
    scopes
        .try_reserve(roots.len())
        .map_err(|_| ManifestError::ResourceLimitExceeded {
            resource: "manifest_root_allocation",
            limit: MAX_COMPOSITE_MANIFEST_ROOTS as u64,
            actual: roots.len() as u64,
        })?;
    let mut ids = HashSet::new();
    let mut identities = HashSet::new();
    for root in roots {
        validate_root_id(root.id)?;
        if !ids.insert(root.id) {
            return Err(ManifestError::InvalidField {
                field: "root id uniqueness",
            });
        }
        let absolute = absolute_path(root.path)?;
        ensure_no_link_like_ancestors(&absolute)?;
        let canonical = fs::canonicalize(&absolute)
            .map_err(|source| io_error("canonicalize manifest root", source))?;
        ensure_no_link_like_ancestors(&canonical)?;
        let metadata = fs::symlink_metadata(&canonical)
            .map_err(|source| io_error("inspect manifest root", source))?;
        if metadata_is_link_like(&metadata) || !metadata.is_dir() {
            return Err(path_rejected(&canonical));
        }
        let identity = file_identity(&canonical, &metadata)?;
        if !identities.insert(identity) {
            return Err(ManifestError::InvalidField {
                field: "root filesystem identity uniqueness",
            });
        }
        scopes.push(RootScope {
            id: root.id.to_owned(),
            canonical_path: canonical,
            filesystem_scope: identity.scope,
        });
    }
    for (index, left) in scopes.iter().enumerate() {
        for right in scopes.iter().skip(index + 1) {
            if left.canonical_path.starts_with(&right.canonical_path)
                || right.canonical_path.starts_with(&left.canonical_path)
            {
                return Err(ManifestError::InvalidField {
                    field: "overlapping roots",
                });
            }
        }
    }
    Ok(scopes)
}

fn inventory_roots(
    roots: &[RootScope],
) -> Result<BTreeMap<(String, String), InventoryFile>, ManifestError> {
    let mut inventory = BTreeMap::new();
    let mut identities = HashSet::new();
    for root in roots {
        let mut stack = vec![root.canonical_path.clone()];
        while let Some(directory) = stack.pop() {
            let mut entries = Vec::new();
            for entry in fs::read_dir(&directory)
                .map_err(|source| io_error("inventory manifest root", source))?
            {
                let entry = entry.map_err(|source| io_error("read manifest root entry", source))?;
                let name = entry
                    .file_name()
                    .into_string()
                    .map_err(|_| path_rejected(&entry.path()))?;
                entries.push((name, entry));
            }
            entries.sort_by(|left, right| left.0.as_bytes().cmp(right.0.as_bytes()));
            for (_, entry) in entries.into_iter().rev() {
                let path = entry.path();
                let metadata = fs::symlink_metadata(&path)
                    .map_err(|source| io_error("inspect manifest inventory entry", source))?;
                if metadata_is_link_like(&metadata) {
                    return Err(path_rejected(&path));
                }
                let canonical = fs::canonicalize(&path)
                    .map_err(|source| io_error("canonicalize manifest inventory entry", source))?;
                if !canonical.starts_with(&root.canonical_path) {
                    return Err(path_rejected(&path));
                }
                let identity = file_identity(&canonical, &metadata)?;
                if identity.scope != root.filesystem_scope {
                    return Err(path_rejected(&path));
                }
                if metadata.is_dir() {
                    stack.push(canonical);
                    continue;
                }
                if !metadata.is_file() || !identities.insert(identity) {
                    return Err(path_rejected(&path));
                }
                let relative = normalized_relative_from_root(&root.canonical_path, &canonical)?;
                let key = (root.id.clone(), relative);
                if inventory
                    .insert(
                        key,
                        InventoryFile {
                            absolute_path: canonical,
                            bytes: metadata.len(),
                            identity,
                        },
                    )
                    .is_some()
                {
                    return Err(ManifestError::FileSetMismatch);
                }
            }
        }
    }
    Ok(inventory)
}

fn verify_declared_file(
    admitted: &AdmittedFile,
    collect: bool,
) -> Result<Option<Vec<u8>>, ManifestError> {
    let metadata = fs::symlink_metadata(&admitted.absolute_path)
        .map_err(|source| io_error("inspect admitted file", source))?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(path_rejected(&admitted.absolute_path));
    }
    if metadata.len() != admitted.bytes
        || file_identity(&admitted.absolute_path, &metadata)? != admitted.identity
    {
        return Err(ManifestError::FileMetadataMismatch {
            path: admitted.relative_path.clone(),
            field: "pre-open identity or size",
        });
    }
    let mut file = File::open(&admitted.absolute_path)
        .map_err(|source| io_error("open admitted file", source))?;
    let opened_metadata = file
        .metadata()
        .map_err(|source| io_error("inspect opened admitted file", source))?;
    if opened_metadata.len() != admitted.bytes
        || file_identity(&admitted.absolute_path, &opened_metadata)? != admitted.identity
    {
        return Err(ManifestError::FileMetadataMismatch {
            path: admitted.relative_path.clone(),
            field: "opened identity or size",
        });
    }

    let mut content = if collect {
        let capacity =
            usize::try_from(admitted.bytes).map_err(|_| ManifestError::ResourceLimitExceeded {
                resource: "admitted content allocation",
                limit: usize::MAX as u64,
                actual: admitted.bytes,
            })?;
        let mut bytes = Vec::new();
        bytes
            .try_reserve_exact(capacity)
            .map_err(|_| ManifestError::ResourceLimitExceeded {
                resource: "admitted content allocation",
                limit: admitted.bytes,
                actual: admitted.bytes,
            })?;
        Some(bytes)
    } else {
        None
    };
    let mut sha256 = Sha256::new();
    let mut sha512 = Sha512::new();
    let mut utf8 = Utf8Validator::default();
    let mut actual_bytes = 0u64;
    let mut buffer = [0u8; HASH_BUFFER_BYTES];
    loop {
        let read = file
            .read(&mut buffer)
            .map_err(|source| io_error("read admitted file", source))?;
        if read == 0 {
            break;
        }
        let chunk = &buffer[..read];
        actual_bytes =
            actual_bytes
                .checked_add(read as u64)
                .ok_or(ManifestError::ResourceLimitExceeded {
                    resource: "admitted file read bytes",
                    limit: u64::MAX,
                    actual: u64::MAX,
                })?;
        sha256.update(chunk);
        sha512.update(chunk);
        if admitted.encoding_policy == EncodingPolicy::Utf8 {
            utf8.push(chunk)
                .map_err(|()| ManifestError::EncodingMismatch {
                    path: admitted.relative_path.clone(),
                })?;
        }
        if let Some(bytes) = &mut content {
            bytes.extend_from_slice(chunk);
        }
    }
    if admitted.encoding_policy == EncodingPolicy::Utf8 && !utf8.finish() {
        return Err(ManifestError::EncodingMismatch {
            path: admitted.relative_path.clone(),
        });
    }
    let final_metadata = file
        .metadata()
        .map_err(|source| io_error("reinspect admitted file", source))?;
    if actual_bytes != admitted.bytes
        || final_metadata.len() != admitted.bytes
        || file_identity(&admitted.absolute_path, &final_metadata)? != admitted.identity
    {
        return Err(ManifestError::FileMetadataMismatch {
            path: admitted.relative_path.clone(),
            field: "post-read identity or size",
        });
    }
    if finalize_sha256(sha256) != admitted.sha256 {
        return Err(ManifestError::HashMismatch {
            path: admitted.relative_path.clone(),
            algorithm: "SHA-256",
        });
    }
    if finalize_sha512(sha512) != admitted.sha512 {
        return Err(ManifestError::HashMismatch {
            path: admitted.relative_path.clone(),
            algorithm: "SHA-512",
        });
    }
    Ok(content)
}

#[derive(Default)]
struct Utf8Validator {
    pending: [u8; 3],
    pending_len: usize,
}

impl Utf8Validator {
    fn push(&mut self, chunk: &[u8]) -> Result<(), ()> {
        let mut combined = [0u8; HASH_BUFFER_BYTES + 3];
        combined[..self.pending_len].copy_from_slice(&self.pending[..self.pending_len]);
        combined[self.pending_len..self.pending_len + chunk.len()].copy_from_slice(chunk);
        let length = self.pending_len + chunk.len();
        match std::str::from_utf8(&combined[..length]) {
            Ok(_) => {
                self.pending_len = 0;
                Ok(())
            }
            Err(error) if error.error_len().is_none() => {
                let remainder = length - error.valid_up_to();
                if remainder == 0 || remainder > self.pending.len() {
                    return Err(());
                }
                self.pending[..remainder].copy_from_slice(&combined[error.valid_up_to()..length]);
                self.pending_len = remainder;
                Ok(())
            }
            Err(_) => Err(()),
        }
    }

    const fn finish(&self) -> bool {
        self.pending_len == 0
    }
}

fn read_bounded_regular_file(
    path: &Path,
    max_bytes: usize,
    operation: &'static str,
) -> Result<Vec<u8>, ManifestError> {
    let absolute = absolute_path(path)?;
    ensure_no_link_like_ancestors(&absolute)?;
    let metadata = fs::symlink_metadata(&absolute).map_err(|source| io_error(operation, source))?;
    if metadata_is_link_like(&metadata) || !metadata.is_file() {
        return Err(path_rejected(&absolute));
    }
    let max_bytes_u64 = max_bytes as u64;
    if metadata.len() > max_bytes_u64 {
        return Err(ManifestError::ResourceLimitExceeded {
            resource: "manifest_file_bytes",
            limit: max_bytes_u64,
            actual: metadata.len(),
        });
    }
    let identity = file_identity(&absolute, &metadata)?;
    let mut file = File::open(&absolute).map_err(|source| io_error(operation, source))?;
    let opened = file
        .metadata()
        .map_err(|source| io_error(operation, source))?;
    if opened.len() != metadata.len() || file_identity(&absolute, &opened)? != identity {
        return Err(ManifestError::FileMetadataMismatch {
            path: absolute.display().to_string(),
            field: "manifest opened identity or size",
        });
    }
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(metadata.len() as usize)
        .map_err(|_| ManifestError::ResourceLimitExceeded {
            resource: "manifest_file_allocation",
            limit: max_bytes_u64,
            actual: metadata.len(),
        })?;
    (&mut file)
        .take(max_bytes_u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|source| io_error(operation, source))?;
    if bytes.len() as u64 != metadata.len() {
        return Err(ManifestError::FileMetadataMismatch {
            path: absolute.display().to_string(),
            field: "manifest read length",
        });
    }
    let final_opened = file
        .metadata()
        .map_err(|source| io_error(operation, source))?;
    let final_path =
        fs::symlink_metadata(&absolute).map_err(|source| io_error(operation, source))?;
    if metadata_is_link_like(&final_path)
        || final_opened.len() != metadata.len()
        || final_path.len() != metadata.len()
        || file_identity(&absolute, &final_opened)? != identity
        || file_identity(&absolute, &final_path)? != identity
    {
        return Err(ManifestError::FileMetadataMismatch {
            path: absolute.display().to_string(),
            field: "manifest post-read identity or size",
        });
    }
    Ok(bytes)
}

fn validate_root_id(value: &str) -> Result<(), ManifestError> {
    if value.is_empty()
        || value.len() > MAX_ROOT_ID_BYTES
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-'))
    {
        return Err(ManifestError::InvalidField { field: "root_id" });
    }
    Ok(())
}

fn validate_relative_path(value: &str) -> Result<(), ManifestError> {
    if value.is_empty()
        || value.len() > MAX_RELATIVE_PATH_BYTES
        || value.starts_with('/')
        || value.starts_with("//")
        || value.contains(['\\', '\0', ':'])
    {
        return Err(ManifestError::PathRejected {
            path: value.to_owned(),
        });
    }
    for segment in value.split('/') {
        if segment.is_empty() || segment == "." || segment == ".." {
            return Err(ManifestError::PathRejected {
                path: value.to_owned(),
            });
        }
    }
    Ok(())
}

fn normalized_relative_from_root(root: &Path, path: &Path) -> Result<String, ManifestError> {
    let relative = path.strip_prefix(root).map_err(|_| path_rejected(path))?;
    let mut segments = Vec::new();
    for component in relative.components() {
        let Component::Normal(segment) = component else {
            return Err(path_rejected(path));
        };
        let segment = segment.to_str().ok_or_else(|| path_rejected(path))?;
        segments.push(segment);
    }
    let normalized = segments.join("/");
    validate_relative_path(&normalized)?;
    Ok(normalized)
}

fn absolute_path(path: &Path) -> Result<PathBuf, ManifestError> {
    if path.is_absolute() {
        Ok(path.to_path_buf())
    } else {
        Ok(std::env::current_dir()
            .map_err(|source| io_error("resolve current directory", source))?
            .join(path))
    }
}

fn ensure_no_link_like_ancestors(path: &Path) -> Result<(), ManifestError> {
    for ancestor in path.ancestors().filter(|path| !path.as_os_str().is_empty()) {
        let metadata = fs::symlink_metadata(ancestor)
            .map_err(|source| io_error("inspect path ancestor", source))?;
        if metadata_is_link_like(&metadata) {
            return Err(path_rejected(ancestor));
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
fn file_identity(path: &Path, metadata: &Metadata) -> Result<FileIdentity, ManifestError> {
    use std::os::windows::ffi::OsStrExt;
    use std::os::windows::fs::MetadataExt;
    let mut index = 1469598103934665603u64;
    const PRIME: u64 = 1099511628211;
    for unit in path.as_os_str().encode_wide() {
        for byte in unit.to_le_bytes() {
            index ^= u64::from(byte);
            index = index.wrapping_mul(PRIME);
        }
    }
    for value in [
        metadata.file_size(),
        metadata.creation_time(),
        metadata.last_write_time(),
        u64::from(metadata.file_attributes()),
    ] {
        for byte in value.to_le_bytes() {
            index ^= u64::from(byte);
            index = index.wrapping_mul(PRIME);
        }
    }
    Ok(FileIdentity { scope: 0, index })
}

#[cfg(unix)]
fn file_identity(_path: &Path, metadata: &Metadata) -> Result<FileIdentity, ManifestError> {
    use std::os::unix::fs::MetadataExt;
    Ok(FileIdentity {
        scope: metadata.dev(),
        index: metadata.ino(),
    })
}

#[cfg(not(any(windows, unix)))]
fn file_identity(_path: &Path, _metadata: &Metadata) -> Result<FileIdentity, ManifestError> {
    Err(ManifestError::FileMetadataMismatch {
        path: "<filesystem identity>".to_owned(),
        field: "unsupported platform identity",
    })
}

fn validate_upper_hex(
    field: &'static str,
    value: &str,
    decoded_bytes: usize,
) -> Result<(), ManifestError> {
    if value.len() != decoded_bytes * 2
        || !value
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'A'..=b'F').contains(&byte))
        || value.bytes().all(|byte| byte == b'0')
    {
        return Err(ManifestError::InvalidField { field });
    }
    Ok(())
}

fn decode_upper_hex<const N: usize>(
    field: &'static str,
    value: &str,
) -> Result<[u8; N], ManifestError> {
    validate_upper_hex(field, value, N)?;
    let mut decoded = [0u8; N];
    for (output, pair) in decoded.iter_mut().zip(value.as_bytes().chunks_exact(2)) {
        let high = decode_nibble(pair[0]).ok_or(ManifestError::InvalidField { field })?;
        let low = decode_nibble(pair[1]).ok_or(ManifestError::InvalidField { field })?;
        *output = (high << 4) | low;
    }
    Ok(decoded)
}

const fn decode_nibble(byte: u8) -> Option<u8> {
    match byte {
        b'0'..=b'9' => Some(byte - b'0'),
        b'A'..=b'F' => Some(byte - b'A' + 10),
        _ => None,
    }
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

fn io_error(operation: &'static str, source: io::Error) -> ManifestError {
    ManifestError::Io { operation, source }
}

fn path_rejected(path: &Path) -> ManifestError {
    ManifestError::PathRejected {
        path: path.display().to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{BPE_TOKEN_START, TokenizerTrainer, TrainConfig, base_byte_token};
    use std::fs::OpenOptions;
    use std::io::Write;
    use std::sync::atomic::{AtomicU64, Ordering};

    static NEXT_TEST_ORDINAL: AtomicU64 = AtomicU64::new(0);

    struct Fixture {
        root: PathBuf,
        manifest_path: PathBuf,
        wire: ManifestWire,
    }

    fn unique_root(label: &str) -> PathBuf {
        let ordinal = NEXT_TEST_ORDINAL.fetch_add(1, Ordering::Relaxed);
        std::env::temp_dir().join(format!(
            "ashira_v3_manifest_{}_{}_{label}",
            std::process::id(),
            ordinal
        ))
    }

    fn write_create_new(path: &Path, bytes: &[u8]) {
        let mut file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("create unique manifest fixture file");
        file.write_all(bytes).expect("write manifest fixture file");
        file.sync_all().expect("sync manifest fixture file");
    }

    fn hex_upper(bytes: &[u8]) -> String {
        const HEX: &[u8; 16] = b"0123456789ABCDEF";
        let mut output = String::with_capacity(bytes.len() * 2);
        for byte in bytes {
            output.push(char::from(HEX[usize::from(byte >> 4)]));
            output.push(char::from(HEX[usize::from(byte & 0x0F)]));
        }
        output
    }

    fn entry(
        ordinal: u32,
        family: CorpusFamily,
        relative_path: &str,
        bytes: &[u8],
        encoding_policy: EncodingPolicy,
    ) -> ManifestEntryWire {
        ManifestEntryWire {
            ordinal,
            family,
            root_id: "demo".to_owned(),
            relative_path: relative_path.to_owned(),
            enabled: true,
            bytes: bytes.len() as u64,
            sha256: hex_upper(&Sha256::digest(bytes)),
            sha512: hex_upper(&Sha512::digest(bytes)),
            encoding_policy,
        }
    }

    fn write_canonical_manifest(path: &Path, wire: &ManifestWire) {
        let mut bytes = serde_json::to_vec(wire).expect("serialize manifest fixture");
        bytes.push(b'\n');
        write_create_new(path, &bytes);
    }

    fn fixture(label: &str, book_bytes: &[u8], book_policy: EncodingPolicy) -> Fixture {
        let root = unique_root(label);
        let data = root.join("data");
        fs::create_dir_all(&data).expect("create manifest fixture root");
        let files = [
            (
                "a_identity.txt",
                CorpusFamily::Identity,
                b"aa\n".as_slice(),
                EncodingPolicy::Utf8,
            ),
            (
                "b_scripture.txt",
                CorpusFamily::Scripture,
                b"aa\n".as_slice(),
                EncodingPolicy::Utf8,
            ),
            (
                "c_wikitext.txt",
                CorpusFamily::Wikitext,
                b"aa\n".as_slice(),
                EncodingPolicy::Utf8,
            ),
            (
                "d_bookcorpus.bin",
                CorpusFamily::Bookcorpus,
                book_bytes,
                book_policy,
            ),
        ];
        let mut entries = Vec::new();
        for (index, (name, family, bytes, policy)) in files.into_iter().enumerate() {
            write_create_new(&data.join(name), bytes);
            entries.push(entry((index + 1) as u32, family, name, bytes, policy));
        }
        let wire = ManifestWire {
            schema: COMPOSITE_MANIFEST_SCHEMA.to_owned(),
            label: None,
            entries,
        };
        let manifest_path = root.join("manifest.json");
        write_canonical_manifest(&manifest_path, &wire);
        Fixture {
            root: data,
            manifest_path,
            wire,
        }
    }

    fn admit(fixture: &Fixture) -> Result<AdmittedCorpus, ManifestError> {
        admit_corpus_manifest(
            &fixture.manifest_path,
            &[ManifestRoot {
                id: "demo",
                path: &fixture.root,
            }],
        )
    }

    fn demo_fixture(label: &str) -> Fixture {
        let root = unique_root(label);
        let data = root.join("corpus");
        fs::create_dir_all(&data).expect("create demo manifest root");
        let bytes = b"deterministic wikitext demo\n";
        write_create_new(&data.join("wikitext.txt"), bytes);
        let wire = ManifestWire {
            schema: DEMO_WIKITEXT_MANIFEST_SCHEMA.to_owned(),
            label: Some(DEMO_WIKITEXT_MANIFEST_LABEL.to_owned()),
            entries: vec![entry(
                1,
                CorpusFamily::Wikitext,
                "wikitext.txt",
                bytes,
                EncodingPolicy::Utf8,
            )],
        };
        let manifest_path = root.join("demo_wikitext_manifest.json");
        write_canonical_manifest(&manifest_path, &wire);
        Fixture {
            root: data,
            manifest_path,
            wire,
        }
    }

    fn admit_demo(fixture: &Fixture) -> Result<AdmittedCorpus, ManifestError> {
        admit_demo_wikitext_manifest(
            &fixture.manifest_path,
            &[ManifestRoot {
                id: "demo",
                path: &fixture.root,
            }],
        )
    }

    #[test]
    fn demo_wikitext_profile_is_explicit_and_cannot_weaken_composite_admission() {
        let demo = demo_fixture("demo_profile");
        let admitted = admit_demo(&demo).expect("admit explicit WikiText demo manifest");
        assert_eq!(
            admitted.profile(),
            ManifestAdmissionProfile::DemoWikitextOnly
        );
        assert_eq!(admitted.files().len(), 1);
        assert_eq!(admitted.files()[0].family(), CorpusFamily::Wikitext);
        assert_eq!(admitted.total_bytes(), 28);

        assert_eq!(
            admit(&demo)
                .expect_err("composite admission must reject demo schema")
                .class(),
            "InvalidField"
        );

        let composite = fixture("composite_not_demo", b"aa\n", EncodingPolicy::Bytes);
        assert_eq!(
            admit_demo(&composite)
                .expect_err("demo admission must reject composite schema")
                .class(),
            "InvalidField"
        );
    }

    #[test]
    fn demo_wikitext_profile_rejects_wrong_label_and_family() {
        let fixture = demo_fixture("demo_negative");

        let mut wrong_label = fixture.wire.clone();
        wrong_label.label = Some("balanced_composite".to_owned());
        let wrong_label_path = fixture.manifest_path.with_file_name("wrong_label.json");
        write_canonical_manifest(&wrong_label_path, &wrong_label);
        assert_eq!(
            admit_demo_wikitext_manifest(
                &wrong_label_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &fixture.root,
                }],
            )
            .expect_err("wrong demo label must fail")
            .class(),
            "InvalidField"
        );

        let mut wrong_family = fixture.wire.clone();
        wrong_family.entries[0].family = CorpusFamily::Bookcorpus;
        let wrong_family_path = fixture.manifest_path.with_file_name("wrong_family.json");
        write_canonical_manifest(&wrong_family_path, &wrong_family);
        assert_eq!(
            admit_demo_wikitext_manifest(
                &wrong_family_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &fixture.root,
                }],
            )
            .expect_err("non-WikiText demo family must fail")
            .class(),
            "InvalidField"
        );
    }

    #[test]
    fn bundled_wikitext_demo_manifest_admits_read_only() {
        let source_root = Path::new(env!("CARGO_MANIFEST_DIR"));
        let manifest = source_root.join("demo/demo_wikitext_manifest.json");
        let corpus_root = source_root.join("demo/corpus");
        let admitted = admit_demo_wikitext_manifest(
            &manifest,
            &[ManifestRoot {
                id: "demo_wikitext",
                path: &corpus_root,
            }],
        )
        .expect("admit bundled WikiText-only demo manifest");
        assert_eq!(
            admitted.profile(),
            ManifestAdmissionProfile::DemoWikitextOnly
        );
        assert_eq!(admitted.files().len(), 1);
        assert_eq!(admitted.files()[0].relative_path(), "wikitext.txt");
        assert_eq!(admitted.total_bytes(), 1_047_897);
        assert_eq!(
            admitted.files()[0].sha256(),
            decode_upper_hex::<32>(
                "bundled demo sha256",
                "71A024D25F902E2F5911E475A68DB9749D58AA560A3A37A5BAE1A0FE20867454",
            )
            .expect("decode locked bundled demo digest")
        );
    }

    #[test]
    fn canonical_manifest_admits_trains_and_freezes_on_one_path() {
        let fixture = fixture("valid", b"aa\n", EncodingPolicy::Bytes);
        let admitted = admit(&fixture).expect("admit canonical four-family manifest");
        assert_eq!(admitted.files().len(), 4);
        assert_eq!(admitted.total_bytes(), 12);
        assert_eq!(admitted.files()[0].ordinal(), 1);
        assert_eq!(admitted.files()[3].family(), CorpusFamily::Bookcorpus);

        let weights = FamilyWeights::try_new(5_000, 3_000, 4_000, 4_000)
            .expect("positive explicit demo weights");
        let files = admitted.training_files(weights);
        let config = TrainConfig {
            vocab_size: usize::try_from(BPE_TOKEN_START + 1).expect("small target"),
            min_frequency: 1,
            deterministic: true,
        };
        let mut trainer = TokenizerTrainer::new();
        let stats = trainer
            .train_weighted(&files, &config)
            .expect("bounded manifest training");
        assert_eq!(stats.learned_merges, 1);
        let mut repeated = TokenizerTrainer::new();
        repeated
            .train_weighted(&files, &config)
            .expect("repeat bounded manifest training");
        assert_eq!(trainer.compute_hash_hex(), repeated.compute_hash_hex());
        let tokenizer = trainer.freeze().expect("freeze trained tokenizer");
        let byte_a = base_byte_token(b'a');
        assert_eq!(
            tokenizer.merged_token(byte_a, byte_a),
            Some(BPE_TOKEN_START)
        );
        assert_eq!(
            tokenizer.token_bytes(BPE_TOKEN_START),
            Some(b"aa".as_slice())
        );
    }

    #[test]
    fn manifest_rejects_noncanonical_paths_order_and_missing_family() {
        let zero_family = fixture("zero_family", b"", EncodingPolicy::Bytes);
        assert_eq!(
            admit(&zero_family)
                .expect_err("enabled family bytes must be nonzero")
                .class(),
            "InvalidField"
        );

        let fixture = fixture("schema_failures", b"aa\n", EncodingPolicy::Bytes);

        let extra_root = unique_root("undeclared_root");
        fs::create_dir_all(&extra_root).expect("create undeclared root fixture");
        assert_eq!(
            admit_corpus_manifest(
                &fixture.manifest_path,
                &[
                    ManifestRoot {
                        id: "demo",
                        path: &fixture.root,
                    },
                    ManifestRoot {
                        id: "extra",
                        path: &extra_root,
                    },
                ],
            )
            .expect_err("supplied roots must exactly equal declared roots")
            .class(),
            "InvalidField"
        );

        let mut traversal = fixture.wire.clone();
        traversal.entries[0].relative_path = "../escape.txt".to_owned();
        let traversal_path = fixture.manifest_path.with_file_name("traversal.json");
        write_canonical_manifest(&traversal_path, &traversal);
        assert_eq!(
            admit_corpus_manifest(
                &traversal_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &fixture.root,
                }],
            )
            .expect_err("traversal must fail")
            .class(),
            "PathRejected"
        );

        let mut missing_family = fixture.wire.clone();
        missing_family.entries[3].family = CorpusFamily::Wikitext;
        let family_path = fixture.manifest_path.with_file_name("missing_family.json");
        write_canonical_manifest(&family_path, &missing_family);
        assert_eq!(
            admit_corpus_manifest(
                &family_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &fixture.root,
                }],
            )
            .expect_err("missing enabled family must fail")
            .class(),
            "InvalidField"
        );

        let pretty_path = fixture.manifest_path.with_file_name("pretty.json");
        let mut pretty = serde_json::to_vec_pretty(&fixture.wire).expect("pretty manifest");
        pretty.push(b'\n');
        write_create_new(&pretty_path, &pretty);
        assert_eq!(
            admit_corpus_manifest(
                &pretty_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &fixture.root,
                }],
            )
            .expect_err("noncanonical whitespace must fail")
            .class(),
            "NonCanonicalManifest"
        );
    }

    #[test]
    fn manifest_rejects_hash_encoding_and_inventory_mismatches() {
        let hash_fixture = fixture("hash_failure", b"aa\n", EncodingPolicy::Bytes);
        let mut bad_hash = hash_fixture.wire.clone();
        bad_hash.entries[0].sha256 = "AA".repeat(32);
        let bad_hash_path = hash_fixture.manifest_path.with_file_name("bad_hash.json");
        write_canonical_manifest(&bad_hash_path, &bad_hash);
        assert_eq!(
            admit_corpus_manifest(
                &bad_hash_path,
                &[ManifestRoot {
                    id: "demo",
                    path: &hash_fixture.root,
                }],
            )
            .expect_err("hash mismatch must fail")
            .class(),
            "HashMismatch"
        );

        let inventory_fixture = fixture("inventory_failure", b"aa\n", EncodingPolicy::Bytes);
        write_create_new(&inventory_fixture.root.join("undeclared.txt"), b"extra");
        assert_eq!(
            admit(&inventory_fixture)
                .expect_err("undeclared file must fail")
                .class(),
            "FileSetMismatch"
        );

        let utf8_fixture = fixture("utf8_failure", &[0xFF], EncodingPolicy::Utf8);
        assert_eq!(
            admit(&utf8_fixture)
                .expect_err("invalid UTF-8 must fail")
                .class(),
            "EncodingMismatch"
        );
    }

    #[test]
    fn admitted_training_read_rechecks_content_before_ingestion() {
        let fixture = fixture("read_recheck", b"aa\n", EncodingPolicy::Bytes);
        let admitted = admit(&fixture).expect("initial admission");
        let first = admitted.files()[0].clone();
        let mut file = OpenOptions::new()
            .append(true)
            .open(first.path())
            .expect("open retained fixture for mutation");
        file.write_all(b"changed").expect("mutate retained fixture");
        file.sync_all().expect("sync retained mutation");
        assert_eq!(
            first
                .read_verified_bytes()
                .expect_err("changed file must fail before ingestion")
                .class(),
            "FileMetadataMismatch"
        );
    }

    #[cfg(windows)]
    #[test]
    fn manifest_inventory_rejects_live_windows_reparse_entry() {
        use std::os::windows::fs::symlink_file;

        let fixture = fixture("reparse", b"aa\n", EncodingPolicy::Bytes);
        let target = fixture.manifest_path.with_file_name("outside_target.txt");
        write_create_new(&target, b"outside");
        symlink_file(&target, fixture.root.join("undeclared_link.txt"))
            .expect("create unprivileged file symlink fixture");
        assert_eq!(
            admit(&fixture)
                .expect_err("reparse entry must fail closed")
                .class(),
            "PathRejected"
        );
    }
}
