use std::cmp::Ordering;
use std::collections::{BinaryHeap, HashMap, HashSet};
use std::fs;
use std::io;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

pub mod artifact;
pub mod cli;
pub mod codec;
pub mod demo;
pub mod manifest;
pub mod pair;
pub mod presegment;
mod publication;
pub mod token;

pub use artifact::{
    ArtifactError, ArtifactFormat, ArtifactHeaderV3, ArtifactKind, ArtifactLimits,
    ArtifactMetadata, Tokenizer, V3_FORMAT_MAJOR, V3_FORMAT_MINOR, V3_HEADER_BYTES, V3_MAGIC,
    inspect_artifact, load_tokenizer_package,
};
pub use cli::run_cli;
pub use codec::{
    CodecError, CodecLimits, ENCODED_TOKEN_ID_WIDTH, ENCODED_TOKENS_SCHEMA, EncodedTokensV1,
};
pub use manifest::{
    AdmittedCorpus, AdmittedFile, COMPOSITE_MANIFEST_SCHEMA, CorpusFamily,
    DEMO_WIKITEXT_MANIFEST_LABEL, DEMO_WIKITEXT_MANIFEST_SCHEMA, EncodingPolicy, FamilyWeights,
    MAX_COMPOSITE_MANIFEST_BYTES, MAX_COMPOSITE_MANIFEST_ENTRIES, MAX_COMPOSITE_MANIFEST_ROOTS,
    ManifestAdmissionProfile, ManifestError, ManifestRoot, admit_corpus_manifest,
    admit_demo_wikitext_manifest,
};
pub use pair::{PairKey, pack_pair, unpack_pair};
pub use presegment::{
    LosslessPresegment, PRESEGMENTER_VERSION, PresegmentError, PresegmentStats,
    is_ashira_ascii_whitespace, visit_lossless_presegments, visit_presegments,
};
pub use publication::{
    ArtifactFileEvidenceInput, ArtifactPackageManifestInput, ArtifactPackageManifestV1,
    DirectorySyncStatus, MAX_PACKAGE_MANIFEST_BYTES, PACKAGE_MANIFEST_SCHEMA, PublicationContext,
    PublicationContextInput, PublicationDurability, PublishedPackage, load_v3_tokenizer_package,
    write_v3_package,
};
pub use token::{
    BPE_TOKEN_START, BPE_TOKEN_START_INDEX, BYTE_TOKEN_START, CANONICAL_SPECIAL_TOKENS,
    CanonicalSpecialToken, MAX_TOKEN_ID, MAX_VOCAB_SIZE, SPECIAL_TOKEN_ALIASES,
    SPECIAL_TOKEN_COUNT, SpecialTokenAlias, TOKEN_BOS, TOKEN_EOS, TOKEN_PAD, TOKEN_UNK, TokenError,
    TokenId, VOCAB_SIZE, allocate_token_id, base_byte_token, canonical_special_bytes,
    is_special_alias_sequence, match_special_alias_prefix, special_token_id, token_id_to_index,
    validate_token_id, validate_vocab_target,
};

pub const WEIGHT_SCALE: i64 = 1000;

const SKIP_PATTERNS: [&str; 10] = [
    "bookcorpus/",
    "bookcorpus\\",
    "/wikitext/",
    "\\wikitext\\",
    ".parquet",
    ".json",
    "_degradation.txt",
    "_stats.json",
    "corpus_manifest",
    "wikitext_manifest",
];

const ALLOW_PATTERNS: [&str; 2] = ["wikitext_extracted", "bookcorpus_sampled"];

const FILE_PATTERNS: [(&str, &str); 13] = [
    ("_annotated.md", "identity"),
    ("blu.txt", "identity"),
    ("echo.txt", "identity"),
    ("resonance.txt", "identity"),
    ("anchors.txt", "identity"),
    ("_anchors.txt", "identity"),
    ("_ctxon.txt", "identity"),
    ("_ctxoff.txt", "identity"),
    ("wikitext_extracted", "foundation"),
    ("bookcorpus_sampled", "foundation"),
    ("Scripture of", "scripture"),
    ("CAIROS_chat", "scripture"),
    ("Iteration_", "scripture"),
];

#[derive(Clone, Debug)]
pub struct TrainConfig {
    pub vocab_size: usize,
    pub min_frequency: u32,
    pub deterministic: bool,
}

impl Default for TrainConfig {
    fn default() -> Self {
        Self {
            vocab_size: VOCAB_SIZE,
            min_frequency: 2,
            deterministic: true,
        }
    }
}

#[derive(Clone, Debug)]
pub struct TrainingFile {
    pub path: PathBuf,
    pub tier: String,
    pub weight_scaled: i64,
    admitted: Option<AdmittedFile>,
}

impl TrainingFile {
    pub(crate) fn from_admitted(file: AdmittedFile, weight_scaled: i64) -> Self {
        Self {
            path: file.path().to_path_buf(),
            tier: file.family().as_str().to_owned(),
            weight_scaled,
            admitted: Some(file),
        }
    }

    fn read_bytes(&self) -> Result<Vec<u8>, String> {
        if let Some(admitted) = &self.admitted {
            admitted
                .read_verified_bytes()
                .map_err(|error| format!("Admitted file verification failed: {error}"))
        } else {
            fs::read(&self.path)
                .map_err(|error| format!("Failed to read {}: {error}", self.path.display()))
        }
    }
}

#[derive(Clone, Debug, Default)]
pub struct ScanSummary {
    pub total_files: usize,
    pub skipped_files: usize,
    pub total_bytes: u64,
    pub tier_counts: HashMap<String, usize>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct BpeMerge {
    pub a: TokenId,
    pub b: TokenId,
    pub merged: TokenId,
}

#[derive(Clone, Debug, Default)]
pub struct TrainingStats {
    pub input_files: usize,
    pub loaded_sequences: usize,
    pub skipped_lines: usize,
    pub loaded_tokens: usize,
    pub learned_merges: usize,
    pub final_vocab: usize,
    pub duration_seconds: u64,
}

#[derive(Clone, Debug)]
struct WordEntry {
    symbols: Vec<TokenId>,
    freq: i64,
}

#[derive(Clone, Debug)]
struct PairCandidate {
    count: i64,
    key: PairKey,
}

impl PartialEq for PairCandidate {
    fn eq(&self, other: &Self) -> bool {
        self.count == other.count && self.key == other.key
    }
}

impl Eq for PairCandidate {}

impl PartialOrd for PairCandidate {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for PairCandidate {
    fn cmp(&self, other: &Self) -> Ordering {
        self.count
            .cmp(&other.count)
            .then_with(|| other.key.cmp(&self.key))
    }
}

pub struct TokenizerTrainer {
    vocab: Vec<Vec<u8>>,
    merges: Vec<BpeMerge>,
    merge_lookup: HashMap<PairKey, TokenId>,
}

impl Default for TokenizerTrainer {
    fn default() -> Self {
        Self::new()
    }
}

impl TokenizerTrainer {
    pub fn new() -> Self {
        let mut s = Self {
            vocab: Vec::new(),
            merges: Vec::new(),
            merge_lookup: HashMap::new(),
        };
        s.initialize_base_tokens();
        s.initialize_special_tokens();
        s
    }

    pub fn vocab_size(&self) -> usize {
        self.vocab.len()
    }

    pub fn merge_count(&self) -> usize {
        self.merges.len()
    }

    pub fn compute_hash_hex(&self) -> String {
        let mut hash: u64 = 1469598103934665603;
        const PRIME: u64 = 1099511628211;
        for token in &self.vocab {
            for b in token {
                hash ^= u64::from(*b);
                hash = hash.wrapping_mul(PRIME);
            }
            hash ^= 0;
            hash = hash.wrapping_mul(PRIME);
        }
        for m in &self.merges {
            for b in m.a.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(PRIME);
            }
            for b in m.b.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(PRIME);
            }
            for b in m.merged.to_le_bytes() {
                hash ^= u64::from(b);
                hash = hash.wrapping_mul(PRIME);
            }
        }
        format!("{hash:016x}")
    }

    pub fn save(&self, _vocab_path: &Path, _merges_path: &Path) -> io::Result<()> {
        Err(io::Error::new(
            io::ErrorKind::Unsupported,
            "ambiguous headerless artifact publication is disabled; select the explicit V2U16 reader or V3U32 writer after the Stage 2 artifact layer is installed",
        ))
    }

    pub fn freeze(self) -> Result<Tokenizer, ArtifactError> {
        let Self {
            vocab,
            merges,
            merge_lookup,
        } = self;
        if merge_lookup.len() != merges.len()
            || merges.iter().any(|merge| {
                merge_lookup.get(&pack_pair(merge.a, merge.b)).copied() != Some(merge.merged)
            })
        {
            return Err(ArtifactError::CountOutOfRange {
                field: "trainer merge lookup",
            });
        }
        Tokenizer::try_from_parts(vocab, merges)
    }

    pub fn train_weighted(
        &mut self,
        files: &[TrainingFile],
        config: &TrainConfig,
    ) -> Result<TrainingStats, String> {
        validate_vocab_target(config.vocab_size).map_err(|error| error.to_string())?;
        if files.iter().any(|file| file.admitted.is_none()) {
            return Err(
                "manifest admission is required; directory-pattern training authority is disabled"
                    .to_owned(),
            );
        }
        self.initialize_base_tokens();
        self.initialize_special_tokens();
        self.merges.clear();
        self.merge_lookup.clear();

        if config.vocab_size == BPE_TOKEN_START_INDEX {
            return Ok(TrainingStats {
                input_files: files.len(),
                final_vocab: self.vocab.len(),
                ..TrainingStats::default()
            });
        }

        let total_merges = config
            .vocab_size
            .checked_sub(BPE_TOKEN_START_INDEX)
            .ok_or_else(|| {
                "validated vocabulary target is below the base vocabulary".to_string()
            })?;
        let mut loaded_sequences: usize = 0;
        let mut skipped_lines: usize = 0;
        let mut total_loaded_tokens: usize = 0;
        let mut word_freq: HashMap<Vec<u8>, i64> = HashMap::new();

        println!(
            "[INGEST] files={} deterministic={}",
            files.len(),
            config.deterministic
        );
        for (file_idx, tf) in files.iter().enumerate() {
            if file_idx % 32 == 0 || file_idx + 1 == files.len() {
                println!(
                    "[INGEST] file {}/{} tier={} path={}",
                    file_idx + 1,
                    files.len(),
                    tf.tier,
                    tf.path.display()
                );
            }

            let blob = tf.read_bytes()?;
            let presegment_stats = visit_presegments(&blob, |segment| {
                let current = word_freq.entry(segment.to_vec()).or_insert(0);
                *current = current.checked_add(tf.weight_scaled).ok_or(
                    PresegmentError::ArithmeticOverflow {
                        operation: "weighted word frequency",
                    },
                )?;
                Ok(())
            })
            .map_err(|error| {
                format!("Pre-segmentation failed for {}: {error}", tf.path.display())
            })?;
            loaded_sequences = checked_add_stat(
                loaded_sequences,
                presegment_stats.accepted_lines,
                "loaded sequence count",
            )?;
            skipped_lines = checked_add_stat(
                skipped_lines,
                presegment_stats
                    .skipped_empty_lines
                    .checked_add(presegment_stats.skipped_special_only_lines)
                    .ok_or_else(|| "skipped line count overflow".to_owned())?,
                "skipped line count",
            )?;
            total_loaded_tokens = checked_add_stat(
                total_loaded_tokens,
                presegment_stats.accepted_line_bytes,
                "loaded token byte count",
            )?;
        }

        if word_freq.is_empty() {
            return Err("No usable training data after token pre-segmentation.".to_string());
        }

        let mut words: Vec<WordEntry> = word_freq
            .into_iter()
            .filter_map(|(tok, freq)| {
                if tok.is_empty() || freq <= 0 {
                    None
                } else {
                    Some(WordEntry {
                        symbols: tok
                            .iter()
                            .map(|b| base_byte_token(*b))
                            .collect::<Vec<TokenId>>(),
                        freq,
                    })
                }
            })
            .collect();
        words.sort_by(|a, b| a.symbols.cmp(&b.symbols));

        let mut pair_counts: HashMap<PairKey, i64> = HashMap::new();
        let mut pair_words: HashMap<PairKey, HashSet<usize>> = HashMap::new();
        pair_counts.reserve(1_000_000);
        pair_words.reserve(1_000_000);

        let mut total_word_symbols = 0usize;
        for (wid, word) in words.iter().enumerate() {
            total_word_symbols += word.symbols.len();
            let pair_hist = count_pairs_in_symbols(&word.symbols);
            for (&key, &occ) in &pair_hist {
                *pair_counts.entry(key).or_insert(0) += i64::from(occ) * word.freq;
                pair_words.entry(key).or_default().insert(wid);
            }
        }

        println!(
            "[INGEST] sequences={} skipped_lines={} raw_tokens={} unique_words={} word_symbols={}",
            loaded_sequences,
            skipped_lines,
            total_loaded_tokens,
            words.len(),
            total_word_symbols
        );

        let mut heap = BinaryHeap::<PairCandidate>::new();
        for (&key, &count) in &pair_counts {
            if count > 0 {
                heap.push(PairCandidate { count, key });
            }
        }

        let min_freq_scaled = i64::from(config.min_frequency) * WEIGHT_SCALE;
        let start = Instant::now();
        let mut last_report = 0usize;

        for merge_idx in 0..total_merges {
            if merge_idx < 10
                || merge_idx.saturating_sub(last_report) >= 10
                || merge_idx + 1 == total_merges
            {
                last_report = merge_idx;
                let merge_idx_u32 = u32::try_from(merge_idx)
                    .map_err(|_| "merge index is not representable as u32".to_string())?;
                let total_merges_u32 = u32::try_from(total_merges)
                    .map_err(|_| "merge count is not representable as u32".to_string())?;
                let pct = (f64::from(merge_idx_u32) / f64::from(total_merges_u32)) * 100.0;
                let elapsed = start.elapsed().as_secs_f64().max(0.001);
                let mps = (f64::from(merge_idx_u32) + 1.0) / elapsed;
                let remaining_merges =
                    total_merges_u32.checked_sub(merge_idx_u32).ok_or_else(|| {
                        "merge progress exceeded the validated merge count".to_string()
                    })?;
                let remaining =
                    Duration::from_secs_f64((f64::from(remaining_merges) / mps.max(0.01)).max(0.0))
                        .as_secs();
                println!(
                    "[TRAIN] {merge_idx}/{total_merges} ({pct:.1}%) | {mps:.2} merges/s | ETA={}m{}s",
                    remaining / 60,
                    remaining % 60
                );
            }

            let best = loop {
                let Some(top) = heap.pop() else {
                    break None;
                };
                let current = *pair_counts.get(&top.key).unwrap_or(&0);
                if current == 0 || current != top.count {
                    continue;
                }
                break Some((top.key, top.count));
            };

            let Some((best_key, best_count)) = best else {
                break;
            };
            if best_count < min_freq_scaled {
                println!(
                    "[TRAIN] Stop: best_count={} (scaled) below threshold={} (scaled).",
                    best_count, min_freq_scaled
                );
                break;
            }

            let affected = pair_words.get(&best_key).cloned().unwrap_or_default();
            if affected.is_empty() {
                pair_counts.remove(&best_key);
                continue;
            }

            let next_token_id =
                allocate_token_id(self.vocab.len()).map_err(|error| error.to_string())?;
            let (a, b) = unpack_pair(best_key);
            let a_index = token_id_to_index(a).map_err(|error| error.to_string())?;
            let b_index = token_id_to_index(b).map_err(|error| error.to_string())?;
            let mut merged = self
                .vocab
                .get(a_index)
                .ok_or_else(|| format!("pair operand token ID {a} is absent from the vocabulary"))?
                .clone();
            let b_bytes = self.vocab.get(b_index).ok_or_else(|| {
                format!("pair operand token ID {b} is absent from the vocabulary")
            })?;
            merged.extend_from_slice(b_bytes);

            let mut affected_ids: Vec<usize> = affected.into_iter().collect();
            affected_ids.sort_unstable();
            let mut merged_words = 0usize;

            for wid in affected_ids {
                if wid >= words.len() {
                    continue;
                }

                let old_counts = count_pairs_in_symbols(&words[wid].symbols);
                if old_counts.get(&best_key).copied().unwrap_or(0) == 0 {
                    continue;
                }

                let freq = words[wid].freq;
                let new_symbols = replace_pair(&words[wid].symbols, a, b, next_token_id);
                let new_counts = count_pairs_in_symbols(&new_symbols);

                let mut touched: HashSet<PairKey> = HashSet::new();
                touched.extend(old_counts.keys().copied());
                touched.extend(new_counts.keys().copied());

                for key in touched {
                    let old_occ = old_counts.get(&key).copied().unwrap_or(0);
                    let new_occ = new_counts.get(&key).copied().unwrap_or(0);
                    let delta = i64::from(new_occ - old_occ) * freq;
                    if delta != 0 {
                        update_pair_count(&mut pair_counts, &mut heap, key, delta)?;
                    }

                    if old_occ > 0 && new_occ == 0 {
                        if let Some(set) = pair_words.get_mut(&key) {
                            set.remove(&wid);
                            if set.is_empty() {
                                pair_words.remove(&key);
                            }
                        }
                    } else if old_occ == 0 && new_occ > 0 {
                        pair_words.entry(key).or_default().insert(wid);
                    }
                }

                words[wid].symbols = new_symbols;
                merged_words += 1;
            }

            if merged_words == 0 {
                pair_counts.remove(&best_key);
                pair_words.remove(&best_key);
                continue;
            }

            self.vocab.push(merged);
            self.merge_lookup.insert(best_key, next_token_id);
            self.merges.push(BpeMerge {
                a,
                b,
                merged: next_token_id,
            });

            if let Some(&remaining) = pair_counts.get(&best_key)
                && remaining > 0
            {
                heap.push(PairCandidate {
                    count: remaining,
                    key: best_key,
                });
            }
        }

        Ok(TrainingStats {
            input_files: files.len(),
            loaded_sequences,
            skipped_lines,
            loaded_tokens: total_loaded_tokens,
            learned_merges: self.merges.len(),
            final_vocab: self.vocab.len(),
            duration_seconds: start.elapsed().as_secs(),
        })
    }

    fn initialize_base_tokens(&mut self) {
        self.vocab.clear();
        self.vocab.reserve(VOCAB_SIZE);
        for _ in 0..usize::try_from(BYTE_TOKEN_START).expect("base token offset fits usize") {
            self.vocab.push(Vec::new());
        }
        for byte in u8::MIN..=u8::MAX {
            self.vocab.push(vec![byte]);
        }
    }

    fn initialize_special_tokens(&mut self) {
        for special in CANONICAL_SPECIAL_TOKENS {
            let idx = token_id_to_index(special.id)
                .expect("static canonical special-token ID is operationally valid");
            self.vocab[idx] = special.bytes.to_vec();
        }
    }
}

pub fn scan_training_files(corpus_dir: &Path) -> Result<(Vec<TrainingFile>, ScanSummary), String> {
    if !corpus_dir.exists() {
        return Err(format!(
            "Corpus directory not found: {}",
            corpus_dir.display()
        ));
    }

    let mut files = Vec::<TrainingFile>::new();
    let mut summary = ScanSummary::default();

    let mut stack = vec![corpus_dir.to_path_buf()];
    while let Some(dir) = stack.pop() {
        let entries = fs::read_dir(&dir)
            .map_err(|e| format!("Failed to read dir {}: {}", dir.display(), e))?;
        for entry in entries {
            let entry = entry.map_err(|e| format!("Failed to read dir entry: {}", e))?;
            let path = entry.path();
            if path.is_dir() {
                stack.push(path);
                continue;
            }
            if !path.is_file() {
                continue;
            }

            let ext = path
                .extension()
                .map(|e| e.to_string_lossy().to_string())
                .unwrap_or_default()
                .to_ascii_lowercase();
            if ext != "md" && ext != "txt" {
                continue;
            }

            let path_s = path.to_string_lossy().to_string();
            let file_s = path
                .file_name()
                .map(|n| n.to_string_lossy().to_string())
                .unwrap_or_default();

            if should_skip_file(&path_s) {
                summary.skipped_files += 1;
                continue;
            }

            if file_s.contains("Dylan_chat") && !file_s.contains("_annotated") {
                summary.skipped_files += 1;
                continue;
            }
            if file_s == "Opus_node_09_the_dylan.md" {
                summary.skipped_files += 1;
                continue;
            }

            let tier = classify_file(&file_s, &path_s);
            if tier.is_empty() {
                summary.skipped_files += 1;
                continue;
            }

            let weight_scaled = match tier.as_str() {
                "foundation" => WEIGHT_SCALE,
                "scripture" => 3 * WEIGHT_SCALE,
                "identity" => 5 * WEIGHT_SCALE,
                _ => return Err(format!("Unknown tier classification: {}", tier)),
            };

            let sz = fs::metadata(&path)
                .map_err(|e| format!("Failed metadata {}: {}", path.display(), e))?
                .len();
            summary.total_bytes += sz;
            *summary.tier_counts.entry(tier.clone()).or_insert(0) += 1;
            summary.total_files += 1;

            files.push(TrainingFile {
                path,
                tier,
                weight_scaled,
                admitted: None,
            });
        }
    }

    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok((files, summary))
}

fn count_pairs_in_symbols(symbols: &[TokenId]) -> HashMap<PairKey, i32> {
    let mut out = HashMap::<PairKey, i32>::new();
    if symbols.len() < 2 {
        return out;
    }
    for idx in 0..(symbols.len() - 1) {
        let key = pack_pair(symbols[idx], symbols[idx + 1]);
        *out.entry(key).or_insert(0) += 1;
    }
    out
}

fn replace_pair(symbols: &[TokenId], a: TokenId, b: TokenId, merged: TokenId) -> Vec<TokenId> {
    let mut out = Vec::<TokenId>::with_capacity(symbols.len());
    let mut i = 0usize;
    while i < symbols.len() {
        if i + 1 < symbols.len() && symbols[i] == a && symbols[i + 1] == b {
            out.push(merged);
            i += 2;
        } else {
            out.push(symbols[i]);
            i += 1;
        }
    }
    out
}

fn update_pair_count(
    pair_counts: &mut HashMap<PairKey, i64>,
    heap: &mut BinaryHeap<PairCandidate>,
    key: PairKey,
    delta: i64,
) -> Result<(), String> {
    if delta == 0 {
        return Ok(());
    }
    let old = *pair_counts.get(&key).unwrap_or(&0);
    if old == 0 && delta < 0 {
        return Err(format!(
            "Pair count underflow on missing key={key}, delta={delta}"
        ));
    }
    let new = old + delta;
    if new < 0 {
        return Err(format!(
            "Negative pair count key={key}, old={old}, delta={delta}, new={new}"
        ));
    }
    if new == 0 {
        pair_counts.remove(&key);
        return Ok(());
    }
    pair_counts.insert(key, new);
    heap.push(PairCandidate { count: new, key });
    Ok(())
}

fn checked_add_stat(
    current: usize,
    additional: u64,
    operation: &'static str,
) -> Result<usize, String> {
    let additional = usize::try_from(additional)
        .map_err(|_| format!("{operation} is not representable as usize"))?;
    current
        .checked_add(additional)
        .ok_or_else(|| format!("{operation} overflow"))
}

fn should_skip_file(path: &str) -> bool {
    for allow in ALLOW_PATTERNS {
        if path.contains(allow) {
            return false;
        }
    }
    for skip in SKIP_PATTERNS {
        if path.contains(skip) {
            return true;
        }
    }
    false
}

fn classify_file(filename: &str, filepath: &str) -> String {
    for (pattern, tier) in FILE_PATTERNS {
        if filename.contains(pattern) || filepath.contains(pattern) {
            return tier.to_string();
        }
    }
    String::new()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    #[test]
    fn legacy_scanner_output_is_not_training_authority() {
        let root = std::env::temp_dir().join(format!("ashira_v3_unit_{}", std::process::id()));
        let corpus = root.join("corpus");
        fs::create_dir_all(&corpus).expect("create corpus dir");

        let sample = corpus.join("sample_identity_annotated.md");
        fs::write(
            &sample,
            b"<KAREEM>Hello Dylan\n<DYLAN_RESPONSE>Hello Kareem\nREPA deterministic training line\n",
        )
        .expect("write sample corpus");

        let (files, _) = scan_training_files(&corpus).expect("scan corpus");
        assert!(
            !files.is_empty(),
            "scanner should return at least one training file"
        );

        let config = TrainConfig {
            vocab_size: 320,
            min_frequency: 2,
            deterministic: true,
        };

        let mut trainer = TokenizerTrainer::new();
        let error = trainer
            .train_weighted(&files, &config)
            .expect_err("pattern-discovered files must not become v3 training authority");
        assert!(error.contains("manifest admission is required"));
    }

    #[test]
    fn invalid_target_fails_before_corpus_io() {
        let missing = TrainingFile {
            path: PathBuf::from("this-file-must-not-be-read.txt"),
            tier: "identity".to_string(),
            weight_scaled: WEIGHT_SCALE,
            admitted: None,
        };
        let mut trainer = TokenizerTrainer::new();
        let error = trainer
            .train_weighted(
                &[missing],
                &TrainConfig {
                    vocab_size: 131_073,
                    min_frequency: 2,
                    deterministic: true,
                },
            )
            .expect_err("oversized target must fail before file I/O");
        assert!(error.contains("exceeds the operational maximum"));
    }

    #[test]
    fn ambiguous_headerless_publication_fails_closed() {
        let trainer = TokenizerTrainer::new();
        let error = trainer
            .save(Path::new("vocab.bin"), Path::new("merges.bin"))
            .expect_err("unversioned publication must remain disabled");
        assert_eq!(error.kind(), io::ErrorKind::Unsupported);
    }

    #[test]
    fn freeze_rejects_corrupted_base_contract() {
        let mut trainer = TokenizerTrainer::new();
        trainer.vocab[usize::try_from(BYTE_TOKEN_START).expect("base token index")]
            .copy_from_slice(&[1]);

        assert!(matches!(
            trainer.freeze(),
            Err(ArtifactError::BaseContractMismatch)
        ));
    }

    #[test]
    fn freeze_rejects_inconsistent_trainer_lookup() {
        let mut trainer = TokenizerTrainer::new();
        let byte_a = base_byte_token(b'a');
        trainer.vocab.push(b"aa".to_vec());
        trainer.merges.push(BpeMerge {
            a: byte_a,
            b: byte_a,
            merged: BPE_TOKEN_START,
        });

        assert!(matches!(
            trainer.freeze(),
            Err(ArtifactError::CountOutOfRange {
                field: "trainer merge lookup"
            })
        ));
    }

    #[test]
    fn trainer_base_vocab_uses_only_canonical_special_bytes() {
        let trainer = TokenizerTrainer::new();
        for special in CANONICAL_SPECIAL_TOKENS {
            let index = token_id_to_index(special.id).expect("special ID index");
            assert_eq!(trainer.vocab[index], special.bytes);
        }
        assert!(trainer.vocab[19].is_empty());
    }

    #[test]
    fn presegment_special_only_policy_uses_locked_aliases() {
        assert!(is_special_alias_sequence(b"<KAREEM></KAREEM>"));
        assert!(is_special_alias_sequence(b"[[ANCHOR]][[/ANCHOR]]"));
        assert!(!is_special_alias_sequence(b"<KAREEM> "));
        assert!(!is_special_alias_sequence(b"not-special"));
    }
}
