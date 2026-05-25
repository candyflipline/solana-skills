# Codegen baseline — pre-MIR snapshot

**Captured:** 2026-05-24, against `main` at the start of the `mir` branch (commit `6113a13`, v2.29.2).

**Purpose.** The MIR proposal (issue #66) claims each codegen module should shrink in LoC and case-count after porting onto a typed intermediate representation. This file freezes the *before* numbers so the post-MIR claim is auditable.

**Method.** Raw LoC via `wc -l`. Case-count metrics via `grep` over the source — count of `=>` match-arms (proxy for explicit branching), count of `match ` blocks, count of fn definitions, and per-codegen dispatch on the `(field, op, value)` triple that effect lowering pivots on. Numbers are coarse but reproducible.

## LoC

Files measured: the five spec-aware emit modules called out by the MIR proposal (Lean, Anchor/Quasar, Kani classic, Kani impl-targeted, proptest) plus the integration-test and unit-test emitters that ride the same per-handler dispatch.

| File | LoC | Notes |
|---|---:|---|
| `lean_gen.rs` | **8,661** | Solana proofs (Rust + sBPF renderers); per-property + per-handler theorem emission |
| `codegen.rs` | **7,543** | Anchor + Quasar Rust scaffolding via `Target` enum; Pinocchio target reserved (errors at dispatcher) |
| `kani.rs` | 2,437 | classic ensures-preservation Kani harness shape |
| `kani_impl.rs` | 1,353 | impl-targeted Kani harness (v2.26) — calls real Anchor handler |
| `proptest_gen.rs` | 2,110 | proptest harness; per-handler property checks against the spec model |
| `integration_test.rs` | 681 | in-process SVM integration tests |
| `unit_test.rs` | 887 | unit tests |
| `rust_codegen_util.rs` | 1,398 | shared utilities between codegen.rs and integration_test.rs |
| **Sub-total — emit modules** | **25,070** | |
| `check.rs` | 11,436 | spec validation + lint + the `ParsedSpec` types every codegen reads |
| `chumsky_parser.rs` | 4,631 | surface parser |
| `chumsky_adapter.rs` | 5,245 | parser → AST adapter |
| `ast.rs` | 1,040 | typed AST (the parsed-side IR that the MIR proposal *complements*, not replaces) |
| **Sub-total — parsing/check** | **22,352** | |

The MIR proposal lists "five-plus" codegens; the actual layout is **four primary emit modules** (`lean_gen.rs`, `codegen.rs`, `kani.rs` + `kani_impl.rs`, `proptest_gen.rs`) plus two test emitters (`integration_test.rs`, `unit_test.rs`). Quasar is **not a separate codegen** — it is a `Target::Quasar` variant inside `codegen.rs`, sharing ~95% of dispatch with `Target::Anchor`. Track this — the "5 codegens" framing in #66 is slightly inflated.

## Case-count dispatch

| File | `match` blocks | `=>` arms | fns (any) | pub fns |
|---|---:|---:|---:|---:|
| `lean_gen.rs` | 66 | 80 | 179 | 2 |
| `codegen.rs` | 72 | 75 | 137 | 9 |
| `kani.rs` | 11 | 11 | 26 | 1 |
| `proptest_gen.rs` | 16 | 26 | 39 | 1 |

`lean_gen.rs` and `codegen.rs` each carry ~70 match blocks. Roughly per-handler-feature: lifecycle gating, effect dispatch, account-validation, CPI substitution, error mapping, schema include expansion, conditional-effect lowering. Most of these are the cross-cutting concerns #66 calls out.

## Effect-op string-literal dispatch (the proposal's central motivation)

Each codegen dispatches independently on the effect-op string in `(field, op, value)` triples: `set`, `add`, `add_sat`, `add_wrap`, `sub`, `sub_sat`, `sub_wrap`, `mul`, `div`. Count of string-literal matches per file:

| File | effect-op literals | effect-loop sites |
|---|---:|---:|
| `lean_gen.rs` | **33** | 13 |
| `codegen.rs` | 17 | 5 |
| `kani.rs` | 6 | 2 |
| `proptest_gen.rs` | 2 | 2 |

Three readings:

1. The dispatch IS duplicated. Even after recent consolidations (`rust_codegen_util.rs` lifted 1,398 LoC of shared Rust-side rendering), each codegen still owns its own table of op-string → emitted-form.
2. The duplication is uneven. `lean_gen.rs` has ~2× the dispatch sites of `codegen.rs`, and ~6× of `kani.rs`. Whatever consolidation gain MIR provides, it's biggest on the Lean side.
3. `proptest_gen.rs`'s low count (2 literals, 2 loop sites) suggests it already abstracts effect dispatch through a shared helper. Worth tracing which one — if the abstraction is good, it might be the template for the MIR-side dispatch.

## Conditional-effect lowering — concrete divergence

Issue #42 (closed 2026-05-18) added conditional effects via `ParsedEffectBranches`. References to that type per emit module:

| File | `ParsedEffectBranches` references |
|---|---:|
| `lean_gen.rs` | **4** (v2.21 Slice 4 — renders Lean `match` term) |
| `codegen.rs` | 0 |
| `kani.rs` | 0 |
| `proptest_gen.rs` | 0 |

**Only `lean_gen.rs` consumes `ParsedEffectBranches`.** Anchor, Kani, and proptest codegens see the same `ParsedSpec` and either fall back to flat-union lowering or quietly drop the conditional shape. This is exactly the bug-class the MIR proposal motivates: a new spec feature requires N+1 edits across emit modules; missing edits result in silent divergence rather than build failures.

**This is concrete evidence for the MIR thesis.** Worth including in the divergence inventory (task #4) and in the bug-bundle replay (task #12).

## Cross-cutting transforms — current footprint

These are the candidates to become MIR→MIR passes:

| File | LoC | What it does |
|---|---:|---|
| `cpi_substitute.rs` | 483 | substitutes interface ensures into caller proofs (v2.26 axiom-discharge + state-binders) |
| `consolidate.rs` | 208 | merges multiple proof projects into one |
| `interface_gen.rs` | 245 | renders interface stubs / qedspec-resolved imports |
| `cpi_substitute` + interface | 728 total | |

Plus schema-include expansion in `check.rs` (not factored out yet) and lifecycle-gating logic embedded in each emit module. Catalog these in task #6 before deciding what becomes a MIR→MIR pass.

## What this baseline does NOT measure

- **Cyclomatic complexity** per function. `wc -l` and grep arm counts are blunt; a function with 200 lines of straight-line string formatting is simpler than 50 lines of nested matches.
- **Test coverage** per codegen. Bug-class divergence shows up in tests, not in source. The bug-bundle replay (task #12) captures that.
- **Generated-output stability.** The "byte-for-byte diff or only cosmetic differences" acceptance criterion in #66 Phase 1 needs a separate fixture-output snapshot, not these source-file numbers.

## What success actually looks like

The point of MIR is **bug-reduction in codegen**, not LoC purity. Codegen has been the source of most qedgen bugs (the v2.15.1 bundle #39/40/41/43, the ParsedEffectBranches divergence above, the Lean codegen issues tracked in [[project-lean-codegen-bugs-v2-12]]). LoC shrinkage is a side-effect — useful as a sanity check, not as the goal.

The real success metrics, in priority order:

1. **Bug classes structurally eliminated.** Concretely: after MIR, can a "new spec feature handled by one codegen, silently dropped by another" bug still happen? Today it can (ParsedEffectBranches is the proof). Post-MIR, that class should be structurally impossible — adding a spec feature means adding an MIR `Stmt` kind, and every codegen has a `match` on `Stmt` that won't compile if a kind is unhandled.

2. **Bug-bundle replay.** Encode #39, #40, #41, #43 (and the ParsedEffectBranches gap) as fixtures that fail on `main`-era output and pass on MIR-era output without per-codegen patches. This is task #12; it's the audit trail that MIR fixes what it claims to fix.

3. **Cross-codegen consistency.** When the same `.qedspec` lowers through MIR, every codegen's view of "what fields this handler modifies" / "what effects this handler has" should agree by construction, because they're reading the same MIR node. The current divergence (33 effect-op string-literals in `lean_gen.rs`, 17 in `codegen.rs`, 6 in `kani.rs`, 2 in `proptest_gen.rs`) becomes one canonical lowering site per `Stmt` kind.

4. **LoC + case-count, as a downstream measurement.** Re-measure at the end of MIR Phase 4 and append a "post-MIR" section. Shrinkage is expected but not chased — if a codegen ends the port at the same LoC because its dispatch was already clean, that's fine; if it shrinks 50% because most of its logic was duplicated effect-op tables, that's also fine.

### What this means for the intrinsic-promotion rule

No fixed "≥3 of 5 codegens" quorum. Add a node shape when it eliminates a class of cross-codegen divergence bug. Don't add it for elegance. The MIR's value is judged at the per-shape level: does `Stmt::TokenTransfer` eliminate bugs the flat `(field, op, value)` dispatch can have? Yes → include it. Does `Stmt::SomeRareIntrinsic` only show up in one fixture and not eliminate a bug class? Skip it; fall back to the escape-hatch nodes.
