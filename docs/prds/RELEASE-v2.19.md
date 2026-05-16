# Release v2.19.0 — Pinocchio probe, Miri verify, scaffold-to-spec interview

v2.19 lands two complementary additions to QEDGen's brownfield audit
surface:

1. **Pinocchio runtime probe** (`qedgen probe --program <root>`) that
   enumerates `unsafe`-serde and arithmetic sites in Pinocchio
   programs, parses adjacent `// SAFETY:` comments, and maps each site
   to a candidate finding with both Mollusk and Miri repro prompts.
   The auditor subagent expands the prompts into runnable tests.
2. **Miri verify backend** (`qedgen verify --miri`) that runs Pinocchio
   repros under `cargo +nightly miri test` and surfaces UB / aliasing /
   overflow plus dual-execution divergence against Mollusk as Critical.
3. **Scaffold-to-spec interview** — the brownfield value-delivery lever
   that ties them together. One CLI dance: scan a Solana program,
   cluster findings into ≤10 candidate spec clauses, ratify via
   markdown interview, emit `<program>.qedspec` + rejected-clause
   scoping notes + bug-flagged finding files. Covers all four runtimes
   (Pinocchio, Anchor, Quasar, Native — Native ships as preview).

## What's in

### Pinocchio probe surface (M1 of the Pinocchio audit work)

`crates/qedgen/src/pinocchio_probe.rs` walks every `*.rs` under the
project's `src/` and enumerates 10 site kinds:

- `BorrowUnchecked` — `*.borrow_*_unchecked*()`
- `BytemuckCall` — `bytemuck::(from|try_from|cast)*<T>`
- `RawPtrCastFromAccount` — raw `as *const _` / `transmute` on account data
- `CustomLoadCall` — `load*` fn inside `unsafe { }` with `_unchecked` first arg
- `TryIntoUnwrapOnSlice` — `_[..].try_into().unwrap()`
- `SetLamportsArith` — `set_lamports(...)` / `*lamports {+/-}= _`
- `SetAmountArith` — `set_amount(amount() {+/-} _)`
- `IndexedAccountAccess` — `accounts[N]` literal
- `IndexedDataSlice` — `data[CONST..CONST{+/-}N]`
- `SafetyComment` — `// SAFETY:` blocks attached to the next `unsafe { }` scope

Each site maps to a `Finding` with two reproducer prompts
(`MolluskPrompt` + `MiriPrompt`). The MiriPrompt carries
`adversarial_inputs` derived from the SAFETY comment's parsed clauses
(`uninit_init_flag`, `foreign_owner`, `swap_position`, etc.) plus
`invariant_asserts` the agent brackets the handler call with.

`Reproducer::MolluskPrompt` and `Reproducer::MiriPrompt` are new
v2.19 variants on the `Reproducer` enum. Both carry
`template_path`, `substitutions: BTreeMap<String, String>`, and
`repro_path` — the agent reads the template, expands substitutions,
and writes the filled repro to disk.

Three fixture programs under `examples/pinocchio-fixtures/`:
`pata-create`, `ptoken-close-account`, `ptoken-transfer`. Each
mirrors a real Pinocchio shape (synthesized, not vendored — see
each fixture's `NOTICE.md`).

### Miri verify backend (M2 of the Pinocchio audit work)

`crates/qedgen/src/miri_verify.rs` discovers
`.qed/probes/pinocchio/*/repro_miri.rs` under the project root,
shells `cargo +nightly miri test`, and parses output for UB /
aliasing / overflow / `SAFETY claim STALE` markers into structured
`MiriDiagnostic`s. Each diagnostic surfaces as a finding through the
existing `verify` envelope.

**Dual-execution divergence detection.** When a Pinocchio repro fails
under Miri but the corresponding Mollusk repro passes, that's a
release-mode wrap + sBPF-alignment masking signal. Surfaced as
`Category::ExecutionDivergence` (Critical) — the deployed `.so` runs
under-detected UB that the host Miri interpreter exposes.

CLI: `qedgen verify --miri` (alone or alongside `--proptest` /
`--kani` / `--lean`). With no backend flags, the auto-detect path now
picks up `--miri` if `.qed/probes/pinocchio/*/repro_miri.rs` files
exist on disk.

`qedgen probe --program <root>` runtime-detects Pinocchio
automatically (presence of `pinocchio` Cargo dep). Override with
`--runtime pinocchio` if detection fails.

### Scaffold-to-spec interview (M1-M4 of the interview work)

The brownfield bear-hug. One command per phase:

```bash
# Phase 1: scan + cluster + write the interview prompts
qedgen probe --program <root> --emit-spec-candidates \
  --audit-dir .qed/audit/<timestamp>

# Phase 2: user edits .qed/audit/<timestamp>/interview.md
# Each cluster has 4 markdown checkboxes: accept / narrow / reject / bug

# Phase 3: ratify into a .qedspec + side-files
qedgen ratify --audit-dir .qed/audit/<timestamp> \
  --out <program>.qedspec
```

What the audit dir contains after Phase 1:
- `interview.md` — markdown prompts, one section per cluster,
  organized by confidence (High → Medium → Low)
- `clusters.json` — schema-v3 envelope: full cluster metadata,
  finding back-references, suggested syntax, write-routing
- `skeleton.qedspec` — pre-interview structural skeleton (handler
  stubs only; semantic clauses come from the interview)

What ratify writes:
- `<program>.qedspec` — skeleton + ratified clauses merged in
  (program-scope invariants emitted as
  `invariant N "description"` — description-form, parser-valid;
  handler-scope clauses as structured `// TODO ratified (...)` markers
  with `// Target form: ...` lines pointing at the parseable shape)
- `.qed/plan/scoping.md` — rejected clusters with user rationale
- `.qed/findings/scaffold-to-spec-<cluster_id>.md` — one file per
  bug-flagged cluster (the user identified the implicit precondition
  as a real missing-enforcement bug, not a spec gap)

**14 cluster kinds** (universal across runtimes — `account_owner_check`,
`account_init_check`, `account_signer_check`, `account_type_tag_check`,
`account_distinct`, `arithmetic_no_overflow`, `arithmetic_bound_pre`,
`pda_canonical_derivation`, `pda_seed_uniqueness`,
`lifecycle_one_shot`, `lifecycle_monotonic`, `cpi_program_pin`,
`cpi_account_direction`, `dispatch_caller_establishes_callee_requires`).
Per-runtime extractors map detected site shapes to cluster kinds;
clustering algorithm and prompts/spec emission are runtime-agnostic.

**Promotion threshold:** when ≥3 handlers contribute proto-clauses of
the same kind, the cluster is promoted from Handler-scope to
Program-scope (consolidates the question, surfaces as a candidate
program-wide invariant). Stable, deterministic cluster IDs across re-runs.

**Property tests** for every cluster kind × scope combination: the
emitted spec is parser-validated through `chumsky_adapter::parse_str`.
Ratification is byte-idempotent — re-running on the same audit dir
produces the same spec.

#### Runtime coverage

| Runtime | Extractor | Skeleton | Patterns covered |
|---|---|---|---|
| Pinocchio | `pinocchio_extractor.rs` | `pinocchio_to_spec.rs` (`pub fn process_*` walk) | SAFETY-comment classification, `_unchecked` loads, `set_amount`/`set_lamports` arith, bytemuck/raw-cast type confusion |
| Anchor | `anchor_extractor.rs` | `anchor_adapt::adapt` (IDL-driven) | `AccountInfo`/`UncheckedAccount` escape hatches, `#[account(seeds=…)]` without `bump`, raw `+-*/` in handler bodies (skipping `#[account(space=…)]` macro context), `init_if_needed` |
| Quasar | Routes through `anchor_extractor` | `anchor_adapt::adapt` | Inherited from Anchor; Quasar-specific drift detection deferred to future |
| **Native (preview)** | `native_extractor.rs` | `pinocchio_to_spec::render_skeleton_native` (any `pub fn`) | `invoke_signed` without nearby program-ID pin, `Pubkey::create_program_address`, raw arith, `**try_borrow_mut_lamports()? OP x` lamport demotion |

**Native is marked preview.** Coverage is narrower than Anchor's
because Native has no framework conventions — every check is the
author's responsibility, and syntactic detection is conservatively
false-negative-biased to keep FP rate manageable. Manual signer-check
absence, owner-check absence, and discriminator collision remain
covered at the auditor SKILL.md layer (Read+Grep on the impl, not in
the CLI extractor).

#### Schema v3 — backwards-compatible envelope extension

The probe envelope's new `clusters[]` field appears only when
`--emit-spec-candidates` is passed. v2-shape consumers without that
flag see the unchanged envelope. Cluster schema documented in the
SCOPING-v2.19-scaffold-to-spec.md design doc.

## Migration

No breaking changes — every addition is additive behind opt-in flags.

- Existing probe runs without `--emit-spec-candidates` are unchanged.
- Existing `qedgen verify` runs without `--miri` skip the Miri backend.
- The auditor SKILL.md (`skills/qedgen-auditor/SKILL.md` +
  `.claude/skills/qedgen-auditor/SKILL.md`) has a new step 6 that
  documents the interview flow; the legacy silent-scaffold path still
  applies for sBPF / exotic runtimes the extractor doesn't cover.

## Pre-release validation

- `cargo fmt --check` ✓
- `cargo clippy -- -D warnings` ✓
- `cargo test --release` — 643 passes, 0 failures, 8 ignored.
- `bash scripts/check-readme-drift.sh` ✓
- Bundled examples (`examples/rust/escrow/qed.lock` etc.) — checked
  against current spec via `qedgen check --frozen`.
- End-to-end dogfood: pre-fix `p-token` (`cf136e7^`) — 99 findings
  cluster into 6 candidate clauses → user bug-flags the
  asymmetric-disclosed `owner_locked_writes` cluster → finding file
  captures the cf136e7 disclosure with user rationale + 20
  cross-referenced finding IDs.

## What's not in v2.19

The original SCOPING doc called for a full AST-to-qedspec serializer
in M2; we punted it during execution after discovering the M1 text-based
ratify is byte-idempotent and produces parser-clean output. Defers to
the first release where Anchor/Native extractors need to mutate
existing user-written specs (vs. appending to skeletons) — currently
no such use case has surfaced.

Native extractor's manual-signer-check and manual-owner-check
detection (the patterns where syntactic detection requires data-flow
analysis) remain at the auditor SKILL.md / Read+Grep layer. Promotes
to extractor-coverage when we have a real-world signal that the
agent-layer is missing them.
