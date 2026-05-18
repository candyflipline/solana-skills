# Release v2.21.0 — Crucible crash-first + spec-mode codegen quality + tooling ergonomics

> **Release status (2026-05-18):** paused on `feat/v2.21` at commit
> `7f6b94e` pending fold-in of deferred v2.21.1 items per
> `docs/prds/PLAN-v2.21-finish.md`. The pre-release gates below were
> green at `7f6b94e`; they'll be re-run after the fold completes.
> Branch is feature-complete for the PRD's blocking slices but the
> user explicitly chose to land S2.1 + S2.2 + S1.2 lamport + S1.2
> discriminator into the same release rather than ship them as
> v2.21.1.

v2.21 ships the bear-hug reposition of Crucible alongside the v2.20.x
backlog of codegen-quality fixes that real users (`rewards-feedback`)
hit on the spec-mode path. The headline is the Crucible reframe:
brownfield audits can now reach coverage-guided fuzzing without
authoring a `.qedspec` first.

## What's in

### Slice 1 — Crucible crash-first (`abcf40d`)

`qedgen probe --fuzz <budget> --root <project>` lifts the v2.20 gate
that required `--spec <path>`. The new brownfield path:

- Auto-detects the runtime (Anchor / Quasar / qedgen-codegen ship in
  v2.21; Pinocchio / Native / sBPF error with a v2.22+ pointer).
- Synthesises a minimal `ParsedSpec` from `pub fn name(ctx:
  Context<X>, ...)` signatures in `<root>/src/**/*.rs`.
- Writes a harness at `<root>/.qed/fuzz/<prog>/` whose
  `invariant_test()` body is empty — Crucible's intrinsic crash
  detector (panic, `unwrap` on `None`, `BorrowMutError`, arithmetic
  overflow) does the lifting. No spec invariants asserted.
- `--fuzz <budget> --spec <path>` (unchanged) still drives the v2.20
  spec-asserted harness; `--fuzz <budget> --spec <path> --root <path>`
  layers spec invariants on top.
- `--fuzz 0 --root <path>` is a dry-run that emits the harness and
  exits — handy for previewing what the agent needs to fill before
  paying the Crucible build cost.

```rust
// Mode: PROTOCOL (no spec). invariant_test() body is intentionally
// empty — Crucible still surfaces panics, unwrap-on-None,
// BorrowMutError, and arithmetic overflow as crashes via its
// host-loop crash detector. Spec-invariant assertions are not
// emitted in this mode.
#[invariant_test]
fn invariant_test(_fixture: &mut BuggyAnchorFixture) {
    // Protocol mode — no spec assertions.
}
```

Touched: `main.rs` (+162), `crucible_gen.rs` (+135),
`crucible_brownfield.rs` (new, ~400), `crucible_probe.rs` (+7),
`tests/crucible_brownfield_smoke.rs` (new, ~250), `references/cli.md`,
new fixture `examples/regressions/v2.21-crucible-crash-first/`.

**Deferred to v2.21.1** (PRD scope-guards): lamport-conservation
companion module, account discriminator / size invariants, auto-fill
of the `accounts(todo!())` agent-fill site, brownfield support for
non-Anchor runtimes.

### Slice 6 — codegen determinism (`405eeff`)

PR #45's rebase surfaced run-to-run drift in codegen output:
`render_properties_multi` collected per-target-account property
groups into a `HashMap`, then *iterated the map to drive output
emission*. Rust's `HashMap` is process-seeded, so two different
binaries (or the same binary across two processes) produced different
byte streams for the same spec on the same git tree.

`lean_gen::render_properties_multi` now uses `BTreeMap`. The other
HashMap / HashSet sites across `codegen.rs`, `kani.rs`,
`proptest_gen.rs`, `lean_gen.rs`, `rust_codegen_util.rs` are
membership / lookup operations whose iteration order doesn't reach
output — audited as part of this slice.

New regression test (`crates/qedgen/tests/codegen_determinism.rs`)
runs `qedgen codegen --all` twice per bundled spec and asserts
byte-identical output. Fires on any future HashMap-driven-output
regression.

Bundled examples did not churn — the prior HashMap happened to
produce the same key order as `BTreeMap` (alphabetic) on the
committed specs, but that was coincidence.

### Slice 2 (partial) — spec-mode codegen quality (`7b6e11b`)

Four of the seven rewards-feedback codegen bugs flagged in PRD-v2.20
§"Deferred to v2.21+" close in v2.21:

- **S2.3 — Cargo.toml clobber**: `generate_cargo_toml` now parses
  both the on-disk and freshly-rendered files into sections, replaces
  qedgen-owned sections (`[package]`, `[lib]`, `[features]`,
  `[dependencies]`, `[workspace]`) and upserts qedgen-owned deps
  inside `[dependencies]`, but preserves any user-added section
  (`[dev-dependencies]`, `[profile.release]`, custom features) and
  any user-added dep. PRD Option A.

- **S2.5 — `now()` builtin**: parses as `Expr::App { func: "now",
  args: [] }` via a new zero-arg atom in `chumsky_parser::group_b`.
  Lowers per backend:
  - Rust: `(solana_program::clock::Clock::get().unwrap().unix_timestamp as u64)`
  - Lean: bare `now`, axiomatized via new
    `QEDGen.Solana.Valid.now : Nat` (re-exported as unqualified
    `now` from `QEDGen.Solana`)
  - Kani / proptest: `kani::any::<u64>()` / `any::<u64>()`
  `walk_apps` skips `now` so per-spec uninterpreted axioms don't
  shadow the support-library declaration.

- **S2.6 — backslash continuation**: `"first \⏎ second"` joins into
  a single logical string; CRLF also accepted; existing `\\` / `\"`
  / `\n` / `\t` escapes preserved.

- **S2.7 — P7 lint** (`undeclared_state_field_in_effect`): fires
  when `effect { undeclared := ... }` or
  `effect { x := state.undeclared + 1 }` references a field absent
  from the spec's state schema. LHS + RHS check; the RHS scan looks
  at the rendered Lean form (`s.<field>`) to catch composition
  cases. Synthetic `_case_N` / `_otherwise` handlers skipped to
  avoid double-reporting.

**Deferred to v2.21.1+**: S2.1 (cross-state monomorphisation),
S2.2 (single-State Kani struct), S2.4 (Codama IDL ingest). Each
is a multi-day codegen pass per the PRD's own day estimate.

### Slice 3 — Pubkey state-field lowering (Option B) (`97553df`)

Pre-v2.21, Pubkey state fields were silently filtered out of the
proptest / Kani State struct (via `mutable_fields`'s `t != "Pubkey"`
predicate) while handler bodies still referenced them — producing 13
compile errors on `cargo test --test proptest` for any spec carrying
a Pubkey field. The P6 lint rejected the shape; the v2.20 workaround
was to move the field into a handler param.

v2.21 ships PRD Option B:

- `primitive_map(Pubkey, Standalone)` lowers to `[u8; 32]`. The
  user-facing Anchor / Quasar program target still emits real
  `Pubkey`.
- `mutable_fields` retains every declared field.
- `emit_state_struct` no longer bails on Pubkey.
- P6 downgraded from `Warning` to `Info` with a message describing
  the lowering; `test_complete_spec_clean` reverts to the original
  "zero Warnings" form.

Five bundled examples regenerated — Pubkey-carrying State structs
now exercise the harness end-to-end.

### Slice 5 — `qedgen check --regen-drift --write` (`88309b9`)

v2.20's `--regen-drift` detected but didn't fix. Rebasing PR #45
across v2.20 needed per-example `cd && qedgen codegen --all` with
manual target detection. v2.21 adds:

```bash
qedgen check --regen-drift --write
```

`WriteMode::Write` copies the temp-regenerated content over the repo
path for every `DriftKind::Changed` entry; `--write` exits 0 when
all such entries resolve, non-zero only when manifests are missing or
`MissingGeneratedCounterpart` entries remain.

The Slice 5 commit also lands the example catch-up the Slice 3
Pubkey change required (5 examples × ~7 files each).

### Slice 4 — Lean conditional effects (`a409ed0`)

(Shipped per the PRD's "ship-if-ready" guidance.) v2.20 lowered
`effect { match X { … } }` correctly for Rust but flattened it for
Lean — every potentially-modified field surfaced as a per-handler
obligation. v2.21's `render_transitions` dispatches on
`handler.effect_branches`, emitting:

```lean
def collect_feesTransition (s : State) ... : Option State :=
  if s.status = .Active ∧ amount > 0 then
    match fee_type with
    | 0 => some { s with fees_a_withdrawn := s.fees_a_withdrawn + amount, status := .Active }
    | 1 => some { s with fees_b_withdrawn := ..., status := .Active }
    | 2 => some { s with fees_c_accumulated := ..., status := .Active }
    | _ => some { s with fees_d_accumulated := 0, status := .Active }
  else none
```

Literal-integer + wildcard patterns only in v2.21; enum-pattern
lowering is v2.22 work per the PRD scope guard. Saturating /
wrapping ops collapse to the same `s.X + v` form as checked `+=`
because Lean `Nat` is unbounded.

## Pre-release gates

- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo test` — 715 passed, 0 failed, 8 ignored
- [x] `bash scripts/check-readme-drift.sh`
- [x] `bash scripts/check-lake-build.sh` — 10/10 examples green
- [x] `bash scripts/check-version-consistency.sh` — 2.21.0 everywhere
- [x] Zero unintended `sorry` (ensures-as-axiom CPI theorems excepted)
- [x] `qedgen check --regen-drift` clean against all 5 bundled
      examples
- [x] No `feedback_no_anchor_v2_mentions` violations
- [x] `CLAUDE.md` ↔ `claude.md` byte-identical
- [x] `Cargo.toml` + `package.json` bumped to 2.21.0

## Deferred to v2.21.1 / v2.22+

- **Slice 1 protocol-invariant companion module** — lamport-
  conservation diff across CPI returns + account discriminator /
  size invariants. v2.21 ships the brownfield emit pipeline; the
  five protocol invariants from the PRD's S1.2 land as three (panic
  / unwrap / borrow-mut, caught intrinsically by Crucible). Lamport
  + discriminator are v2.21.1.
- **Slice 1 non-Anchor brownfield runtimes** — Pinocchio / Native /
  sBPF. Brownfield mode errors with a clear "v2.22+ tracking"
  message.
- **Slice 2 deferred sub-bugs** —
  - S2.1: cross-state-type field monomorphisation into wrong
    modules (Distribution.balance vs Claim.balance disambiguation).
  - S2.2: single-State Kani struct for multi-ADT specs (mirror the
    proptest per-ADT module split).
  - S2.4: Codama IDL ingest in `qedgen spec --idl`.
- **Slice 4 enum-pattern Lean lowering** — identifier patterns
  over enum-shaped state (reserved at the v2.20 PRD level for v2.7's
  full enum-State work).
- **AskUserQuestion-equivalent across harnesses** — Codex / Cursor
  interview parity with Claude Code.
- **PRD-v2.20 §S3.6 hook auto-install** via `npx skills add` —
  ergonomics polish.

## Footer — relationship to existing memories

- `feedback_crucible_crash_first` — Slice 1 design principle.
- `feedback_audit_bear_hug` — strategic frame for Slice 1's
  brownfield path.
- `feedback_audit_first_finding_buys_time` — operational metric
  Slice 1 directly serves.
- `feedback_minor_release_completeness` — v2.21's six-slice scope
  fits the minor-release pattern.
- `feedback_cleanup_v3` — code cleanup still deferred to v3.0;
  v2.21 stays additive.
- `feedback_no_anchor_v2_mentions` — naming policy unchanged;
  swept on release prep.
