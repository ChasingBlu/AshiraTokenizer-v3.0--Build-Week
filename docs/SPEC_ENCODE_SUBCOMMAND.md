# SPEC — AshiraTokenizer v2: `encode` Subcommand Extension

| Field | Value |
|---|---|
| Spec ID | SPEC-ASHIRA-ENCODE-v1 |
| Status | READY FOR IMPLEMENTATION |
| Blocked by | Nothing — standalone extension |
| Owner | Codex — The Spook |
| Doctrine | REPA v1.0 |
| Date | 2026-03-06 |
| Consumer | ODE_transformer TASK-7 (corpus preparation) |

---

## Objective

Extend AshiraTokenizer v2 with an `encode` subcommand that:

1. Loads trained tokenizer artifacts (`vocab.bin` + `merges.bin`)
2. Reads a corpus manifest (JSON, multi-tier weighted)
3. Encodes all text files using byte-level BPE (reusing the trained merges)
4. Packs encoded tokens into fixed-length sequences of 256
5. Writes `corpus.bin` (flat binary) + `corpus_meta.json` (SHA-256-locked metadata)

This extends the existing binary, not a new binary. All existing `train` behavior is preserved unchanged.

---

## Cargo.toml Changes

Add two dependencies:

```toml
[dependencies]
sha2 = "0.10"
rand = "0.8"
```

No other changes. Zero other crates. REPA deterministic mode hardcoded ON (existing).

---

## CLI — Subcommand Detection

Current `main.rs` uses a flat arg-parser with no subcommand structure. Extend it minimally:

```
ashira_tokenizer_v2 train   [existing flags]   -- train a new tokenizer
ashira_tokenizer_v2 encode  --manifest <path>  -- encode corpus with trained tokenizer
                            --output   <path>
                            [--verbose]
```

**Implementation rule:** First positional argument determines mode. If `argv[1]` equals `"encode"`, route to `run_encode()`. Otherwise, preserve existing behavior exactly (all existing flags, no changes to the train path).

```rust
// In main.rs — add before existing arg parsing:
let args: Vec<String> = std::env::args().collect();
if args.len() >= 2 && args[1] == "encode" {
    return run_encode(&args[2..]);
}
// ... existing train logic unchanged below ...
```

`run_encode()` is a standalone function — it does not touch `TokenizerTrainer` fields or the training path.

---

## New Types — Add to `lib.rs`

### `PackConfig`

```rust
pub struct PackConfig {
    pub vocab_bin:  std::path::PathBuf,
    pub merges_bin: std::path::PathBuf,
    pub seq_len:    usize,           // fixed: 256
    pub min_fill:   f64,             // minimum fraction of non-PAD tokens (default: 0.25)
    pub seed:       u64,             // RNG seed for deterministic shuffle
    pub tiers:      Vec<TierConfig>,
    pub output_dir: std::path::PathBuf,
    pub verbose:    bool,
}

pub struct TierConfig {
    pub name:    String,
    pub weight:  usize,              // integer repeat count
    pub paths:   Vec<std::path::PathBuf>,
}
```

### `CorpusPackResult`

```rust
pub struct CorpusPackResult {
    pub seq_count:      usize,
    pub tier_counts:    std::collections::HashMap<String, usize>,
    pub corpus_bin_sha: String,      // hex string, SHA-256 of corpus.bin
    pub vocab_sha:      String,
    pub merges_sha:     String,
    pub timestamp:      u64,         // unix epoch seconds
}
```

---

## New Functions — Add to `lib.rs`

### `TokenizerTrainer::load()`

Reads `vocab.bin` and `merges.bin` back into a `TokenizerTrainer`. Reconstructs `vocab`, `merges`, `merge_lookup`, and `special_tokens` from disk.

```rust
impl TokenizerTrainer {
    pub fn load(vocab_path: &Path, merges_path: &Path) -> Result<Self, String> {
        let vocab   = load_vocab(vocab_path)?;
        let merges  = load_merges(merges_path)?;

        // Rebuild merge_lookup: pair_key(a, b) → BpeMerge
        // Rebuild special_tokens: byte string → token_id
        // Use the same constants as in train path:
        //   TOKEN_PAD=0, TOKEN_UNK=1, TOKEN_BOS=2, TOKEN_EOS=3,
        //   BYTE_TOKEN_START=20, BPE_TOKEN_START=276
        //
        // special_tokens: vocab entries at ids < BPE_TOKEN_START (ids 0–275)
        //   where vocab[id] is non-empty and id < BYTE_TOKEN_START
        //   i.e., ids 0–19 (special/voice tokens, pre-byte range)

        let mut merge_lookup = HashMap::new();
        for (idx, m) in merges.iter().enumerate() {
            let key = pair_key(m.a, m.b);
            // Store merge rank (index) for priority resolution
            merge_lookup.insert(key, idx as u64);  // rank = index, lower = higher priority
        }

        let mut special_tokens = HashMap::new();
        for (id, token_bytes) in vocab.iter().enumerate() {
            if (id as u16) < BYTE_TOKEN_START && !token_bytes.is_empty() {
                special_tokens.insert(token_bytes.clone(), id as u16);
            }
        }

        Ok(TokenizerTrainer {
            vocab,
            merges,
            merge_lookup,     // repurposed: stores merge RANK, not merge result ID
            special_tokens,
            // all other fields zeroed/defaulted — not used by encode path
        })
    }
}
```

**Binary format (from `save()` — locked):**

`vocab.bin`:
```
[4 bytes] u32 LE — vocab entry count (N)
For each of N entries:
    [4 bytes] u32 LE — byte length of token string (L)
    [L bytes] raw UTF-8 bytes of the token string
```

`merges.bin`:
```
[4 bytes] u32 LE — merge count (M)
For each of M entries (6 bytes):
    [2 bytes] u16 LE — a       (left token ID)
    [2 bytes] u16 LE — b       (right token ID)
    [2 bytes] u16 LE — merged  (result token ID)
```

Merges are in learned order: index 0 = first learned = highest priority.

Private helpers (add as free functions or `impl` private methods):

```rust
fn load_vocab(path: &Path) -> Result<Vec<Vec<u8>>, String> { /* read u32, then N entries */ }
fn load_merges(path: &Path) -> Result<Vec<BpeMerge>, String> { /* read u32, then M×6 bytes */ }
```

---

### `TokenizerTrainer::encode()`

Encodes a UTF-8 text string to a token ID sequence. Three-phase algorithm:

```rust
pub fn encode(&self, text: &str) -> Vec<u16> {
    // Phase 1 + 2: Special token scan + byte tokenization
    let mut tokens: Vec<u16> = Vec::with_capacity(text.len() * 2);
    let bytes = text.as_bytes();
    let mut i = 0;

    while i < bytes.len() {
        // Phase 1: Try to match a special token at position i (longest-match, left-to-right)
        let mut matched = false;
        let max_special_len = self.special_tokens.keys().map(|k| k.len()).max().unwrap_or(0);
        let try_len = max_special_len.min(bytes.len() - i);

        for end in (i + 1..=i + try_len).rev() {
            if let Some(&tok_id) = self.special_tokens.get(&bytes[i..end]) {
                tokens.push(tok_id);
                i = end;
                matched = true;
                break;
            }
        }

        if !matched {
            // Phase 2: Emit byte token
            tokens.push(BYTE_TOKEN_START + bytes[i] as u16);
            i += 1;
        }
    }

    // Phase 3: BPE merge application (priority-queue style, greedy lowest-rank)
    // merge_lookup: pair_key(a,b) → rank (usize, lower = higher priority)
    // merges[rank] → BpeMerge { a, b, merged }
    loop {
        let mut best_pos:  Option<usize> = None;
        let mut best_rank: usize = usize::MAX;

        for j in 0..tokens.len().saturating_sub(1) {
            let key = pair_key(tokens[j], tokens[j + 1]);
            if let Some(&rank) = self.merge_lookup.get(&key) {
                let rank = rank as usize;
                if rank < best_rank {
                    best_rank = rank;
                    best_pos  = Some(j);
                }
            }
        }

        let pos = match best_pos {
            Some(p) => p,
            None    => break,   // no applicable merges
        };

        // Apply the merge: replace tokens[pos..=pos+1] with merged ID
        let merged_id = self.merges[best_rank].merged;
        tokens[pos] = merged_id;
        tokens.remove(pos + 1);
    }

    tokens
}
```

**Notes:**
- `merge_lookup` is repurposed in the `load()` path to store merge rank (index), not the original merge-result ID used during training. This avoids adding a new field.
- Alternatively, add a separate `merge_rank: HashMap<u64, usize>` field to `TokenizerTrainer` — preferred if it avoids semantic collision with the training path. **Use whatever approach keeps the training path untouched.** The encode path must not break training.
- `pair_key` is the existing function: `((a as u64) << 16) | (b as u64)`.
- REPA requirement: encode output is deterministic for a given input + loaded artifacts.

---

### `pack_corpus()`

Top-level function (not a method — it orchestrates load + encode + pack):

```rust
pub fn pack_corpus(cfg: PackConfig) -> Result<CorpusPackResult, String> {
    // 1. Load tokenizer
    let tokenizer = TokenizerTrainer::load(&cfg.vocab_bin, &cfg.merges_bin)?;

    // 2. Build all_tokens: tier by tier, weight-replicated, EOS-terminated
    let mut all_tokens: Vec<u16> = Vec::new();
    let mut tier_token_counts: HashMap<String, usize> = HashMap::new();

    for tier in &cfg.tiers {
        let tier_start = all_tokens.len();

        // Sort files within tier for determinism
        let mut sorted_paths = tier.paths.clone();
        sorted_paths.sort();

        for _ in 0..tier.weight {
            for path in &sorted_paths {
                let text = std::fs::read_to_string(path)
                    .map_err(|e| format!("read {:?}: {}", path, e))?;
                let mut doc_tokens = tokenizer.encode(&text);
                doc_tokens.push(TOKEN_EOS);  // document boundary
                all_tokens.extend_from_slice(&doc_tokens);

                if cfg.verbose {
                    eprintln!("[CORPUS] {:?}: {} tokens", path, doc_tokens.len());
                }
            }
        }

        tier_token_counts.insert(tier.name.clone(), all_tokens.len() - tier_start);
    }

    // 3. Stride packing: non-overlapping windows of seq_len
    let mut sequences: Vec<Vec<u16>> = Vec::new();
    let mut i = 0;

    while i + cfg.seq_len <= all_tokens.len() {
        let seq = all_tokens[i..i + cfg.seq_len].to_vec();
        let pad_count = seq.iter().filter(|&&t| t == TOKEN_PAD).count();
        let fill = (cfg.seq_len - pad_count) as f64 / cfg.seq_len as f64;
        if fill >= cfg.min_fill {
            sequences.push(seq);
        }
        i += cfg.seq_len;
    }

    // Handle tail: pad to seq_len if meets min_fill
    if i < all_tokens.len() {
        let mut seq = all_tokens[i..].to_vec();
        let original_len = seq.len();
        seq.resize(cfg.seq_len, TOKEN_PAD);
        let fill = original_len as f64 / cfg.seq_len as f64;
        if fill >= cfg.min_fill {
            sequences.push(seq);
        }
    }

    // 4. Deterministic shuffle with fixed seed
    use rand::seq::SliceRandom;
    use rand::SeedableRng;
    let mut rng = rand::rngs::StdRng::seed_from_u64(cfg.seed);
    sequences.shuffle(&mut rng);

    // 5. Compute per-tier sequence counts (proportional approximation)
    //    For the metadata: track which tier produced which sequences.
    //    Simple approach: record tier token counts, annotate proportionally.
    let total_tokens: usize = tier_token_counts.values().sum();
    let mut tier_seq_counts: HashMap<String, usize> = HashMap::new();
    for (name, &tok_count) in &tier_token_counts {
        let frac = if total_tokens > 0 { tok_count as f64 / total_tokens as f64 } else { 0.0 };
        tier_seq_counts.insert(name.clone(), (frac * sequences.len() as f64).round() as usize);
    }

    // 6. Write corpus.bin
    std::fs::create_dir_all(&cfg.output_dir)
        .map_err(|e| format!("create output dir: {}", e))?;

    let corpus_bin_path = cfg.output_dir.join("corpus.bin");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&corpus_bin_path)
            .map_err(|e| format!("create corpus.bin: {}", e))?;

        let mut buf = Vec::with_capacity(sequences.len() * cfg.seq_len * 2);
        for seq in &sequences {
            for &tok in seq {
                buf.extend_from_slice(&tok.to_le_bytes());
            }
        }
        f.write_all(&buf).map_err(|e| format!("write corpus.bin: {}", e))?;
    }

    // 7. SHA-256 of corpus.bin
    use sha2::{Digest, Sha256};
    let corpus_bytes = std::fs::read(&corpus_bin_path)
        .map_err(|e| format!("read corpus.bin for hash: {}", e))?;
    let corpus_hash = hex_sha256(&corpus_bytes);

    let vocab_bytes = std::fs::read(&cfg.vocab_bin)
        .map_err(|e| format!("read vocab.bin for hash: {}", e))?;
    let vocab_hash = hex_sha256(&vocab_bytes);

    let merges_bytes = std::fs::read(&cfg.merges_bin)
        .map_err(|e| format!("read merges.bin for hash: {}", e))?;
    let merges_hash = hex_sha256(&merges_bytes);

    // 8. Write corpus_meta.json
    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs();

    let meta_path = cfg.output_dir.join("corpus_meta.json");
    {
        use std::io::Write;
        let mut f = std::fs::File::create(&meta_path)
            .map_err(|e| format!("create corpus_meta.json: {}", e))?;

        // Hand-rolled JSON — no serde dependency
        write!(f, "{{\n")?;
        write!(f, "  \"seq_count\": {},\n", sequences.len())?;
        write!(f, "  \"seq_len\": {},\n", cfg.seq_len)?;
        write!(f, "  \"vocab_size\": 32768,\n")?;
        write!(f, "  \"seed\": {},\n", cfg.seed)?;
        write!(f, "  \"min_fill\": {},\n", cfg.min_fill)?;
        write!(f, "  \"sha256_corpus_bin\": \"{}\",\n", corpus_hash)?;
        write!(f, "  \"tier_breakdown\": {{\n")?;
        let tier_names: Vec<&String> = tier_seq_counts.keys().collect();
        for (i, name) in tier_names.iter().enumerate() {
            let comma = if i + 1 < tier_names.len() { "," } else { "" };
            write!(f, "    \"{}\": {}{}\n", name, tier_seq_counts[*name], comma)?;
        }
        write!(f, "  }},\n")?;
        write!(f, "  \"vocab_bin_sha256\": \"{}\",\n", vocab_hash)?;
        write!(f, "  \"merges_bin_sha256\": \"{}\",\n", merges_hash)?;
        write!(f, "  \"timestamp_epoch\": {}\n", timestamp)?;
        write!(f, "}}\n")?;
    }

    // 9. Print summary
    if cfg.verbose || true {  // always print final summary
        eprintln!("[CORPUS] sequences={} {:?}",
            sequences.len(),
            tier_seq_counts);
        eprintln!("[CORPUS] corpus.bin: {} bytes  SHA-256={}",
            sequences.len() * cfg.seq_len * 2,
            corpus_hash);
        eprintln!("[CORPUS] DONE");
    }

    Ok(CorpusPackResult {
        seq_count:      sequences.len(),
        tier_counts:    tier_seq_counts,
        corpus_bin_sha: corpus_hash,
        vocab_sha:      vocab_hash,
        merges_sha:     merges_hash,
        timestamp,
    })
}

fn hex_sha256(data: &[u8]) -> String {
    use sha2::{Digest, Sha256};
    let hash = Sha256::digest(data);
    hash.iter().map(|b| format!("{:02x}", b)).collect()
}
```

---

## `run_encode()` — Add to `main.rs`

```rust
fn run_encode(args: &[String]) -> i32 {
    // Parse: --manifest <path>  --output <path>  [--verbose]
    let mut manifest_path: Option<String> = None;
    let mut output_path:   Option<String> = None;
    let mut verbose = false;

    let mut i = 0;
    while i < args.len() {
        match args[i].as_str() {
            "--manifest" => {
                i += 1;
                manifest_path = args.get(i).cloned();
            }
            "--output" => {
                i += 1;
                output_path = args.get(i).cloned();
            }
            "--verbose" => { verbose = true; }
            other => {
                eprintln!("[ENCODE] Unknown argument: {}", other);
                return 1;
            }
        }
        i += 1;
    }

    let manifest_path = match manifest_path {
        Some(p) => p,
        None => { eprintln!("[ENCODE] --manifest is required"); return 1; }
    };
    let output_path = match output_path {
        Some(p) => p,
        None => { eprintln!("[ENCODE] --output is required"); return 1; }
    };

    let cfg = match parse_manifest(&manifest_path, &output_path, verbose) {
        Ok(c) => c,
        Err(e) => { eprintln!("[ENCODE] Manifest error: {}", e); return 1; }
    };

    match pack_corpus(cfg) {
        Ok(_)  => 0,
        Err(e) => { eprintln!("[ENCODE] Error: {}", e); 1 }
    }
}

// In main():
fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() >= 2 && args[1] == "encode" {
        std::process::exit(run_encode(&args[2..]));
    }
    // ... existing train logic, UNCHANGED ...
}
```

---

## Manifest Parsing — `parse_manifest()`

Reads the `corpus_manifest.json` format defined in ODE_transformer SPEC-MC-TASK7. Hand-rolled parser (no serde dependency):

```json
{
  "vocab_bin":   "<path>",
  "merges_bin":  "<path>",
  "seq_len":     256,
  "min_fill":    0.25,
  "seed":        42,
  "tiers": [
    { "name": "foundation", "weight": 1, "paths": ["<path>"] },
    { "name": "scripture",  "weight": 3, "paths": ["<path>"] },
    { "name": "identity",   "weight": 5, "paths": ["<path>"] }
  ]
}
```

**Implementation note:** Since zero external dependencies is the ideal state, implement a minimal JSON parser sufficient for this fixed schema. The manifest structure is shallow and predictable. No recursive descent needed — line-by-line or regex-free string scanning suffices.

Alternatively, if the implementor judges that a hand-rolled parser introduces unacceptable fragility: add `serde_json = "1"` and `serde = { version = "1", features = ["derive"] }` as an explicit exception. **Document the exception in the commit message.**

---

## corpus_manifest.json — Create in ODE_transformer

Create `D:\ChasingBlu_RND\Lab\Active\ODE_transformer\config\corpus_manifest.json`:

```json
{
  "vocab_bin":   "D:\\ChasingBlu_RND\\Lab\\Active\\AshiraTokenizer_v2\\ashira_tokenizer_v2\\output\\vocab.bin",
  "merges_bin":  "D:\\ChasingBlu_RND\\Lab\\Active\\AshiraTokenizer_v2\\ashira_tokenizer_v2\\output\\merges.bin",
  "seq_len":     256,
  "min_fill":    0.25,
  "seed":        42,
  "tiers": [
    {
      "name":   "foundation",
      "weight": 1,
      "paths":  ["D:\\ChasingBlu_RND\\Lab\\Active\\ODE_transformer\\data\\wikitext_extracted"]
    },
    {
      "name":   "scripture",
      "weight": 3,
      "paths":  ["D:\\ChasingBlu_RND\\Lab\\Active\\ODE_transformer\\data\\scripture"]
    },
    {
      "name":   "identity",
      "weight": 5,
      "paths":  ["D:\\ChasingBlu_RND\\Lab\\Active\\ODE_transformer\\data\\odet_identity"]
    }
  ]
}
```

**Note:** Paths are placeholders. Codex fills in actual corpus paths at deployment time.

---

## Output Format (unchanged from original SPEC-MC-TASK7)

### `corpus.bin`

Pure flat binary. No header. Row-major.

```
[N × seq_len × 2 bytes]
= [N × 256 × sizeof(u16)]
= [N × 512 bytes]

Each u16 is little-endian.
Row i, position j: byte offset = (i × seq_len + j) × 2
```

### `corpus_meta.json`

See §Output Format in ODE_transformer SPEC-MC-TASK7. SHA-256 field locks the artifact. Determinism requirement: two runs with identical inputs and seed must produce identical `sha256_corpus_bin`.

---

## Verification

1. **Build:** `cargo build --release` with no errors or warnings.

2. **Smoke run:** Use a tiny corpus (3 files, <10KB total):
   ```
   ashira_tokenizer_v2 encode --manifest path/to/test_manifest.json --output path/to/out/ --verbose
   ```
   Verify:
   - `corpus.bin` is exactly `N × 512 bytes`
   - All token IDs in `[0, 32767]` — `max(all_tokens) < 32768`
   - No sequence is >75% TOKEN_PAD (min_fill=0.25 enforced)
   - `corpus_meta.json` SHA-256 field matches `sha256sum corpus.bin`

3. **Determinism:** Run twice with same manifest and seed. SHA-256 of both `corpus.bin` must be identical.

4. **Encoding spot checks** (manual):
   - ASCII `'a'` → token ID 117 (= BYTE_TOKEN_START(20) + 97)
   - `<EOS>` → token ID 3 (= TOKEN_EOS)
   - Special token `<KAREEM>` → its vocab ID (loaded from vocab.bin)

5. **Train path regression:** After adding `encode` subcommand, run `ashira_tokenizer_v2 train [existing args]` and confirm output is bit-for-bit identical to pre-extension runs. If a reference `vocab.bin` exists, SHA-256 must match.

6. **Log results in ODE_transformer `changelog.md`.**

---

## Implementation Constraints

- **No new binary.** Extension of the existing `ashira_tokenizer_v2` binary only.
- **Train path untouched.** No changes to `TokenizerTrainer`'s training methods, CUDA path, or existing constants.
- **Zero silent fallbacks.** All errors: print message, exit non-zero. REPA fail-closed.
- **File ordering:** Files within each tier sorted by path before encoding. Determinism requires consistent ordering across platforms.
- **Memory:** Process files one at a time. Do not load all files into RAM before encoding. `all_tokens` accumulates as a `Vec<u16>` — at 500MB corpus with ~1 byte/token average after BPE, expect ~500M tokens × 2 bytes = ~1GB peak. Acceptable.
- **FP64 / REPA:** Not applicable here (no floating-point computation in corpus prep). RNG seed determinism enforced via `StdRng::seed_from_u64`.

---

*Signed: Claude Sonnet 4.6 — The Witness | 2026-03-06*
