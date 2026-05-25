# Codegen divergence inventory — pre-MIR

**Captured:** 2026-05-24, `mir` branch off v2.29.2.

**Purpose.** The MIR proposal (issue #66) is motivated by cross-codegen divergence — a new spec feature handled by one codegen and silently dropped by another. This file enumerates every concrete divergence we can name today, so the MIR's value is judged against measured reality rather than abstract architectural claims.

Each entry lists: **shape** (what kind of divergence), **measured evidence** (grep counts or bug references), **impact** (what fails), **MIR fix** (does the proposed IR structurally prevent this?).

Three categories: A. cross-codegen feature-handling divergence (one codegen lowers it, another silently drops it), B. duplicated dispatch (same logic in N places, drifts independently), C. codegen-specific bug surfaces (representation choices that produce bugs unique to that codegen).

---

## A. Cross-codegen feature-handling divergence

### A1. `ParsedEffectBranches` — conditional effects only land in Lean

**Shape.** Issue #42 added conditional effects (`if/match` blocks inside handler effect bodies) via `ParsedEffectBranches`. Only one codegen reads the new structure.

**Evidence.**

| File | `ParsedEffectBranches` references |
|---|---:|
| `lean_gen.rs` | 4 (v2.21 Slice 4 renders a Lean `match` term) |
| `codegen.rs` | 0 |
| `kani.rs` | 0 |
| `kani_impl.rs` | 0 |
| `proptest_gen.rs` | 0 |

**Impact.** A handler with conditional effects gets a structurally accurate Lean theorem but Anchor/Kani/proptest fall back to the flat-union lowering (every potentially-modified field gets a per-handler obligation, regardless of which arm actually fires). The Anchor handler body is **less precise than the proof says**; Kani/proptest harnesses can't distinguish the arms.

**MIR fix.** Yes, structurally. `Stmt::Branch(pred, then_block, else_block)` in MIR makes the arms first-class node kinds; every codegen has a `match` on `Stmt` that won't compile if a kind is unhandled. Closes this class by construction.

---

### A2. Variant promotion (`state := .Variant {...}`) absent in Kani + proptest

**Shape.** The "promote one lifecycle variant to another, carrying payload" lowering. Codegen emits destructure-and-rebuild Rust (v2.29 Slice C); Lean emits the corresponding theorem. Kani and proptest emit nothing variant-specific.

**Evidence.**

| File | variant-promotion refs (`cross_variant` / `variant_promot` / `VariantTag` / `.Variant`) |
|---|---:|
| `codegen.rs` | 15 |
| `lean_gen.rs` | 3 |
| `kani.rs` | 0 |
| `kani_impl.rs` | 0 |
| `proptest_gen.rs` | 0 |

**Impact.** Verification gap. A handler that does `state := .Active { admin := signer.pubkey }` has its Anchor code generated and its Lean theorem stated, but Kani and proptest harnesses don't exercise the transition. Failures in variant-promotion bookkeeping (wrong fields carried, wrong tag set) would be caught by neither — they'd surface only when the user runs the deployed program.

**MIR fix.** Yes. A `Stmt::VariantPromote { from_tag, to_tag, payload_bindings }` (or similar — exact shape TBD in Phase 0) makes the operation a first-class node. Every codegen handles it explicitly or fails to compile.

---

### A3. Abort semantics — heavy in Lean, moderate in Anchor, thin to absent elsewhere

**Shape.** `requires X else throw ErrFoo`, `aborts_if`, abort-branch enumeration. Each codegen handles abort semantics differently — or not at all.

**Evidence.**

| File | abort refs (`ParsedAbort` / `aborts_if` / `emit_error` / `on_error`) |
|---|---:|
| `lean_gen.rs` | 48 |
| `codegen.rs` | 16 |
| `kani.rs` | 3 |
| `proptest_gen.rs` | 0 |

**Impact.** A handler whose spec says "must abort with `ErrInsufficientFunds` when `state.balance < amount`" has:
- Lean: the abort branch is a theorem obligation.
- Anchor: an `if … return err!(…)` early-return.
- Kani: a thin handful of assume/assert sites; abort-branch coverage is shallow.
- proptest: zero. The property generator doesn't construct inputs that trip the abort, and the harness doesn't assert that aborts happen at the right boundary.

**MIR fix.** Partial. `Stmt::Abort(err_ref)` + `Stmt::Branch(pred, ..., Stmt::Abort(...))` make abort sites first-class nodes; every codegen must lower them. But the *depth* of abort-branch coverage (e.g., does proptest construct adversarial inputs that hit the abort?) remains a codegen-implementation choice. Structural divergence closed; coverage divergence not.

---

### A4. CPI substitution / ensures-as-hypothesis — proof-side only

**Shape.** v2.26 axiom-discharge for Tier-1/2 callees: substitute callee ensures into caller proofs. Only proof-side codegens consume the substitution.

**Evidence.**

| File | cpi-substitute refs (`cpi_substitute` / `substitute_callee` / `render_cpi`) |
|---|---:|
| `lean_gen.rs` | 11 |
| `kani_impl.rs` | 6 |
| `kani.rs` | 1 |
| `codegen.rs` | 0 |
| `proptest_gen.rs` | 0 |

**Impact.** Mostly intentional — Anchor codegen emits the literal CPI call, not its substituted semantics. But proptest's zero refs are a real gap: the spec model proptest runs against has *no awareness* of what a CPI does to caller state. A handler that calls `Token.transfer(...)` and then asserts `state.balance < pre_balance` would not be checked by proptest (the model doesn't update the balance through the CPI).

**MIR fix.** Partial. `Stmt::Cpi { target, method, args }` is a first-class node; lowering rules can be uniform. proptest's model-side execution can apply the same substituted ensures the proof side does. But this requires *coordinated decisions* about callee semantics across codegens — the IR makes it possible, doesn't enforce it.

---

### A5. Quantifier handling — divergent per backend

**Shape.** `forall` / `exists` in requires/ensures. Each backend has its own lowering primitives.

**Evidence.**

| File | quantifier refs |
|---|---:|
| `lean_gen.rs` | 20 (native ∀/∃ + `Finset.sum`) |
| `proptest_gen.rs` | 7 (per-slot bounded generators) |
| `kani.rs` | 2 (bounded iteration over N slots) |
| `codegen.rs` | 0 (spec-only, no runtime emission) |
| `kani_impl.rs` | 0 |

**Impact.** Mostly expected — quantifiers are spec-only at runtime. But the *bounded* nature of Kani/proptest quantifier handling means a `forall` over an unbounded collection silently degrades to a per-slot check. If issue #67's quantifier proposal lands (over storage Maps), the bounded-vs-unbounded distinction needs to be enforced uniformly.

**MIR fix.** Partial. Quantifier expressions stay in the opaque-string `Expr` carrier per the MIR design constraint, so divergence in *expression rendering* (`∀ x ∈ S, P(x)` vs `kani::any() + assume`) persists. What MIR can fix: at the spec level, distinguish bounded-collection-quantifiers from unbounded-runtime-quantifiers as separate `Stmt` shapes, forcing each codegen to acknowledge the distinction.

---

## B. Duplicated dispatch

### B1. Effect-op string-literal dispatch

**Shape.** Each codegen owns its own table of effect-op string → emitted form. Adding a new op (e.g., `add_sat`) requires N independent edits.

**Evidence** (from `codegen-baseline.md`):

| File | string-literal matches | effect-loop sites |
|---|---:|---:|
| `lean_gen.rs` | 33 | 13 |
| `codegen.rs` | 17 | 5 |
| `kani.rs` | 6 | 2 |
| `proptest_gen.rs` | 2 | 2 |

**Impact.** When v2.24 added `+=!` / `+=?` (per-effect error names via [[project-v224-per-effect-errors]]), each codegen needed an independent edit to teach its dispatch about the new op-kind. Missing one would silently lower the new op as a default — likely as a wrapping arithmetic op, which is exactly the class of silent-failure bug the per-effect error proposal was meant to prevent.

**MIR fix.** Yes. `Stmt::CheckedAdd(path, expr, err_ref)` / `Stmt::SatAdd(path, expr)` / `Stmt::WrapAdd(path, expr)` are first-class node kinds. One canonical lowering per node per codegen. Missing a node fails to compile.

---

### B2. Lifecycle / status handling

**Shape.** `requires status == Active` / lifecycle preambles / `Status` enum management. Heavy duplication across three codegens.

**Evidence.**

| File | lifecycle refs |
|---|---:|
| `lean_gen.rs` | 76 |
| `kani.rs` | 68 |
| `codegen.rs` | 65 |
| `proptest_gen.rs` | 60 |
| `kani_impl.rs` | 0 (calls real handler, so lifecycle handled by the handler itself) |

**Impact.** 60–76 references each. Lifecycle is the most-duplicated cross-cutting concern in the codebase. The duplication shows up as inconsistent lifecycle behavior across codegens — bug #43 (Lean duplicate `status` field) is a direct consequence: lean_gen and codegen.rs encode lifecycle differently (Lean uses a struct field, Anchor uses an inner-enum tag), and the field-name collision only fires in Lean.

**MIR fix.** Yes, partially. A `HandlerMir.transition: Option<(VariantTag, VariantTag)>` field + `Stmt::LifecycleGate { allowed: Vec<VariantTag> }` makes lifecycle a first-class concept. Each codegen still chooses its representation (struct field vs inner-enum), but the *trigger* for lifecycle handling is one node, not 60+ scattered conditionals.

---

## C. Codegen-specific bug surfaces (representation choices)

These are bugs MIR doesn't directly prevent — they're consequences of each codegen making independent representation choices. MIR mitigates by reducing the *number* of choices but doesn't eliminate the class.

### C1. `#39` proptest 12-tuple limit (representation: tuple-based `prop_map`)

**Root cause.** `proptest_gen.rs` originally emitted an inline `(strat1, ..., stratN).prop_map(...)` over all state fields. `Strategy::prop_map` is implemented for tuples up to 12 elements. Wide state structs fail to compile.

**Why it's codegen-specific.** Lean has no tuple-arity limit; Anchor doesn't use tuples for state generation; Kani uses `kani::any()` per field. Only proptest's representation choice intersects with proptest's tuple-arity limit.

**Current status.** Fixed via `prop_compose!` migration (see `proptest_gen.rs:865`).

**MIR fix.** No structural fix — this is a proptest-internal representation choice. MIR may inadvertently constrain the choice space if proptest codegen consumes a typed `Stmt::Generate { fields: Vec<...> }` instead of inlining the tuple. Tertiary win at best.

---

### C2. `#40` guards reference `s` without binding (representation: conditional `s` scope)

**Root cause.** Anchor codegen's guard fn signature depends on whether the state account is writable — writable handlers get a different binder than non-writable ones. The conditional was buggy; non-writable handlers got a guard body that referenced `s` without ever binding it.

**Why it's codegen-specific.** Lean doesn't have the `&mut` / `&` distinction. Kani synthesizes its own state; proptest gets state from the generator. Only Anchor codegen's conditional-binder logic produces this class.

**Current status.** Fixed tactically (commit history shows the gate widening landed in v2.29 fix series).

**MIR fix.** No direct structural fix — the choice of binder is Anchor-codegen-internal. MIR can mitigate by emitting a canonical `state: &State` binding for every handler regardless of writability, but that's a codegen policy decision, not an IR-level structural fix.

---

### C3. `#41` operator precedence in disjunctive `requires` (representation: string concatenation)

**Root cause.** Kani codegen joined sibling `requires` clauses with `&&` and inlined each clause's expression as a raw string. A clause containing top-level `||` had its precedence regrouped by Rust's `&&` binding tighter than `||`.

**Why it's codegen-specific.** Lean uses parenthesized `∧`/`∨` natively; Anchor uses early-return per-clause (so each clause is its own statement, no precedence interaction); proptest uses `prop_assume!` per clause. Only Kani's concatenation approach produced this class.

**MIR fix.** No — MIR's opaque-expression-string constraint preserves this class. The pre-rendered `rust_expr_binary: String` (consumed by Kani) is still a raw string that needs to be parenthesized at concatenation sites. **The fix is at the lowering layer**: every `Expr::rust_expr_binary` should be wrapped in outer parens at concat time, defensively. This is a *coding discipline* fix, not an IR fix.

This is the most important caveat for the MIR proposal: structural typing of statements does not eliminate expression-rendering bugs.

---

### C4. `#43` Lean duplicate `status` field (representation: lifecycle as a struct field)

**Root cause.** Lean codegen encodes lifecycle as `structure State { status : Nat, ... }`. A user-declared `status : U8` field collides with the auto-emitted lifecycle field.

**Why it's codegen-specific.** Anchor encodes lifecycle as a separate `enum Status` + tag — no struct-field collision possible. Kani's symbolic state doesn't have named lifecycle fields. Only Lean's struct-field representation produces the collision.

**MIR fix.** No direct structural fix, but related to B2 (lifecycle handling). If MIR's `HandlerMir.transition` is the canonical lifecycle representation, the Lean codegen still has to choose how to encode it; the bug-class persists. The fix is to rename the auto-emitted field (e.g., `__lifecycle_status`) so it never collides — codegen policy, not IR.

---

## What MIR structurally fixes vs doesn't

**Structural wins** (the bug class becomes compile-time impossible):

- A1 ParsedEffectBranches divergence — every codegen handles `Stmt::Branch` or fails to compile.
- A2 Variant-promotion gap in Kani/proptest — `Stmt::VariantPromote` forces every codegen to handle it.
- A3 Abort-site enumeration — `Stmt::Abort` makes abort branches first-class; coverage depth remains per-codegen but presence is enforced.
- A4 CPI nodes — `Stmt::Cpi` is first-class; whether proptest applies the same callee-ensures semantics as Lean is a coordination question, not a divergence-discovery question.
- B1 Effect-op dispatch — one canonical lowering per `Stmt::CheckedAdd` / `SatAdd` / `WrapAdd` per codegen.
- B2 Lifecycle trigger — `HandlerMir.transition` + `Stmt::LifecycleGate` make lifecycle a single concept; representation is still per-codegen.

**Not fixed by MIR** (the bug class persists; mitigation is coding discipline or out-of-IR policy):

- A5 Quantifier *rendering* divergence — expressions stay opaque strings.
- C1 proptest tuple-arity — representation choice internal to proptest_gen.rs.
- C2 Anchor binder conditional — representation choice internal to codegen.rs.
- C3 Operator-precedence in concatenation — opaque-string expressions don't carry precedence info; mitigation = defensive parens at lowering time.
- C4 Lean field-name collisions — representation choice internal to lean_gen.rs.

## Implications

1. **MIR pays for itself on the A1, A2, A3, B1, B2 classes alone.** Five concrete bug-classes structurally eliminated; one of them (A1) has a closed issue that already failed to land cross-codegen.

2. **The "opaque expressions" constraint is the single biggest limit on MIR's reach.** Classes A5, C3 persist because expression rendering is per-codegen. If a future phase re-evaluates expressions-as-typed-trees, these would close — but that's a much bigger lift.

3. **Codegen-internal representation choices (C1, C2, C4) need per-codegen guidelines.** The MIR direction doesn't replace the need for codegen-quality work; it eliminates the cross-codegen *coordination* burden so the per-codegen work can focus on representation choices instead of dispatch sprawl.

4. **The intrinsic-promotion judgment** (per [[feedback-mir-is-bug-reduction]]): does this node shape close a class in A or B? Yes → worth adding. Does it only address something in C? Probably not enough leverage.
