# Design — Pre/post property lowering

**Status:** shipped in v2.23 (historical design note). Module names below predate the v2.32 MIR migration: `proptest_gen.rs` → `proptest_gen_mir.rs`, `kani.rs` → `kani_mir.rs`.
**Date:** 2026-05-20
**Triggered by:** `solana-payment-channels/.qed/plan/findings/001-temporal-marker-loss-in-proptest-lowering.md`
**Related findings:** 002 (quantifier drop — same anti-pattern, different surface)

## TL;DR

Properties authored with `old(state.x)` express a binary predicate over
`(pre, post)`. Today they lower into a **unary** Rust function
`fn p(s: &State) -> bool` where both sides of the comparison reference
the same state value, so every preservation property containing
`old(...)` collapses to a structural tautology (`s.x cmp s.x`) and the
proptest / Kani harness reports green. Lean lowers `old(...)`
correctly via `Ctx::Ensures` + `inside_old`; Rust does not. The Rust
side is missing both the property-fn signature shape (`fn p(pre, post)
-> bool`) and the harness-level pre-state capture
(`let pre = s.clone();`). Every preservation property is silently
vacuous. The fix is structural — classify properties at parse time,
bifurcate the Rust property fn shape, and capture pre-state in the
preservation harnesses.

## Bug surface, in one example

A spec from the bundled payment-channels program:

```
property settled_monotonic :
  state.settled >= old(state.settled)
  preserved_by all
```

Today's lowered Rust:

```rust
// proptest_gen.rs / kani.rs property fn:
fn settled_monotonic(s: &State) -> bool {
    s.settled >= s.settled        // E: tautology — always true
}

// proptest_gen.rs preservation test:
fn settle_preserves_settled_monotonic(s in arb_state(), voucher in 0u64..=u64::MAX) {
    let mut s = s;
    prop_assume!(settled_monotonic(&s));     // assume true (vacuous)
    if settle(&mut s, voucher) {
        prop_assert!(settled_monotonic(&s),  // assert true (vacuous)
                     "settled_monotonic must hold after settle");
    }
}
```

`proptest` reports the property green. Nothing about post-state
`settled` having to be `>=` pre-state `settled` is actually checked.
The same shape repeats for Kani.

Compare the Lean side — same spec, lowered correctly:

```lean
theorem settle_preserves_settled_monotonic
    (s : State) (voucher : U64) (h_pre : settled_monotonic_pre s) :
    let s' := settle s voucher
    s'.settled ≥ s.settled := by    -- s'.settled vs s.settled, real obligation
  ...
```

Lean works because `chumsky_adapter.rs:598 path_to_lean` honors the
`inside_old` flag — `state.x` lowers to `s'.x` (post-state) by default
in `Ctx::Ensures`, and to `s.x` (pre-state) inside `old(...)`. Rust's
`path_to_rust` at line 1025 takes the same flag — and ignores it. The
unary Rust harness shape leaves no pre-state for it to bind to anyway.

## Today's data flow (verbatim from the codebase)

```
parser (chumsky_parser.rs:330)
    ↓ Expr::Old(inner)
chumsky_adapter.rs:752  expr_to_rust(Expr::Old(inner), ...)
    ↓ render_path_with_pod(p, ctx, inside_old=true, ...)
chumsky_adapter.rs:946  render_path_with_pod
    ↓ path_to_rust(p, ctx, inside_old, consts)
chumsky_adapter.rs:1025 path_to_rust(p, _ctx, _inside_old, consts)  ← FLAG DROPPED
    ↓ "s.<field>"  (same for old(...) and bare state.x)
ParsedProperty.rust_expression  =  "s.settled >= s.settled"
```

Then both `proptest_gen.rs::emit_preservation_tests_for` and
`kani.rs` mutate `s` in place via `op(&mut s, ...)` and assert
`prop(&s)` post-mutation. Pre-state is no longer addressable.

## Lean's pattern (the precedent we're matching)

```
chumsky_adapter.rs:598 path_to_lean
  if state path:
    inside_old           → "s."         (pre-state)
    Ctx::Ensures         → "s'."        (post-state)
    Ctx::Guard           → "s."         (single state)
```

So Lean property bodies pick up both pre and post from one fn signature
that takes both. The Lean theorem statement provides `s` and a `let
s' := op s` binding; the property body's tokens render to the right
variable based on `inside_old` + `Ctx`.

The Rust analog needs:
1. A way for the property fn to receive both pre and post.
2. A path renderer that knows which one to emit.
3. A harness that captures pre-state before the mutation.

## Proposed model

### Property classification

Every parsed property gets a classification computed at AST time:

```rust
// crates/qedgen/src/check.rs (add to ParsedProperty)
pub struct ParsedProperty {
    // ... existing fields ...
    pub class: PropertyClass,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PropertyClass {
    /// Single-state predicate — no `old(...)`, no temporal markers.
    /// Lowers to `fn name(s: &State) -> bool`.
    Unary,
    /// Transition predicate — body references `old(...)`. Lowers to
    /// `fn name(pre: &State, post: &State) -> bool`. Only meaningful
    /// at handler boundaries, so this class is excluded from
    /// `assert_all_properties(&State, &str)` and from
    /// `prop_assume!` sites that bind only one state.
    Binary,
}
```

Classification is a one-pass AST walk: any `Expr::Old(_)` anywhere in
the property body ⇒ `Binary`; otherwise `Unary`. The walk is
straightforward — same shape as `quantifier::find_nested_quantifier`
which already exists.

This classification is **derived from existing AST nodes**, not a new
DSL surface. Spec authors keep writing `old(...)` — they don't
declare classes.

### Property fn shape, by class

Unary (unchanged):

```rust
fn settled_some_positive_local(s: &State) -> bool {
    s.settled > 0
}
```

Binary (new):

```rust
fn settled_monotonic(pre: &State, post: &State) -> bool {
    post.settled >= pre.settled
}
```

Path lowering rules inside a binary property body:

| spec form     | rust render | rationale                |
|---------------|-------------|--------------------------|
| `state.x`     | `post.x`    | default = post-state     |
| `old(state.x)`| `pre.x`     | `old(...)` = pre-state   |
| `param`       | `param`     | handler-param identifiers come from the harness, not state |

The path-rendering change is in `chumsky_adapter.rs::path_to_rust`:
honor the `inside_old` flag the same way `path_to_lean` does, and add
a `RustOpts.state_mode` field carrying `Unary | Binary` so the default
prefix (`s.` vs `post.`) is correct. The signature already threads
`RustOpts` everywhere; this is one new field.

### Harness change — pre-state capture

`proptest_gen.rs::emit_preservation_tests_for`:

```rust
// BEFORE (today, vacuous on binary properties):
let mut s = s;
if op(&mut s, args) {
    prop_assert!(prop(&s), "...");
}

// AFTER:
let mut post = s.clone();
let pre = s;
if op(&mut post, args) {
    match prop.class {
        PropertyClass::Unary  => prop_assert!(prop(&post), "..."),
        PropertyClass::Binary => prop_assert!(prop(&pre, &post), "..."),
    }
}
```

`State: Clone` already holds (`#[derive(Debug, Clone, Copy, Default)]`
on the proptest path; Kani harnesses use `kani::any()` which is `Copy`
for the same struct).

`kani.rs::emit_preservation_proofs` mirrors the change — symbolic
`pre`, `let mut post = pre;`, mutate post, assert based on class.

### `assert_all_properties` change

The aggregate predicate `assert_all_properties(s: &State, ctx: &str)`
is used at strategy-construction time to reject invalid states.
Binary properties have no meaningful single-state form — calling them
with `(s, s)` is the bug we're fixing. The aggregate emits only
unary properties; binary properties are excluded with a one-line
comment documenting the rationale.

`prop_assume!` sites inside preservation harnesses follow the same
rule — assume only unary properties of the pre-state. Binary
properties at the pre-state slot are necessarily true (`prop(pre,
pre)` is the trivial form), so excluding them costs nothing.

### Spec author surface

No DSL change. Authors keep writing:

```
property settled_monotonic :
  state.settled >= old(state.settled)
  preserved_by all
```

The classifier sees `Expr::Old(...)`, marks the property `Binary`,
and codegen does the right thing downstream. The DSL doc gets a one-
paragraph note that `old(...)` is what selects the per-handler
`(pre, post)` lowering — same content as today's "use `old(...)` for
preservation properties" guidance, with the lowering target made
explicit.

### Defense-in-depth lint

Even with the classifier in place, future codegen changes could
re-introduce the tautology. Add a P1 lint
`vacuous_property_lowering` with three rules, two unconditional and
one **AST-gated**:

1. **Codegen-induced tautology (AST-gated P1).** Property's AST
   contains `Expr::Old(_)` *and* its rendered `rust_expression`
   reduces to `e cmp e` with structurally identical sides. This is
   the 001 bug class — the spec carried temporal content, codegen
   silently dropped the marker, both sides collapsed to the same
   path.
2. **Unsupported-quantifier marker (unconditional P1).** Body
   contains `QEDGEN_UNSUPPORTED_QUANTIFIER`. Stronger sibling of
   today's `unsupported_quantifier_shape` (which only fires when
   `per_slot` is `None`); this one fires regardless.
3. **Literal `true` body (unconditional P1).** Body is the literal
   token `true`. Catches any other codegen path that short-circuited
   to a constant.

**Author-written tautologies are silently accepted.** A property
whose AST has no `Expr::Old(_)` and whose body renders to `e cmp e`
with identical sides is an authored choice (the bundled corpus has
one such case: `pool.qedspec:660-662 admin_field_tracked`, used as
a "surface this field in proofs without preservation noise"
pattern with an explicit rationale comment). Codegen translated it
faithfully; the lint has no business overriding the author. Rule 1
gates on AST `Expr::Old(_)` presence precisely so this case passes
silently.

The lint runs at the boundary between
`chumsky_adapter::expr_to_rust` and
`ParsedProperty.rust_expression` so it catches both today's known
cases and any future codegen path that re-introduces them. The
`ast_body` field on `ParsedProperty` (retained from the parse, see
Implementation pointers) is what Rule 1 consults.

### Tier coverage

| Tier        | Change                                                        |
|-------------|---------------------------------------------------------------|
| chumsky_adapter | `path_to_rust` honors `inside_old`; `RustOpts` gets `state_mode` |
| check       | `ParsedProperty.class` populated by AST walk                  |
| proptest_gen | Preservation tests capture pre, dispatch by class            |
| kani        | Symbolic `pre`, `let mut post = pre`, dispatch by class       |
| proptest_gen + kani | `assert_all_properties` / `prop_assume!` skip Binary  |
| check       | `vacuous_property_lowering` P1 lint                          |
| Lean        | unchanged (already correct)                                   |

## Worked example — the fix in action

Spec:

```
property settled_monotonic :
  state.settled >= old(state.settled)
  preserved_by all

property settled_capped :
  state.settled <= state.deposit
  preserved_by all
```

Generated `proptest.rs` excerpt:

```rust
// Unary
fn settled_capped(s: &State) -> bool {
    s.settled <= s.deposit
}

// Binary — note the (pre, post) signature
fn settled_monotonic(pre: &State, post: &State) -> bool {
    post.settled >= pre.settled
}

// Aggregate skips the binary one
fn assert_all_properties(s: &State, context: &str) {
    prop_assert!(settled_capped(s), "settled_capped failed at {}", context);
    // settled_monotonic is binary; not meaningful at single-state assert sites.
}

// Preservation tests
proptest! {
    #[test]
    fn settle_preserves_settled_capped(s in arb_state(), voucher in arb_voucher()) {
        let mut post = s.clone();
        let pre = s;
        prop_assume!(settled_capped(&pre));
        if settle(&mut post, voucher) {
            prop_assert!(settled_capped(&post),
                         "settled_capped must hold after settle");
        }
    }

    #[test]
    fn settle_preserves_settled_monotonic(s in arb_state(), voucher in arb_voucher()) {
        let mut post = s.clone();
        let pre = s;
        prop_assume!(settled_capped(&pre));   // unary prop_assume only
        if settle(&mut post, voucher) {
            prop_assert!(settled_monotonic(&pre, &post),
                         "settled_monotonic must hold after settle");
        }
    }
}
```

The monotonic test now genuinely fails on a buggy `settle` that
writes a smaller `settled` value; the unary capped test continues to
work as today.

## What it isn't

- **A new DSL feature.** The classification is derived from existing
  `Expr::Old(...)` nodes. No new keyword, no new spec shape.
- **A refactor.** No existing property semantics change. Unary
  properties (the vast majority of bundled specs) lower identically.
  The change is additive on the Binary path that was silently
  vacuous.
- **A breaking change for non-`old(...)` specs.** Specs with no
  `old(...)` see no diff in their generated harness.

Specs that already use `old(...)` *do* see new test logic — they
were testing nothing before; they will now exercise the real
preservation obligation. Some will catch bugs; some will surface
failures in the spec author's mental model. That's the point.

## What gets silently better

Every preservation property in every shipped spec that uses
`old(...)`. The bundled examples in `examples/rust/` that exercise
this pattern (escrow's watermark monotonicity, payment-channels'
settled monotonic, multisig nonce monotonicity, etc.) all get real
verification for the first time.

## What needs explicit attention

- **The bundled examples must regen.** Drift detector
  (`scripts/check-lake-build.sh`, `qedgen check --frozen`) will fire
  on every example whose property bodies contain `old(...)`. Each
  needs to be regenerated and (where they failed silently before)
  actively verified.
- **Tests that were silently passing may surface real bugs.** A spec
  whose `old(...)` body was wrong but never executed may now fail
  loudly. That's the contract repair, not a regression — but the PRD
  should account for triage time on the bundled corpus.
- **`assert_all_properties` callers.** Any test fixture that called
  this expecting all properties to be checked now sees a
  documentation comment that binary properties are excluded. Audit
  call sites.

## Test strategy

1. **AST-level snapshot tests** on classification: hand-built specs
   with various `old(...)` shapes, assert `PropertyClass` is the
   expected value. ~10 cases in `check.rs` test module.
2. **Codegen snapshot tests** in `proptest_gen.rs` /
   `kani.rs`: feed a small spec with one binary + one unary
   property, snapshot the emitted `.rs` and compare against a fixture
   in `tests/snapshots/`. Two cases per backend.
3. **Lint round-trip**: a "vacuous body" fixture that hand-emits
   `s.x >= s.x` triggers the new lint at P1.
4. **End-to-end on a bundled example**: regenerate
   `examples/rust/escrow-split/` from its committed spec, run
   `cargo test --test proptest`, confirm previously-vacuous
   preservation tests now exercise a real assertion (smoke check:
   inject a one-line bug into `settle`, watch the test fail; revert,
   watch it pass).
5. **`scripts/check-lake-build.sh --strict`** stays green — Lean
   side is unchanged.

## Migration shape for the bundled corpus

Per the pre-release checklist (CLAUDE.md), every
`examples/rust/*/formal_verification/` needs to lake-build clean and
every `qed.lock` needs `qedgen check --frozen` to pass. The
classification change re-fingerprints the property section of every
spec that uses `old(...)`. Concrete plan:

1. Regenerate every bundled example from its checked-in spec.
2. Run the proptest tier; expect green on correctness-clean specs.
3. For any spec that goes red, decide per-case: spec bug (fix the
   spec), implementation bug (fix the implementation), or
   handler-induced (file a follow-up and exclude `preserved_by`).
4. Refresh `qed.lock` files and commit.

This is mechanical work but it's load-bearing: shipping the fix
without re-fingerprinting silently de-syncs the lock files.

## Out of scope (separate findings, separate fixes)

- **Finding 002 — quantifier drop.** Largely addressed in v2.20 via
  `per_slot`. The `vacuous_property_lowering` lint catches the
  remaining cases (residual `true` bodies). Full classification of
  `Quantified` as a third property class is a follow-on if `per_slot`
  proves insufficient on real specs.
- **Finding 003 — unbound identifiers** (`now`,
  `ed25519_precompile_message`, `blake3(...)`). A spec-side DSL
  surface — out of scope for the lowering fix.
- **Finding 004 — post-emit `cargo check`.** A separate codegen-
  quality gate; this design doc closes the silent failure for the
  pre/post case, but a self-typecheck remains valuable for catching
  the other classes (duplicate structs, mixed signedness).
- **Findings 005 + 006 — brownfield spec↔impl binding.** Orthogonal;
  not touched here.

## Implementation pointers

Files that change:

- `crates/qedgen/src/ast.rs` — none (existing nodes suffice).
- `crates/qedgen/src/check.rs` — add `PropertyClass` enum, set it on
  `ParsedProperty`. ~30 LoC + classifier walk.
- `crates/qedgen/src/chumsky_adapter.rs`:
  - `RustOpts` gets `state_mode: StateMode { Unary, Binary }`.
  - `path_to_rust` honors `inside_old` (drop the `_` prefix on the
    arg; emit `pre.` in `inside_old`, `post.` or `s.` based on
    `state_mode`).
  - `expr_to_rust_with_state_mode` (or threaded through `RustOpts`)
    becomes the entry for property-body rendering. Property
    rendering passes `state_mode = Binary` when the
    classifier says so; everything else (guards, ensures rendered
    for transition-fn assumes) keeps `state_mode = Unary` (today's
    behavior).
- `crates/qedgen/src/proptest_gen.rs`:
  - `emit_property_predicate` (or equivalent fn) emits the binary
    signature when `class == Binary`.
  - `emit_preservation_tests_for` captures `let pre = s.clone();`
    and dispatches assertion shape by class.
  - `assert_all_properties` filters out binary properties.
  - `prop_assume!` loops over unary properties only.
- `crates/qedgen/src/kani.rs`: same set of changes as
  `proptest_gen.rs` adapted to the Kani harness shape.
- `crates/qedgen/src/check.rs` — add `vacuous_property_lowering`
  lint, P1.

Test fixtures:

- `tests/fixtures/binary_property/` — minimal spec with one
  `old(...)`-bearing property, asserted snapshot of generated Rust.
- `tests/fixtures/vacuous_lowering/` — hand-crafted spec whose
  `rust_expression` should trip the new lint.

Bundled-example regen list (concrete): every spec in `examples/rust/`
whose `properties { ... }` block contains `old(`. To enumerate:

```bash
grep -rl '\bold(' examples/rust/*/*.qedspec examples/rust/*/qed.toml 2>/dev/null
```

## Open questions

1. **`old(...)` inside `requires` / `invariant`.** ~~Open~~
   **Resolved 2026-05-20: reject at the `check.rs` level under
   `Ctx::Guard`.** Bundled-corpus audit found 0 of 45 specs using
   the pattern. The construct is a category error: `requires` is a
   precondition on the pre-state (no transition has happened yet —
   nothing to be "old"); `invariant` is a single-state predicate
   (the binary form is `property … preserved_by …`). Lean's current
   guillemet-quoted `«old(...)»` rendering is accidental
   documentation of the confusion. Slice 1b of the PRD ships the
   P1 `old_in_single_state_context` lint with a fix-it diagnostic.

2. **Should `assert_all_properties` get a sibling
   `assert_all_invariants(pre: &State, post: &State, ctx)` for the
   binary class, callable from integration tests after a handler
   call?** Useful but extra surface; defer until evidence we want
   it (someone hand-writes the call site twice).

3. **Property fn naming under class change.** Today a property `foo`
   becomes `fn foo(s: &State) -> bool`. After: unary stays `fn
   foo(s: &State) -> bool`, binary becomes `fn foo(pre: &State,
   post: &State) -> bool`. Same name, different arity — fine for
   Rust (no overloading expected) but a reader scanning the file
   sees the arity. The convention works; the alternative (suffix
   `_pre_post` or `_step`) adds visual noise. Recommend keep
   single name.

4. **Drift fingerprint coverage.** `crates/qedgen/src/fingerprint.rs`
   already hashes the property section; the rust_expression text
   change after this fix is naturally captured. No fingerprint-
   format change needed, but every existing `qed.lock` re-fingerprints
   — call out in release notes.
