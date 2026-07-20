# AshiraTokenizer v3 Build Week Submission Boundary

**Document ID:** `ASHIRA-V3-BUILD-WEEK-BOUNDARY-R1`
**Status:** `CONTROL BOUNDARY — SUBMISSION PACKAGING BLOCKED`
**Contest deadline:** 2026-07-21, operator-verified
**Current authority:** bounded local Build Week demo pipeline plus separately
gated production-v3 work

## 1. Authorized work

- Preserve public-v2 lineage and create the v3 branch.
- Create requirements/design/traceability skeletons.
- Implement and test bounded Stage 2 typed core and artifact contracts.
- Implement and test bounded Stage 3 manifest, pre-segmentation, calibration,
  storage skeleton, and telemetry.
- Commit authorized work inside the v3 source repository.
- Produce the required internal Sol Stage 0-3 handoff package.
- Execute the local public-pipeline demonstration from the deterministic
  WikiText slice using exact manifest label `demo_wikitext_only`.
- Train/freeze/publish, validate, encode/decode, and compare deterministic A/B
  hashes through the same bounded implementation used by larger runs.

## 2. Work not authorized

- 1%/5%/10% probes.
- Production checkpoint generation.
- Full 131,072-entry training.
- External Build Week submission packaging or submission.
- Public demo, quality, scalability, compliance, certification, or readiness
  claims.
- Modification of public/local v2 source or artifacts.
- Deletion of inherited tracked run/build artifacts without separate authority.

The local demo authority does not authorize a balanced-composite claim, final
v3 training-authority claim, closure of BookCorpus/identity/scripture gates, or
use of the demo-only profile through the production composite admission API.

## 3. Current official-material evidence

Contest rules, eligibility, current license requirements, submission fields,
judging criteria, publication terms, and packaging rules have not been verified
from current official OpenAI contest material in this source document.

Status: `BLOCKED_NEEDS_CURRENT_OFFICIAL_PRIMARY_SOURCE_VERIFICATION`.

External WikiText attribution/license evidence remains required before public
submission packaging. The operator-relayed GPT-55 ruling explicitly makes it a
packaging gate rather than a blocker for local demo execution. No
submission-readiness claim may be made from the local demo or the
operator-verified deadline alone.

## 4. License and repository boundary

- Target package metadata is Apache-2.0 under the controlling directive.
- The inherited public commit contains the complete Apache-2.0 license text but
  inherited `Cargo.toml` still says `UNLICENSED`; Stage 1 does not silently
  relabel it.
- Dependency versions/licenses/transitive closure must be recorded before use.
- The inherited public commit tracks `runs/` and `target/`. They remain lineage
  evidence, not automatically approved submission contents.
- Cargo commands must use a separate controlled target directory unless
  tracked-build sanitation is separately authorized.

## 5. Evidence-state table

| Area | Implemented | Tested | Pending | Blocked |
|---|---|---|---|---|
| Stage 0 lineage | yes | yes | no | no |
| Stage 1 skeleton | yes | yes | continuing reconciliation | no |
| Stage 2 core/artifacts | yes, bounded | committed/evidenced plus later local candidates | D4 candidate commit/evidence | cross-platform/durability proof |
| Stage 3 bounded scaffolding | composite admission/pre-segment/codec and evidenced demo pipeline; read-only A/B comparator local | real WikiText A/B 11/11 byte parity; comparator reproduces both aggregate hashes; 91/91 local regression | comparator commit/evidence and video run | full-scale and binary reproducibility proof |
| Dependency/license closure | matrix skeleton | no | versions/licenses | public release |
| Contest-rule verification | no | no | official sources | submission claim |
| Submission package | no | no | separate directive | yes |

## 6. Required gate before external submission work

An explicit GPT-55/operator directive must identify the official contest
sources, approved repository/artifact boundary, license posture, demo scope,
submission fields, evidence package, and authorized external action.

Until then, Stage 0-3 output is internal engineering evidence only.

## 7. Demo-only manifest ruling — 2026-07-19

The operator relayed GPT-55's ruling that the deterministic WikiText slice is
the sole Build Week D4 corpus member. The canonical manifest must use label
`demo_wikitext_only`; it may close only this local path:

```text
deterministic slice -> manifest -> train/freeze/publish -> encode/decode ->
validate -> rerun hash parity
```

The manifest must not represent balanced-composite coverage, final-v3 training
authority, or closure of BookCorpus, identity, or scripture gates. These claim
boundaries remain operative even if the local demo pipeline passes.

The pipeline additionally requires clean committed source authority and an
external run root. Real same-host WikiText runs A/B from commit
`7ccf22bfe79048b38aed594734a69bb31446b7ab` match all 11 governed files. The
local `demo-compare` candidate now verifies two distinct roots read-only,
strictly validates each self-contained run, and emits one PASS summary only
after exact byte and aggregate-hash parity. Comparator commit/evidence remains
a separate acceptance step; the bounded result is not cross-host, rebuild,
full-128K, composite-corpus, licensing, or public-submission proof.
