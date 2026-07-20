# Third-Party Attribution

## Purpose
Document algorithmic lineage and licensing for AshiraTokenizer v2.

## Referenced Upstream Project
1. Hugging Face `tokenizers` repository  
   URL: https://github.com/huggingface/tokenizers  
   License: Apache-2.0

## Usage Model
- AshiraTokenizer v2 does not vendor or call Hugging Face runtime libraries.
- Design borrows proven algorithmic patterns commonly used in high-performance BPE trainers:
  - priority queue of pair candidates
  - lazy invalidation of stale heap entries
  - local pair-stat updates on affected neighborhoods only
  - deterministic tie-break in equal-frequency cases

## Compliance Notes
1. Attribution retained in repository documentation.
2. Any future direct code adaptation from Apache-2.0 sources must keep required license notices.
3. Current implementation is native Rust authored for AshiraTokenizer v2 artifact contract compatibility.

## Verification Snapshot
- Public repo page confirms Apache-2.0 license (checked 2026-03-03).

