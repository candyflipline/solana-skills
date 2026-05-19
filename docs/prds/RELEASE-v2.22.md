# Release v2.22.0 — Bench-driven catalog + Pinocchio brownfield Crucible fuzz

v2.22 is the inverse-recall response to the subscriptions-vs-Cantina
bench that ran the day v2.21 shipped. The bench surfaced 4 firm-rated
findings the v2.21 probe catalog missed (2 firm-High, 2 firm-Medium,
plus 2 firm-Low) and one QEDGen-only novel at HEAD that needed a new
category to surface structurally. v2.22 closes all five gaps and lifts
the brownfield Crucible runtime gate to reach the Pinocchio side of the
bench corpus.

Five PRD slices ship; the sixth (`qedgen bench` subcommand) was
intentionally not-shipped — bench orchestration stays a SKILL.md-driven
workflow at `.claude/skills/qedgen-auditor-bench/`, not a CLI verb
([[feedback_bench_stays_a_skill]]).

## What's in

### Slice 3 — Pinocchio brownfield Crucible fuzz (`cd892c5`, `c00904a`, `d0e9aef`, `ee0ff59`)

The v2.22 anchor. v2.21 ship intentionally scoped brownfield
`--fuzz --root` to Anchor / Quasar / qedgen-codegen; the Pinocchio
gate at `crucible_brownfield.rs:48-60` errored out. v2.22 lifts the
gate by requiring a maintainer-authored Codama / Anchor 0.30 IDL on
disk (canonical paths: `idl.json`, `program/idl.json`, `idl/*.json`,
`target/idl/*.json`) — scanner-based metadata inference was
considered and rejected as too noisy
([[feedback_multi_framework_audits]]). The Codama IR shape
(`kind: "rootNode"` with nested `program.instructions[]`) is
recognised alongside the Anchor 0.30 top-level shape.

The bench surfaced eight emit-side bugs that the v2.21 fixture didn't
exercise. Each landed as a separate commit:

1. **Codama IR support** — Crucible's `declare_fuzz_program!` macro
   has explicit Codama support via `crucible-idl-gen/src/codama.rs`;
   no in-tree converter needed.
2. **Explicit `declare_fuzz_program!` module name** — pin via
   `declare_fuzz_program!(name = "idls/name.json")` so the generated
   module matches qedgen's snake_case `use` statements regardless of
   the IDL's internal `program.name` casing.
3. **Program name from IDL** — workspaces with no `[package]` in the
   root `Cargo.toml` fall back to the IDL's `metadata.name` /
   `program.name` instead of the leaf-dir name.
4. **IDL args populate `takes_params`** — Codama
   `program.instructions[].arguments[]` (and Anchor 0.30
   `instructions[].args[]`) are extracted with type mapping
   (`numberTypeNode + format` → `U8/.../I128`, `publicKeyTypeNode` →
   `Pubkey`, etc.). `defaultValueStrategy: "omitted"` args
   (discriminator, etc.) are skipped.
5. **Pubkey args don't `Arbitrary`-derive** — Crucible's
   `#[fuzz_fixture]` can't fuzz `Pubkey`. Pubkey-typed args are
   filtered out of the action signature and inlined as
   `Pubkey::default()` in the call literal — matching Crucible's own
   escrow example idiom (pubkeys come from `.accounts(...)`, not
   `.call(...)`).
6. **`.accounts(...)` turbofish** — type inference on `.accounts(todo!())`
   picks `()` (which doesn't implement `ToAccountMetas`); emit
   `.accounts::<accounts::Foo>(todo!())` to pin the generic.
7. **Escaped braces in `todo!()` message** — `{ ... }` was parsed as
   a format directive; emits `{{ ... }}` so the panic message reads
   `accounts::Foo { ... }`.
8. **`.so` path depth** — brownfield harness lives at
   `<root>/.qed/fuzz/<prog>/` (three levels deep) vs spec-mode's
   `<root>/fuzz/<prog>/` (two). `ctx.add_program` path branches on
   `InvariantMode`.

Then **S3.3 — close `.accounts(...)`** (`ee0ff59`): the agent-fill
placeholder is gone. The harness now emits a real
`accounts::Foo { ... }` literal:

- Fixture-owned `Rc<Keypair>` per non-default, non-PDA account
  (`brownfield_keypair_ident` snake-cases the field name to satisfy
  Rust naming-convention lints).
- `setup()` creates each Keypair and funds with 100 SOL.
- Accounts with `default_pubkey` (Codama `publicKeyValueNode`) are
  SKIPPED from the literal — Crucible auto-fills them.
- PDA accounts (`pda_seeds: Some(...)`) emit
  `Pubkey::find_program_address(&[], &self.program_id).0` placeholder;
  seed-aware derivation is v2.22.x.
- `.signers(&[&*self.x, ...])` is built from `isSigner: true` accounts.

Bench result on `solana-program-escrow-2026-05` (Pinocchio audit):

  qedgen probe --fuzz 30 --no-smoke --root <audit-root>
  → 6,803 executions in 24s (~300 exec/s)
  → 8.6% edge coverage (320/3726 edges)
  → 16% branch coverage (298/1863)
  → findings: []  (no crashes in 30s without spec invariants —
                    expected; fuzzer is exploring program-side
                    rejection paths)

Touched: `crucible_brownfield.rs` (extensive), `crucible_gen.rs`
(brownfield-aware fixture + `.accounts(...)` literal),
`check.rs::ParsedHandlerAccount` (+`default_pubkey: Option<String>`),
`main.rs` (probe `--fuzz --root` dispatch), new fixture
`examples/regressions/v2.22-pinocchio-brownfield-fuzz/buggy_pinocchio/`,
docs `references/cli.md` + `skills/qedgen-auditor/references/probe_orchestration.md`.

**Still deferred to v2.23+** — Native (gates on Shank), sBPF (no
AccountInfo at source level), seed-aware PDA derivation, in-tree
Codama IR → Anchor 0.30 converter (only needed if Crucible drops
Codama support).

### Slice 1 — Arithmetic-symbol catalog (`cde36c5`, `6e9646a`, `7750999`)

Three new source-scanner probe rules, runtime-agnostic. All three
fire on the canonical PRD-cited sites on subscriptions Run A with
zero false positives on the two clean audit corpora (escrow +
rewards).

**`silent_success_arithmetic`** (HIGH) — `saturating_sub` /
`saturating_add` on a timestamp-shape receiver (`current_ts`,
`Clock::get()?.unix_timestamp`, `slot`, `epoch`, `block_height`, or
identifier ending in `_ts` / `_secs` / `_time`) whose result feeds a
`>=`/`>` comparison gating a non-trivial effect. The boundary value
(0 / MAX) silently opens a fund-flow gate. Closes CAN-H1 on the
subscriptions bench — `transfer_validation.rs:61`.

**`graceful_error_as_dos`** (HIGH) — `checked_sub` / `checked_add` /
`checked_mul` in an init / create / initialize handler where the
touched account reaches a PDA (signalled by `find_program_address`,
`invoke_signed`, `seeds:` parameter, or `&[Seed`) and the `Err` arm
propagates via `?` or `return Err`. The arithmetic is correct in
isolation; the bug is the failure-mode interaction with the
address's permanence. Closes CAN-H3 on the subscriptions bench —
`helpers/program.rs:48`.

**`unchecked_arith_with_fund_flow`** (LOW) — bare `<ident> *
<literal>` / `+ <literal>` / `- <literal>` inside a fn whose body
also dispatches a token / system CPI (literal `invoke`, `Transfer`,
`MintTo`, or helper-shape `transfer_with_delegate`, `mint_to_user`,
etc.). Preventive: most call sites are safe today under upstream
bounds, but the local code makes no invariant claim. Closes CAN-I3
on the bench — `transfer_subscription.rs:61`.

Each rule emits a `Reproducer::MolluskPrompt` with a per-rule
markdown template at `references/probes/arithmetic_symbol/<rule>.md`.

### Slice 2 — Paired-validator asymmetry (`c899726`)

**`paired_validator_input_domain_mismatch`** (MEDIUM, two-stage
cross-file scanner):

1. Per-file: match `if <cond> { return Err(...) }` patterns; collect
   every field-like identifier in `<cond>` (suffix match on
   `_ts`/`_at`/`_amount`/`_lamports`/`_balance`/`_id`/`_count`/
   `_status`/`_state`/`_bump`/`_authority`/`_owner`/`_mint`/
   `_program`/`_total`/`_limit`/`_threshold`/`_hours`/`_period`/
   `_length`/`_duration`/`_expiry`/`_start`/`_end`/`_deadline`/
   `_index`).
2. Cross-file: group by field; emit one finding per field with 2+
   distinct normalised shapes (whitespace-stripped, `self.`-stripped,
   `&&`-clauses as a set).

Denylist for time-source idents (`current_ts`, `current_time`, `now`,
`now_ts`, `clock_ts`, `unix_timestamp`, `current_slot`,
`current_epoch`) — those match the `_ts` suffix but are validator
arguments, not state fields.

Bench: 7 findings on subscriptions Run A. Two of them are the
PRD-targeted shapes (`expiry_ts` covers CAN-M1/M2 — 3 distinct
shapes; `end_ts` covers CAN-L2/L3 — 5 distinct shapes). Five
additional plausible signals (`expires_at_ts`, `start_ts`,
`token_mint`, `token_program`, `transfer_amount`) the audit
subagent triages. Zero false positives on the clean audits.

Severity escalation rules per PRD §S2.1: documented sentinel +
asymmetric honor → HIGH; create-vs-update on fund-flow field →
HIGH; otherwise MEDIUM.

### Slice 4 — Lifecycle external-state catalog (`16555c1`)

**`external_authority_not_revoked_on_close`** (MEDIUM, cross-file):

- **Stage A** — walk `*.rs` for authority-conferring CPI
  struct-literal shapes: `Approve { delegate: <X> }` /
  `Approve2022` / `ApproveSpl` / `SetAuthority { new_authority: <X>,
  ... }` / `Assign { new_owner: <X> }` / `Assign { owner: <X> }`.
  Record `(rel_file, line, target_account, operator)`.
- **Stage B** — walk fns whose file name starts `close_` / `revoke_`
  / `terminate_` OR whose own name contains `close` / `revoke` /
  `terminate`. Extract the closed PDA from `<Type>::close(<target>,
  ...)` / `close_account(<target>, ...)`. Check whether body
  contains `Revoke` / `RevokeSpl` / `Revoke2022` / `.revoke(` / etc.
  (suppression signal).
- Pair grants × closes by `closed_account == target_account`; emit
  one finding per match.

Bench: 2 findings on subscriptions HEAD (the QED-HEAD-MED-3
canonical) — `close_subscription_authority.rs:70` (self-funded
branch) + `:78` (sponsor-funded branch), both citing the Approve2022
+ ApproveSpl grant sites in `initialize_subscription_authority.rs`.

**Bonus:** 1 finding on subscriptions Run A (`close_multidelegate.rs:59`)
— the same shape at the audited commit (pre-rename). Cantina missed
this; the rule fires structurally. **Additional QEDGen-only novel
beyond the PRD-named finding.**

Zero false positives on escrow / rewards.

### Slice 5 — Probe envelope noise reduction (`48024d0`, `f0e2eca`)

Drops the subscriptions Pinocchio probe noise floor from 114 → 12
ungated Medium findings (PRD target ≤ 25). Two complementary changes
behind a new `Finding.gated_by: Option<Vec<String>>` field:

**Transmute-import filter.** The existing `transmute_account` regex
matched every `transmute` token — including `use core::mem::transmute;`
imports and transmutes on non-account locals. Adds a `use ` line
guard and the same LHS-shape guard the `raw_ptr_cast` branch uses
(requires `data` / `borrow` / `account` / `input` in the surrounding
expression). First-line impact: 82 → 30 `account_type_confusion`
findings on subscriptions.

**`gated_by` triad detector.** For every Pinocchio site that maps to
a zero-copy / offset-overrun finding (`BytemuckCall`,
`RawPtrCastFromAccount`, `CustomLoadCall`, `IndexedDataSlice`,
`TryIntoUnwrapOnSlice`), walks the ~30 source lines preceding the
site for canonical gate signals:

- `length_check` — `.len()` comparison.
- `discriminator_check` — `AccountDiscriminator` / `discriminator` /
  `DISCRIMINATOR` reference.
- `owner_check` — `ProgramAccount::check` / `.owner()` /
  `&crate::ID` reference.

Empty gate list collapses to `None` so the JSON envelope omits the
field for ungated findings — those are the auditor-focus subset.

Final distribution on subscriptions Run A:
- 24 length_check only (instruction-data zero-copy parses)
- 24 discriminator_check only (account-type checks)
- 2 length_check + discriminator_check
- **12 ungated** (auditor focus, the high-signal subset)

All 24 `offset_overrun` findings carry at least one gate signal
(S5.2 lands via the shared detector).

### Folded — community PR #46 (`63fa3a3`, `1b348ba`, `7e4de28`, `bb5747b`)

Three Kani codegen fixes from @0xharp's real-world test-drive on
tendr.bid, plus example regen across the 5 bundled examples:

1. **`fix(kani): resolve effect-conformance RHS via target-aware
   binder`** — per-handler `verify_X_effect_Y()` fns emitted RHS as
   bare identifier; route through `resolve_value(.., Some("pre_"))`
   so any spec whose effect block copies a state field into another
   state field compiles.
2. **`fix(kani): inline mul_div helpers when standalone kani harness
   needs them`** — `tests/kani.rs` doesn't `pub use crate::math::*`;
   any spec calling `mul_div_floor` from an effect's let-binding
   failed. Inline canonical helper bodies, gated by
   `guards_use_math_helpers` so specs that don't use mul_div get a
   zero-byte change.
3. **`fix(kani): type-aware seed-state init via shared
   default_value_for_field`** — `emit_state_init_zeroed` literaled
   every field to `0` regardless of type. With v2.21's Pubkey
   lowering and any `Map[N] T` field, that produced E0308 mismatched
   types. Routes through the existing `default_value_for_field`
   helper (visibility bumped to `pub(crate)`).
4. **`chore(examples): regen bundled kani harnesses for kani.rs
   fixes`** — five `tests/kani.rs` regen so `qedgen check
   --regen-drift` stays clean on the Kani path.

## What's NOT shipped (intentionally)

### Slice 6 — `qedgen bench` subcommand

The bench corpus driver (list / run / report / add) was prototyped
as a CLI subcommand, the implementation collapsed to Option A per
PRD §S6.2 — emit the canonical recipe + sub-agent prompt to stdout
for the user to paste into their harness. That's a template
printer, not a real orchestrator. The harness-driven skill at
`.claude/skills/qedgen-auditor-bench/` does the same job without
bloating the CLI surface, and the auditor is harness-native by
design ([[feedback_audit_as_subagent]]).

Decision saved as a feedback memory ([[feedback_bench_stays_a_skill]])
to avoid re-litigating next release. The CLI commit was reverted
before push; v2.22 ships with no `qedgen bench` verb.

## Bench coverage on subscriptions Run A

The empirical bench-vs-firm result that drove this release:

| PRD ref | Firm finding | Status |
|---|---|---|
| CAN-H1 | `saturating_sub` collapsing time skew | ✅ `silent_success_arithmetic` |
| CAN-H3 | `checked_sub` permanent DoS on PDA init | ✅ `graceful_error_as_dos` |
| CAN-I3 | `period_hours * 3600` unchecked × fund-flow | ✅ `unchecked_arith_with_fund_flow` |
| CAN-M1 | `expiry_ts == 0` sentinel drift (fixed delegation) | ✅ `paired_validator_input_domain_mismatch` |
| CAN-M2 | `expiry_ts == 0` sentinel drift (recurring delegation) | ✅ (subsumed in same) |
| CAN-L2 | `end_ts` strictness drift (create vs update plan) | ✅ (subsumed) |
| CAN-L3 | `TIME_DRIFT_ALLOWED_SECS` tolerance asymmetry | ✅ (subsumed) |
| QED-HEAD-MED-3 | `close_subscription_authority` SPL Approve dangling | ✅ `external_authority_not_revoked_on_close` |

**8 named PRD-targeted shapes → 8 categorical hits.** Zero false
positives on `solana-program-escrow-2026-05` and
`solana-rewards-2026-05/rewards-ghsa-8hgp-gwvv-wrjf` clean smokes
across all new categories.

Plus the Slice 5 noise reduction:
- Subscriptions Run A medium findings: **114 → 12 ungated**.

Plus the Slice 3 Pinocchio brownfield empirical:
- `solana-program-escrow-2026-05` harness compiles + runs Crucible
  for 30s, reaching 16% branch coverage on the audited tree.

## Pre-release gates

- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo test` — 724 passed, 0 failed, 1 ignored (bin) + 6
      passed (crucible_brownfield_smoke integration)
- [x] `bash scripts/check-readme-drift.sh` — 18 CLI commands
      documented, no drift
- [x] `bash scripts/check-lake-build.sh --strict` — 10/10 examples
      green
- [x] `bash scripts/check-version-consistency.sh` — 2.22.0
      everywhere
- [x] Zero unintended `sorry` (ensures-as-axiom CPI theorems +
      DSL-builder string literals excepted)
- [x] `qedgen check --frozen` clean on all 5 bundled examples
- [x] `CLAUDE.md` ↔ `claude.md` byte-identical
- [x] `Cargo.toml` + `package.json` at 2.22.0
- [x] No `feedback_no_anchor_v2_mentions` violations

## Deferred to v2.22.x / v2.23

- **`qedgen bench add <repo-url> <audit-pdf>`** — corpus-entry
  scaffolding via PDF extraction subagent. Skill-side workflow item
  per [[feedback_bench_stays_a_skill]].
- **`bench run` headless autopilot** (`claude --headless` shell-out)
  — waiting on Claude Code's headless interface to stabilise.
- **Native + sBPF brownfield Crucible fuzz** — Native gates on Shank
  IDL discovery (v2.23 target); sBPF parked indefinitely (no
  AccountInfo at source level).
- **Seed-aware PDA derivation in brownfield** — Slice 3.3 emits a
  `find_program_address(&[], ...)` placeholder. Real derivation
  from Codama PDA seeds is v2.22.x scope.
- **In-tree Codama IR → Anchor 0.30 converter** — not needed today
  (Crucible's macro consumes IR natively); pulled in only if
  upstream drops support.
- **Slice 5 cross-function gated-load detector** — intra-function
  only for v2.22; widening to intra-module if bench evidence
  warrants.
- **Auto-stitch combined report from `work/<id>/<ts>*/score.json`**
  — `bench report` reads the hand-written canonical at
  `results/<id>.md`; auto-stitch is v2.22.x.

## Footer — relationship to existing memories

- [[project_subscriptions_cantina_bench]] — the design document. Every
  catalog gap in v2.22 traces back to a finding on this bench.
- [[feedback_audit_bear_hug]] — strategic frame; v2.22 lifts recall
  numbers without changing the composition-bias positioning.
- [[feedback_crucible_crash_first]] — Slice 3 closes the verification
  loop on Pinocchio.
- [[feedback_audit_as_subagent]] + [[feedback_bench_stays_a_skill]] —
  rationale for not shipping `qedgen bench`.
- [[feedback_minor_release_completeness]] — supports the 5-slice scope.
  Minors pile features.
- [[feedback_cleanup_v3]] — applies; v2.22 stays additive.
- [[feedback_no_anchor_v2_mentions]] — naming policy unchanged.
- [[feedback_multi_framework_audits]] — Slice 3's gate-on-Codama
  decision.
