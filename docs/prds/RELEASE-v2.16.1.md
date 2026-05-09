# Release v2.16.1 — codegen bug-fix sweep

Patch release closing five codegen bugs surfaced in May (#38, #39, #40,
#41, #43). #42 (conditional effects in handler `effect` blocks) is a
DSL feature, not a bug, and defers to its own design pass.

## Issues closed

- **#38** Lean liveness theorems can be vacuous when transitions abort.
  The auto-proven path keeps the implication form — its proof script
  closes the success branch via `rfl`, so a working proof necessarily
  exhibits a real witness. The no-mechanical-path branch (where the
  earlier emission was `∃ ops, ops.length ≤ N ∧ ∀ s', applyOps … =
  some s' → s'.status = .target := sorry`) now emits the existential
  form `∃ ops s', applyOps … = some s' ∧ s'.status = .target := by
  sorry`, so a future hand-written proof must produce a non-aborting
  sequence rather than discharge a vacuously-true claim.
- **#39** proptest codegen exceeded `prop_map`'s 12-tuple limit on
  wide state structs. `emit_state_strategy_inner` now emits the
  `prop_compose!` form — same `impl Strategy<Value = State>`
  signature, no arity ceiling.
- **#40** generated guards referenced `s` without binding it on
  handlers without a writable state account. `find_state_account` was
  filtering on `is_writable`; now it falls back to non-writable PDA
  candidates after the writable-only pass, so view-style /
  pre-flight-check / claim handlers get the `s.field → ctx.<acct>.field`
  rewrite.
- **#41** operator-precedence breaking disjunctive `requires` in Kani
  codegen (and the proptest / runtime-guard paths sharing the same
  joiner). `collect_full_guard` in `rust_codegen_util.rs` now wraps
  each clause in `(...)` before `&&`-joining, so a clause containing
  a top-level `or` can't reassociate under Rust's `&&` > `||`
  precedence.
- **#43** Lean codegen emitted a duplicate `status` field on the
  State struct when the user already declared `status : U8`. Two
  helpers — `should_emit_lifecycle_marker` (matches Rust's `>= 2`
  threshold; single-state lifecycles no longer emit a marker) and
  `lifecycle_marker_name` (falls back to `qed_status` on collision).
  Threaded through every `s.status` / `status :=` emission site
  (multi-account `emit_state_struct`, `build_guard_cond_parts`,
  `render_transitions`, single-account `render_indexed_state`, and
  `render_liveness`).

## Backwards compatibility

The fixes are scoped to codegen output for specs that match the bug
patterns. Bundled examples in `examples/rust/{escrow,multisig,lending,
escrow-split}/` are unaffected — `dump_regen_specs` confirms the
regenerated `Spec.lean` for escrow, multisig, and lending is byte-
identical to the committed copies. Existing committed `proptest.rs`
files keep the inline-tuple form (which still compiles for ≤12-field
states); fresh `qedgen codegen --proptest` runs emit the new
`prop_compose!` form.

## Gates

- 508 + 24 unit tests pass
- `cargo fmt --check`, `cargo clippy --release -- -D warnings`,
  `bash scripts/check-readme-drift.sh`, `bash scripts/check-version-
  consistency.sh` clean
- `bash scripts/check-lake-build.sh` skipped (no local `lake`); the
  CI-side gate runs against an elan-equipped runner
