#[cfg(any())]
use ashira_tokenizer_v3::{
    TokenizerTrainer, TrainConfig, VOCAB_SIZE, scan_training_files, validate_vocab_target,
};
#[cfg(any())]
use std::env;
#[cfg(any())]
use std::fs::{self, OpenOptions};
#[cfg(any())]
use std::io::Write;
#[cfg(any())]
use std::path::PathBuf;
#[cfg(any())]
use std::time::{SystemTime, UNIX_EPOCH};

#[cfg(any())]
fn now_utc_epoch() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

#[cfg(any())]
fn print_usage(prog: &str) {
    println!("Usage: {prog} [options]");
    println!("  --corpus <dir>          Path to Corpus_main directory (required)");
    println!("  --output <dir>          Output directory (default: .)");
    println!("  --vocab-size <N>        Target vocabulary size (default: {VOCAB_SIZE})");
    println!("  --min-freq <N>          Minimum weighted pair frequency (default: 2)");
    println!("  --accelerator <x>       cpu | cuda (default: cpu)");
    println!("  --allow-cpu-fallback    Explicitly authorize CPU fallback when cuda is requested");
    println!("  --vram-budget <F>       Compatibility arg (accepted, not used in v2 CPU core)");
    println!("  --help                  Show this help");
}

#[cfg(any())]
fn inherited_pattern_authority_cli_retained_for_audit() {
    let args: Vec<String> = env::args().collect();
    let prog = args
        .first()
        .cloned()
        .unwrap_or_else(|| "ashira_train".to_string());

    let mut corpus = String::new();
    let mut output = ".".to_string();
    let mut vocab_size = VOCAB_SIZE;
    let mut min_freq: u32 = 2;
    let mut accelerator = "cpu".to_string();
    let mut vram_budget: f64 = 0.90;
    let mut allow_cpu_fallback = false;

    let mut i = 1usize;
    while i < args.len() {
        match args[i].as_str() {
            "--help" | "-h" => {
                print_usage(&prog);
                return;
            }
            "--corpus" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --corpus requires value");
                    std::process::exit(2);
                }
                corpus = args[i].clone();
            }
            "--output" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --output requires value");
                    std::process::exit(2);
                }
                output = args[i].clone();
            }
            "--vocab-size" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --vocab-size requires value");
                    std::process::exit(2);
                }
                vocab_size = match args[i].parse::<usize>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("ERROR: invalid --vocab-size");
                        std::process::exit(2);
                    }
                };
            }
            "--min-freq" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --min-freq requires value");
                    std::process::exit(2);
                }
                min_freq = match args[i].parse::<u32>() {
                    Ok(v) => v,
                    Err(_) => {
                        eprintln!("ERROR: invalid --min-freq");
                        std::process::exit(2);
                    }
                };
            }
            "--accelerator" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --accelerator requires value");
                    std::process::exit(2);
                }
                accelerator = args[i].clone();
            }
            "--allow-cpu-fallback" => {
                allow_cpu_fallback = true;
            }
            "--vram-budget" => {
                i += 1;
                if i >= args.len() {
                    eprintln!("ERROR: --vram-budget requires value");
                    std::process::exit(2);
                }
                vram_budget = match args[i].parse::<f64>() {
                    Ok(v) if (0.5..=0.95).contains(&v) => v,
                    _ => {
                        eprintln!("ERROR: --vram-budget must be in [0.5, 0.95]");
                        std::process::exit(2);
                    }
                };
            }
            other => {
                eprintln!("ERROR: unknown arg: {other}");
                print_usage(&prog);
                std::process::exit(2);
            }
        }
        i += 1;
    }

    if let Err(error) = validate_vocab_target(vocab_size) {
        eprintln!("ERROR: invalid --vocab-size: {error}");
        std::process::exit(2);
    }
    if corpus.is_empty() {
        eprintln!("ERROR: --corpus is required");
        print_usage(&prog);
        std::process::exit(2);
    }
    if accelerator != "cpu" && accelerator != "cuda" {
        eprintln!("ERROR: --accelerator must be 'cpu' or 'cuda'");
        std::process::exit(2);
    }
    if accelerator == "cuda" && !allow_cpu_fallback {
        eprintln!("[FAIL-CLOSED] CUDA requested but v2 GPU kernel path is not enabled yet.");
        eprintln!("[FAIL-CLOSED] Re-run with --allow-cpu-fallback to authorize CPU execution.");
        std::process::exit(5);
    }
    if accelerator == "cuda" && allow_cpu_fallback {
        println!("[WARN] CUDA requested; CPU fallback explicitly authorized.");
    }

    let output_dir = PathBuf::from(output.clone());
    if let Err(e) = fs::create_dir_all(&output_dir) {
        eprintln!(
            "ERROR: failed to create output dir {}: {}",
            output_dir.display(),
            e
        );
        std::process::exit(7);
    }

    let log_path = output_dir.join("tokenizer_train.log");
    let mut log = match OpenOptions::new().create(true).append(true).open(&log_path) {
        Ok(f) => f,
        Err(e) => {
            eprintln!("ERROR: cannot open log file {}: {}", log_path.display(), e);
            std::process::exit(7);
        }
    };
    let _ = writeln!(
        log,
        "[{}] [INFO] run started accelerator={} vocab_size={} min_freq={} vram_budget={} allow_cpu_fallback={}",
        now_utc_epoch(),
        accelerator,
        vocab_size,
        min_freq,
        vram_budget,
        allow_cpu_fallback
    );

    println!("=== AshiraTokenizer v3 Trainer (artifact publication pending explicit format) ===");
    println!("REPA deterministic mode: ON");
    println!("Corpus: {corpus}");
    println!("Output: {}", output_dir.display());

    let corpus_path = PathBuf::from(corpus.clone());
    let (files, summary) = match scan_training_files(&corpus_path) {
        Ok(v) => v,
        Err(e) => {
            eprintln!("ERROR: scan failed: {e}");
            let _ = writeln!(log, "[{}] [ERROR] scan failed: {}", now_utc_epoch(), e);
            std::process::exit(4);
        }
    };
    if files.is_empty() {
        eprintln!("ERROR: no training files found");
        let _ = writeln!(log, "[{}] [ERROR] no training files found", now_utc_epoch());
        std::process::exit(4);
    }

    println!(
        "[SCAN] files={} skipped={} bytes={}",
        summary.total_files, summary.skipped_files, summary.total_bytes
    );
    for (tier, count) in summary.tier_counts.iter() {
        println!("[SCAN] tier={tier} files={count}");
    }

    let mut trainer = TokenizerTrainer::new();
    let config = TrainConfig {
        vocab_size,
        min_frequency: min_freq,
        deterministic: true,
    };
    let stats = match trainer.train_weighted(&files, &config) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("ERROR: training failed: {e}");
            let _ = writeln!(log, "[{}] [ERROR] training failed: {}", now_utc_epoch(), e);
            std::process::exit(5);
        }
    };

    let vocab_bin = output_dir.join("vocab.bin");
    let merges_bin = output_dir.join("merges.bin");
    if let Err(e) = trainer.save(&vocab_bin, &merges_bin) {
        eprintln!("ERROR: save failed: {}", e);
        let _ = writeln!(log, "[{}] [ERROR] save failed: {}", now_utc_epoch(), e);
        std::process::exit(6);
    }

    let hash = trainer.compute_hash_hex();
    let config_path = output_dir.join("tokenizer_config.json");
    let config_json = format!(
        "{{\n  \"project\": \"AshiraTokenizer_v3\",\n  \"vocab_size\": {},\n  \"merge_count\": {},\n  \"min_frequency\": {},\n  \"training_files\": {},\n  \"training_bytes\": {},\n  \"loaded_sequences\": {},\n  \"loaded_tokens\": {},\n  \"skipped_lines\": {},\n  \"accelerator\": \"{}\",\n  \"allow_cpu_fallback\": {},\n  \"deterministic\": true,\n  \"hash_fnv1a64\": \"{}\",\n  \"timestamp_epoch\": {}\n}}\n",
        trainer.vocab_size(),
        trainer.merge_count(),
        min_freq,
        stats.input_files,
        summary.total_bytes,
        stats.loaded_sequences,
        stats.loaded_tokens,
        stats.skipped_lines,
        accelerator,
        if allow_cpu_fallback { "true" } else { "false" },
        hash,
        now_utc_epoch()
    );
    if let Err(e) = fs::write(&config_path, config_json.as_bytes()) {
        eprintln!("ERROR: failed writing tokenizer_config.json: {}", e);
        let _ = writeln!(
            log,
            "[{}] [ERROR] config write failed: {}",
            now_utc_epoch(),
            e
        );
        std::process::exit(7);
    }

    println!(
        "[DONE] vocab={} merges={} duration={}s",
        stats.final_vocab, stats.learned_merges, stats.duration_seconds
    );
    println!("[DONE] {}", vocab_bin.display());
    println!("[DONE] {}", merges_bin.display());
    println!("[DONE] {}", config_path.display());
    let _ = writeln!(
        log,
        "[{}] [INFO] complete vocab={} merges={} duration={}s",
        now_utc_epoch(),
        stats.final_vocab,
        stats.learned_merges,
        stats.duration_seconds
    );
}

fn main() {
    let exit_code = {
        let mut stdout = std::io::stdout().lock();
        let mut stderr = std::io::stderr().lock();
        ashira_tokenizer_v3::run_cli(std::env::args_os().collect(), &mut stdout, &mut stderr)
    };
    if exit_code != 0 {
        std::process::exit(exit_code);
    }
}
