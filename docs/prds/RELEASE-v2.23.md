# Release v2.23.0 — Pre/post property lowering

v2.23 closes a silent-vacuous-proof bug class in proptest and Kani
codegen: every preservation `property` whose body referenced `old(...)`
lowered to a structural tautology (`s.x cmp s.x`) and reported green
without actually checking the binary obligation. Lean's sibling path
had been doing this correctly for years; the Rust side was the gap.
The fix is structural — classify properties at parse time, bifurcate
the property-fn signature, and capture pre-state in the per-handler
preservation harness — plus two defense-in-depth lints
(`vacuous_property_lowering` for codegen regressions,
`old_in_single_state_context` for misuse in `requires` / `invariant`).

Trust-side slices (1, 1b, 2-7) ship in v2.23.0. Slice 8 (brownfield
first-contact onboarding flow) carries over to a follow-up; its scope
(~4 working days, new bundled example, two SKILL.md edits) doesn't fit
a focused trust-restoration release.

## What's in

### Slice 1 — AST classification (`PropertyClass` + `ParsedProperty.class`)

Every `ParsedProperty` constructed from a spec now carries a
`class: PropertyClass` field — `Unary` for the common single-state
predicate, `Binary` for bodies that reference `old(...)`. The walk is
a one-pass `expr_contains_old(&Node<Expr>)` in `chumsky_adapter.rs`,
shape-matching `quantifier::find_nested_quantifier`. `ast_body` is
retained on `ParsedProperty` so the Slice 5 lint can gate on temporal
markers without re-parsing.

### Slice 2 — `path_to_rust` honors `inside_old`; `RustOpts.state_mode`

`RustOpts` gains two fields — `state_mode: StateMode { Unary, Binary }`
and `inside_old: bool` — both default to today's behavior at every
existing callsite. `path_to_rust` now mirrors `path_to_lean` (line
598): in Binary mode, `state.x` lowers to `post.x` and `old(state.x)`
lowers to `pre.x`. Unary mode is the legacy `s.x` shape, unchanged.
The chumsky_adapter's `TopItem::Property` arm picks `state_mode` by
the classifier's verdict.

### Slice 3 — `proptest_gen.rs` bifurcation

Binary properties emit `fn <prop>(pre: &State, post: &State) -> bool`;
unary properties keep `fn <prop>(s: &State) -> bool`.
`emit_preservation_tests_for` now captures
`let pre = s.clone(); let mut post = s;` before the handler call and
dispatches the post-assert arity on `prop.class`:

| Class | Per-slot | Post-assert |
|---|---|---|
| Unary | `Some(slot)` | `<prop>_at(&post, binder)` |
| Unary | `None` | `<prop>(&post)` |
| Binary | (any) | `<prop>(&pre, &post)` |

`assert_all_properties(&State, &str)` and `prop_assume!` sites skip
binary properties — their `(pre, post)` signature has no single-state
form. Skip carries a one-line comment so a reader knows where the
binary obligation is checked (the per-handler harness above).

### Slice 4 — `kani.rs` bifurcation

Same shape as Slice 3 against the Kani harness. Non-init preservation
harnesses now emit:

```rust
let pre = State { /* every field = kani::any() */ };
kani::assume(pre.status == Status::<X>);
kani::assume(<unary-prop>(&pre));
let mut post = pre;
if <op>(&mut post, ...) {
    assert!(<prop>(&pre, &post), "...");  // binary
    // or
    assert!(<prop>(&post), "...");        // unary
}
```

Init harnesses emit the zeroed `let pre = State { ... }; let mut post = pre;`
shape for symmetry. The shared `rust_codegen_util::emit_property_predicates_with`
helper picks signature by `prop.class` so both Kani and any future
backend route through one decision.

State-Copy assumption (PRD open question 3): the Kani path assumes
`State: Copy` for `let mut post = pre;`. Every shipping spec satisfies
this; non-Copy state is a documented Kani-side limitation.

### Slice 1b — `old_in_single_state_context` lint

Walks every `requires` clause and every `invariant` body (both
expression-form and description-form skipped) looking for
`Expr::Old(_)`. Fires P1 with a fix-it diagnostic pointing the author
at `ensures` / `property` as the right context for transition-time
obligations. `requires` is a precondition on the pre-state — no
transition has happened yet, so there is no "old"; `invariant` is a
single-state predicate (the binary form is `property …
preserved_by …`).

Required threading `ast_body: Option<Node<Expr>>` onto
`ParsedRequires` and `ParsedInvariant`; populated at 5 production
sites in `chumsky_adapter.rs` (handler requires, interface-handler
requires, match-arm guard, expression-body invariant, plus two
synthetic-requires sites that carry `None`). `ParsedAbort` keeps its
existing shape — it's never populated in production.

Bundled-corpus audit (2026-05-20): 0 of 45 specs use this pattern.
Lint breaks no current example.

### Slice 5 — `vacuous_property_lowering` lint

Three rules in `check.rs`:

1. **Codegen-induced tautology (P1, AST-gated).** Fires when the
   property's AST body contains `Expr::Old(_)` *and* the rendered
   `rust_expression` parses as `<lhs> cmp <rhs>` with `lhs == rhs`.
   This is the 001 bug class — codegen dropped the temporal marker
   and both sides collapsed. Post-Slices 2-4 the rule should be
   unreachable from codegen; it stays as a regression net.
2. **Unsupported-quantifier marker (P1).** Fires when
   `rust_expression` contains `QEDGEN_UNSUPPORTED_QUANTIFIER`.
   Stronger sibling of the legacy `unsupported_quantifier_shape`
   lint (which only fires when `per_slot` is `None`); this one
   fires regardless.
3. **Literal `true` body (P1).** Fires when `rust_expression` trims
   to `true`. Catches any other codegen path that short-circuited
   to a constant.

**Author-written tautologies are silently accepted.** A property
whose AST has no `Expr::Old(_)` and whose body renders to
`<expr> cmp <expr>` with identical sides is an authored choice
(`pool.qedspec:660-662 admin_field_tracked` is the canonical
pattern); codegen translated faithfully and the lint stays out of
the way. Rule 1's AST gate is what enforces the silent-accept.

The string-level `parse_top_level_cmp` helper in `check.rs`
splits rendered Rust comparisons at the top-level operator (paren /
bracket / generic-args depth tracked) without round-tripping through
syn — fast and dependency-free.

### Slice 6 — Bundled-example regen + acceptance

Two of 45 bundled specs use `old(...)`:

- `examples/rust/percolator/percolator.qedspec` — `old(...)` lives
  in `ensures` only, which lowers via the transition-fn assume
  path (not the property-preservation path). v2.23 changes nothing
  in its generated harness; the regen is a no-op confirmation.
- `examples/regressions/issue-8/pool.qedspec` — the canonical
  pre/post test corpus. Five `property` bodies reference `old(...)`
  (`pause_blocks_mutation`, `slot_cursor_monotonic`,
  `transcript_epoch_monotonic`, `vectors_seeded_latches_true`,
  plus the implicational form on line 654). All five now emit a
  binary `(pre, post)` predicate and a per-handler preservation
  harness that captures pre-state before the handler call.

The shipped acceptance run confirmed both: percolator regen produces
byte-identical output; pool regen produces the binary harness shape
verbatim per the PRD's worked example.

### Slice 7 — Docs

- `references/qedspec-dsl.md` — the `old(...)` section now names
  the binary / unary classification and the codegen rule (`state.x`
  → `post.x`, `old(state.x)` → `pre.x`) inside binary bodies.
- `SKILL.md` — the `property` / `preserved_by` bullet under
  "Invariants vs Properties" calls out the v2.23 binary lowering
  and the `vacuous_property_lowering` lint.
- `CLAUDE.md` — pre-release checklist gets item 8a: regen every
  bundled spec with `old(...)` in a `property` body and confirm
  the binary signature ships.

## What's not in (carries to follow-up)

- **Slice 8 — Brownfield first-contact flow.** SKILL.md
  brownfield-detect branch, auditor → spec scaffold handoff,
  `references/finding_to_spec.md` mapping table, and a bundled
  `examples/rust/brownfield-onboarding/` walkthrough. ~4 working
  days of separate work that doesn't fit a focused trust-side
  release. Tracking the source signals
  ([[feedback_audit_as_brownfield_wedge]],
  [[feedback_audit_first_finding_buys_time]]) for the follow-on.

## Migration

Every spec whose `property` body contains `old(...)` re-fingerprints
under v2.23 — `prop.rust_expression` is now the binary form (`post.x`
/ `pre.x`) instead of the legacy collapsed form (`s.x`). `qed.lock`
files for those specs need a refresh. The bundled corpus has two
affected specs (per Slice 6); user-side specs need a one-time
`qedgen codegen` run plus `qed.lock` refresh.

The change is additive on the proptest / Kani output for any spec
without `old(...)` — unary properties keep their signature and body
verbatim, the preservation harness's `let pre = s.clone(); let mut
post = s;` rename is the only diff and it doesn't alter semantics.

Specs that used to pass on a vacuous preservation property may now
fail. That's the contract repair, not a regression — either the
spec was wrong (fix the spec), the implementation was wrong (fix the
implementation), or the property doesn't actually hold under that
handler (drop it from `preserved_by`).

## Footer

- **Source.** `solana-payment-channels/.qed/plan/findings/001-temporal-marker-loss-in-proptest-lowering.md`
  — the brownfield onboarding session that surfaced the bug class
  on a real spec. v2.23 closes finding 001 structurally.
- **Lean precedent.** `chumsky_adapter::path_to_lean` at line 598
  has always honored `inside_old` correctly. The Rust side now
  matches.
- **`per_slot` precedent.** v2.20 §S1.1 established the
  "classify at parse time, dispatch downstream codegen" shape that
  Slice 1 reuses for `old(...)`.
- **Memory updates.** `project_v223_shipped.md` (post-tag).
