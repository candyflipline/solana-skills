# qedgen MIR — design sketch

**Status:** Phase 1c-8 (multi-variant ADT path) and Phase 1d (snapshot equivalence) shipped on the `mir` branch. All 16 emit sections + ADT path emit content; snapshot tests gate every pilot fixture. ADT-path byte-equivalence achieved on `bundled-stdlib-demo` + `cross-program-vault`; `escrow-split` differs only in deferred §15 `cover_trace_proof` witnesses (~13 lines). Flat-path divergences (`escrow` / `lending` / `multisig`) pre-date Phase 1d and are the v2.30 follow-on. Handoff notes in §"Next-session handoff" below.

**Last revised:** 2026-05-25 (Phase 1d close-out).

**Companion docs** (read these first if you want measured evidence behind the claims here):

- [`codegen-baseline.md`](codegen-baseline.md) — LoC + case-count snapshot of the four primary emit modules.
- [`codegen-divergence.md`](codegen-divergence.md) — enumerated cross-codegen divergence classes with measured evidence.
- [`intrinsic-fixture-map.md`](intrinsic-fixture-map.md) — every spec feature × every fixture, mapped to candidate MIR shapes.
- [`cross-cutting-transforms.md`](cross-cutting-transforms.md) — which existing modules are genuine MIR→MIR pass candidates vs. already-shared vs. out-of-scope.

## Motivation

qedgen has four primary codegens (Lean, Anchor/Quasar via `codegen.rs::Target`, Kani classic + impl-targeted, proptest) plus two test emitters. Each lives in its own module and emits directly from `ParsedSpec`. Cross-cutting concerns — lifecycle gating, effect-op dispatch, abort semantics, account-block lowering, CPI substitution — are re-implemented per codegen. This is the source of most qedgen bugs: a new spec feature is N+ edits across emit modules, and missing one yields silent divergence rather than a build failure.

The divergence inventory shows the concrete pattern: `ParsedEffectBranches` (issue #42's conditional-effects shape) is consumed only by `lean_gen.rs` — Anchor, Kani, proptest have **zero** references. The same shape recurs for variant promotion (codegen.rs + lean_gen.rs only), CPI ensures substitution (Lean + Kani only), and others.

**MIR's goal: reduce codegen bugs.** Make cross-codegen divergence structurally impossible by replacing the shared-by-convention `ParsedSpec` dispatch with a typed `Stmt` IR every codegen has to `match` exhaustively. Per [[feedback-mir-is-bug-reduction]], LoC reduction is a side-effect; the metric is bug-class elimination.

## Position in the pipeline

Today:

```
.qedspec --> chumsky_parser --> ParsedSpec --> lean_gen.rs    --> Spec.lean
                                            \-> codegen.rs    --> programs/.../src/lib.rs (Anchor + Quasar via Target)
                                            \-> kani.rs       --> kani/harnesses.rs
                                            \-> kani_impl.rs  --> kani/impl_harnesses.rs
                                            \-> proptest_gen  --> tests/properties.rs
```

Cross-cutting transforms (`cpi_substitute.rs`, parts of `check.rs`) operate at the `ParsedSpec` level and feed each codegen, which then re-dispatches independently.

Proposed:

```
.qedspec --> chumsky_parser --> ParsedSpec --> [lower] --> MIR --> lean_codegen     --> Spec.lean
                                                              \-> anchor_codegen    --> programs/.../src/lib.rs
                                                              \-> kani_codegen      --> kani/harnesses.rs
                                                              \-> proptest_codegen  --> tests/properties.rs
```

Codegens consume MIR; none parse `.qedspec` or touch the chumsky AST. MIR→MIR passes (see `cross-cutting-transforms.md`) run between lowering and codegen.

## Key design constraints

### Expressions are opaque strings

This is the single most important constraint. The parser already lowers expressions per target (`ParsedRequires` carries `lean_expr`, `rust_expr`, `rust_expr_pod`, `rust_expr_binary` — four pre-rendered string forms per clause). Re-modelling expressions as a typed tree inside MIR would either (a) re-parse the pre-rendered strings (wrong direction) or (b) reach back to `crate::ast::Node<crate::ast::Expr>`, which only `ParsedRequires.ast_body` preserves (`ParsedEnsures` and `ParsedAbort` don't carry it).

So MIR is **structurally typed at the statement level, opaque at the expression level**:

```rust
pub struct Expr {
    pub lean: String,
    pub rust: String,
    pub rust_pod: String,
    pub rust_binary: String,
    pub source_span: Option<Span>,
}

pub struct Predicate(pub Expr);
```

Each codegen picks the field it needs (`expr.lean` / `expr.rust` / `expr.rust_binary`). The MIR's value comes from desugaring **structure and dispatch**, not from re-modelling expressions.

This caveat is load-bearing for the divergence inventory: classes A1, A2, A3, A4, B1 (per `codegen-divergence.md`) close under structural typing of statements. Classes A5 (quantifier rendering) and C3 (operator-precedence in concatenation) **persist** — they're expression-rendering issues opaque strings don't address. Mitigation for C3 is defensive parens at lowering time, which is coding discipline, not IR.

### qedgen-local scope; no qedsvm coupling

Per [[feedback-mir-is-bug-reduction]] and the conversation that produced this sketch:

- The `runMir` Lean-side operational semantics is **parked**. It was the Lean object every codegen would target; without it, codegens interpret MIR independently and lifestyle predicates live in qedsvm's `Svm/Solana/` library. Adding `runMir` later is purely additive.
- No `applyOp ≡ runMir` equivalence lemma. No `encodeState` / `decodeState` schema. No cross-repo migration of MIR-adjacent Lean primitives into qedsvm.
- qedsvm stays vendored at `lean_solana/QEDGen/Solana/SBPF/` until qedsvm tags a stable release. The `lake require` flip is deferred.
- qedbridge codegen (Phase 5 in the original proposal) is also deferred until qedsvm stabilizes. The MIR's design doesn't depend on it.

## Shape

```rust
pub struct Mir {
    pub name: Symbol,
    pub state: StateAdt,           // variants + fields, with field types
    pub accounts: AccountTable,    // PDAs, owners, writability, init, authority, token-type
    pub errors: ErrorEnum,
    pub interfaces: InterfaceRegistry,  // imported namespaces / interface stubs
    pub handlers: Vec<HandlerMir>,
    pub invariants: Vec<InvariantMir>,
}

pub struct StateAdt {
    pub variants: Vec<StateVariant>,
}

pub struct StateVariant {
    pub tag: VariantTag,
    pub fields: Vec<(Symbol, Ty)>,
}

pub struct HandlerMir {
    pub name: Symbol,
    pub discriminant: Option<Bytes>,
    pub params: Vec<(Symbol, Ty)>,
    pub accounts: Vec<AccountBinding>,
    pub auth: Option<AccountOrField>,             // signer requirement (from `auth` clause)
    pub transition: Option<(VariantTag, VariantTag)>,  // lifecycle arrow
    pub pre: Vec<Predicate>,                       // requires clauses (already with schema-includes expanded)
    pub body: Block,
    pub post: Vec<Predicate>,                      // ensures clauses
    pub emits: Vec<EventRef>,                      // auxiliary; out of body
}

pub struct Block(pub Vec<Stmt>);

pub enum Stmt {
    // ---- Solana intrinsics validated by ≥5 fixtures ----

    /// Authorization-or-abort. Canonical `requires X else Err` shape; 96 uses
    /// across 15 of 21 main fixtures. Closes divergence class A3.
    RequireOrAbort { pred: Predicate, err: ErrorRef },

    /// SPL Token Transfer. 7 fixtures (5 via `call Token.transfer`, 2 via
    /// `transfers {}` block). Closes divergence classes A2 (Kani/proptest
    /// gap) and A4 (CPI ensures coordination).
    TokenTransfer {
        from: AccountRef,
        to: AccountRef,
        amount: Expr,
        authority: AccountRef,
    },

    /// Lifecycle promotion to a new variant, carrying payload. Only 1 main
    /// fixture but several regression fixtures; closes class A2 for the
    /// Kani/proptest variant-promotion gap.
    VariantPromote {
        from_tag: VariantTag,
        to_tag: VariantTag,
        payload: Vec<(Symbol, Expr)>,
    },

    // ---- Effect-op kinds (validated by 10 fixtures + per-effect-error v2.24 work) ----

    /// `field := value`. Escape hatch for arbitrary effect RHS.
    Assign { path: Path, rhs: Expr },

    /// `field +=` with checked overflow → ErrorRef. Default arithmetic shape
    /// post-v2.24. Closes class B1.
    CheckedAdd { path: Path, delta: Expr, err: ErrorRef },
    CheckedSub { path: Path, delta: Expr, err: ErrorRef },

    /// `field +=!` — wrapping arithmetic, no error. v2.24 marker.
    WrapAdd { path: Path, delta: Expr },
    WrapSub { path: Path, delta: Expr },

    /// `field +=?` — saturating arithmetic, no error. v2.24 marker.
    SatAdd { path: Path, delta: Expr },
    SatSub { path: Path, delta: Expr },

    // ---- Control flow (closes class A1: ParsedEffectBranches divergence) ----

    /// Conditional effect block (issue #42). Currently only Lean codegen
    /// handles it; MIR makes it first-class.
    Branch {
        scrutinee: Predicate,  // or `match` on a value — see open Qs
        then_block: Block,
        else_block: Option<Block>,
    },

    /// Terminal abort. Used as the canonical post-`Branch` exit for fail paths.
    Abort(ErrorRef),

    // ---- Generic escape hatches ----

    /// Generic CPI to a non-Token interface. One occurrence in fixtures
    /// (`call Target.h` in a regression). Reserved for forward compatibility.
    Cpi { target: InterfaceRef, method: MethodRef, args: Vec<Expr> },

    /// Local binding inside a handler body. Rare in current fixtures but
    /// needed if any future surface adds explicit `let`.
    Let { name: Symbol, ty: Ty, value: Expr },
}

// Opaque expression carrier — no internal structure.
pub struct Expr {
    pub lean: String,
    pub rust: String,
    pub rust_pod: String,
    pub rust_binary: String,
    pub source_span: Option<Span>,
}

pub struct Predicate(pub Expr);

pub enum Ty { U8, U16, U32, U64, U128, I64, I128, Bool, Pubkey, Custom(Symbol) }

pub struct AccountTable {
    pub pdas: Vec<PdaDeclaration>,        // top-level `pda <name> [seeds]`
    pub bindings: BTreeMap<Symbol, AccountBindingShape>,
}

pub struct AccountBindingShape {
    pub writable: bool,
    pub readonly: bool,                    // redundant with !writable but stored for clarity
    pub init: bool,
    pub kind: AccountKind,                 // Signer / Token / Mint / Program / Pda / Plain
    pub authority: Option<AccountRef>,
    pub pda_ref: Option<PdaRef>,
}

pub struct PdaDeclaration {
    pub name: Symbol,
    pub seeds: Vec<Expr>,
}

pub enum AccountKind { Signer, Token, Mint, Program, Pda, Plain }

pub enum AccountRef {
    ByBinding(Symbol),                     // refers to an entry in AccountTable.bindings
    BySelf,                                 // 'self' / the handler's primary state
}

pub enum AccountOrField {
    Account(AccountRef),
    AccountField { account: AccountRef, field: Symbol },  // dotted-auth v2.29.1
}
```

Total statement kinds: **11** (4 primary intrinsics + 7 effect/control variants + escape hatches). Half the original proposal's 20-node list. Everything dropped (SystemTransfer / AccountInit / AccountClose / TokenMint / TokenBurn / TokenApprove / SysvarRead / DiscriminantMatch / SignerCheck-as-Stmt / LamportAssert / TokenExt / ProgramSpecific) lacks fixture evidence — see `intrinsic-fixture-map.md`.

## Design rules

1. **A `Stmt` kind earns inclusion by eliminating a divergence class.** Not by reaching a codegen-count quorum. Each kind above is traceable to a class in `codegen-divergence.md` or a fixture cluster in `intrinsic-fixture-map.md`. If a new spec feature can be lowered into existing `Stmt` kinds without re-introducing a divergence class, don't add a new kind.

2. **MIR is desugared, not optimized.** Surface sugar (`+=!`, `else Err`, schema-includes, dotted-auth, `transfers {…}` blocks) lowers to explicit primitive nodes during parser→MIR. Optimizations (const fold, dead-handler elimination) are not in scope for the initial port; revisit later if measurement shows them worthwhile.

3. **MIR is structurally typed at the statement level, opaque at the expression level.** Statement kinds are checked exhaustively; expressions ride as pre-rendered target strings. See "Expressions are opaque strings" above.

4. **MIR is finite and small.** Target: ~11 statement kinds, ~10 type constructors. If the IR grows beyond ~15 `Stmt` kinds, either the surface DSL is over-extended or the intrinsic set drifted from fixture evidence — investigate before adding.

5. **No control flow beyond `Branch` and `Abort`.** No loops, no early return, no exceptions. Solana handlers don't need them.

6. **AccountTable is foundational.** Account-block features (writable / init / authority / type / pda) account for the largest cross-codegen LoC weight (114 + 100 + 74 + 24 + 27 = 339 fixture references). Designing `AccountTable` is Phase 0 work, not deferred to Phase 3.

## Non-goals

- **Not a verification IR.** Proofs are stated against `Svm/Solana/` predicates in qedsvm, not against MIR. MIR is what spec semantics are *expressed* in, not what theorems are stated against.
- **Not a bytecode IR.** That's qedsvm's `Insn` + `CodeReq`.
- **Not an MLIR-style dialect framework.** One IR, no dialect registry, no extensibility hooks.
- **Not a Rust HIR-style typed AST.** The surface AST stays in `chumsky_parser.rs`; MIR is desugared and target-neutral.
- **Not a refinement-theorem emitter.** The original proposal positioned qedbridge codegen as the motivating consumer that justifies MIR's structural typing. That consumer is parked until qedsvm stabilizes; MIR's value case rests on bug-reduction across the four existing codegens.

## Cross-cutting passes (MIR → MIR)

Per `cross-cutting-transforms.md`, five genuine MIR→MIR pass candidates:

| Pass | LoC est. | What it does |
| --- | ---: | --- |
| `cpi_substitute` | ~150 (lift+adapt) | Precompute substituted callee ensures at MIR construction time; expose as `Stmt::TokenTransfer.callee_ensures: Vec<Predicate>` / `Stmt::Cpi.callee_ensures`. Closes A4. |
| `lifecycle_lower` | ~? | Synthesize an entry `Stmt::RequireOrAbort` checking `s.tag == from_tag` from the handler's `transition` field. Closes B1 (was 60–76 refs per codegen). |
| `variant_promote_check` | ~? | Validate that `Stmt::VariantPromote.payload` covers all fields of `to_tag`. Closes A2. |
| `effect_op_validate` | ~? | Ensure `Stmt::CheckedAdd` / `WrapAdd` / `SatAdd` carry the right error refs and target paths exist. Closes B3-shape correctness. |
| `account_consistency` | ~? | Validate that account references in statements match declared `AccountTable` entries. |

These run between parser→MIR lowering and codegen. They're **mandatory** for correctness, not optional optimizations. Optional passes (const_fold, dead_handler) are deferred.

## Migration path

Realistic per the `cross-cutting-transforms.md` analysis. The original proposal's 5–7 week estimate underestimated by ~2×; honest scope is **8–13 weeks** for the qedgen-local port.

| Phase | Scope | Estimate |
| --- | --- | --- |
| 0 | Define MIR types (Stmt + AccountTable + HandlerMir + Mir) + `lower(parsed: &ParsedSpec) -> Mir` function for the pilot scope. Validate against the TokenTransfer-using fixtures. **AccountTable is the major design artifact here** — it's foundational and has the most cross-codegen surface. | ~2 wks |
| 1 | Port Lean codegen for the pilot scope (TokenTransfer + RequireOrAbort + Assign + CheckedAdd/Sub + lifecycle gating via `HandlerMir.transition`). Keep `lean_gen.rs` as a fallback behind a flag. Acceptance: every TokenTransfer-using fixture produces byte-identical or cosmetic-diff-only Lean output. | ~2 wks |
| 2 | Port Anchor codegen (`codegen.rs::Target::Anchor` + `Target::Quasar`) for pilot scope. Second target validates the abstraction. Acceptance: every fixture's Anchor output is byte-identical or cosmetic-diff-only. | ~2 wks |
| 3 | Move MIR→MIR passes: `cpi_substitute`, `lifecycle_lower`, `variant_promote_check`, `effect_op_validate`, `account_consistency`. Reuse between Lean and Anchor validates the pass infrastructure. | ~1 wk |
| 4 | Port Kani (classic + impl) and proptest. proptest gains the `cpi_substitute` output for free, closing divergence A4 by construction. | ~2 wks |
| 5 | Add `Stmt::Branch` + `Stmt::VariantPromote` + `Stmt::WrapAdd` etc. to all codegens. The ones not exercised by the pilot land here. Bug-bundle replay tests (#39/#40/#41/#43 + the ParsedEffectBranches gap) become acceptance gates. | ~1–2 wks |
| 6 | Remove `ParsedSpec`-era fallback paths from all codegens. Delete dead code as cleanup. | ~3 days |

Total: ~10–13 wks of qedgen-local work. No qedsvm coupling, no qedbridge codegen — those come back when qedsvm tags stable.

## Risks

- **Over-abstraction.** Mitigated by the bug-reduction framing — every node kind traces to a divergence class. If we add a kind that doesn't close a class, drop it.
- **Lowering loses information.** Mitigated by keeping `ParsedSpec → Mir` lossless w.r.t. semantics. Source spans flow through opaquely on `Expr.source_span`.
- **Phase 3 underestimated.** The original proposal's 3–5-day estimate was off by ~5×. Real cross-cutting-transform work is ~12 days of port + ~12 days of codegen-side `match Stmt` rewrites. This sketch budgets Phase 3 at 1 wk plus the codegen-side work absorbed into Phases 1–2.
- **AccountTable design is the riskiest single artifact.** It carries the most cross-codegen surface (339 fixture references). Wrong shape forces revision across all four codegens. Mitigation: prototype against Anchor's `#[derive(Accounts)]` emission first (codegen.rs has 15 variant-promotion refs + 100+ account-attr refs to use as the validation surface).
- **Expression-rendering bugs (class C3) are not closed by MIR.** Opaque strings preserve operator-precedence concatenation hazards. Mitigation: at MIR construction time, wrap every `Expr.rust`/`Expr.rust_binary` in defensive outer parens before storage. Coding discipline at the lowering layer, not the IR layer.

## Open questions

1. **`Stmt::Branch` scrutinee shape.** Today's `ParsedEffectBranches` carries a `match`-on-value scrutinee. MIR's `Branch.scrutinee: Predicate` only models boolean tests; a `Stmt::Match { scrutinee: Expr, arms: Vec<(Pattern, Block)> }` may be needed for the issue #42 corpus. Resolve in Phase 0 against `examples/regressions/issue-42-conditional/fee_router.qedspec`.

2. **`InterfaceRegistry` shape.** Unified imports (v2.29 Slice F–I) populate `ParsedSpec.imported_namespaces`. MIR's `Mir.interfaces` either mirrors this directly or holds a different shape optimized for `Stmt::Cpi` callee-ensures lookup. Decide once `cpi_substitute` is ported (Phase 3).

3. **`Predicate` normalization.** Today each clause stores 4 rendered forms (`lean_expr`, `rust_expr`, `rust_expr_pod`, `rust_expr_binary`). MIR mirrors this. Should we add a `kani_expr` field for Kani-specific lowering, or keep `rust_expr_binary` as Kani's canonical input? Probably the latter — adding a fifth render field requires a parser change, which v2.30 should avoid.

4. **Source-location threading.** Every node carries an `Option<Span>` opaquely. Spans flow from chumsky's positions; renderers can ignore them. No further design needed in Phase 0 — confirm in implementation.

5. **`Mir.invariants` shape.** Issue #67's `rule` vs `invariant` distinction is parallel work. Until #67 lands, treat `invariants` as a `Vec<Predicate>` over `(pre, post): (&State, &State)`. Re-shape when #67's parser changes land.

## Implementation status (mir branch)

Tracking what's shipped on the `mir` branch vs. what's still planned. Commits referenced are short SHAs on `mir`.

### Phase 0 — typed IR + lowering — **shipped** (`ab4bdbe`)

- `crates/qedgen/src/mir.rs` (~870 LoC) — full type definitions per the §"Shape" above: `Mir`, `HandlerMir`, `Stmt` (12 kinds), `Expr` (opaque-string carrier), `AccountTable`, `AccountBindingShape`, plus references / types / errors / events / interfaces.
- `mir::lower(parsed: &ParsedSpec) -> Mir` for the pilot scope: handler bodies lowered through `RequireOrAbort`, `TokenTransfer`, `Assign`, `CheckedAdd/Sub`, `WrapAdd/Sub`, `SatAdd/Sub`, `Cpi`, `Emit`, `Abort`. `Branch` and `VariantPromote` recognize their source shape but emit stubs (Phase 5).
- `HandlerMir.transition` lifecycle threaded from pre/post-status.
- `AccountTable` populated from top-level `pda` declarations + per-handler account bindings.
- 10 lowering tests including 5 fixture-driven runs (escrow, escrow-split [multi-file], lending, multisig, bundled-stdlib-demo).

### Phase 1b — lean_gen_mir scaffold + flag — **shipped** (`f670404`)

- `crates/qedgen/src/lean_gen_mir.rs` mirrors `lean_gen::{generate, render}` entry-point shape.
- `QEDGEN_USE_MIR=1` env var routes `qedgen codegen --lean` through the new path. Default stays on legacy.
- Shape-detection dispatch (sBPF / indexed / multi-account / single-account) matches legacy; non-pilot branches emit marker stubs.

### Phase 1c — Lean emission for pilot scope — **closed for this session (14/16 + adjacents)**

Sub-slices shipped:

| § | Emitter | Slice | Status |
|---|---|---|---|
| 1 | `emit_header` (imports) | 1b | ✅ |
| 2 | `emit_namespace_open/close` | 1b | ✅ |
| 3 | `emit_uninterpreted_helpers` + `emit_ref_impls` | 1c-4 | ✅ |
| 4 | `emit_constants` | 1c-4 | ✅ |
| 5 | `emit_lifecycle_marker` (Status inductive) | 1b | ✅ |
| 6 | `emit_state_struct` (cross-variant union) | 1c-1 | ✅ |
| 7 | `emit_transitions` (per-handler) | 1c-1 | ✅ |
| 8 | CPI theorems | — | **deferred** (needs `Mir.interfaces` populated) |
| 9 | `emit_invariants` | 1c-3 | ✅ |
| 10 | `emit_operation_inductive` + applyOp | 1c-1 | ✅ |
| 11 | `emit_properties` + preservation | 1c-5 | ✅ |
| 12 | `emit_aborts_if` (legacy + requires-else) | 1c-2 | ✅ |
| 13 | `emit_ensures` | 1c-2 | ✅ |
| 14 | `emit_frame_conditions` | 1c-3 | ✅ |
| 15 | `emit_covers` + `emit_liveness` + `emit_environments` + `emit_overflow` | 1c-6 | ✅ (statements only — proof scripts deferred) |
| 16 | namespace close | 1b | ✅ |

**15 of 16 sections emit content.** End-to-end smoke-confirmed on `examples/rust/{escrow,lending,multisig,bundled-stdlib-demo,percolator}/*.qedspec` with `QEDGEN_USE_MIR=1`. 15 lean_gen_mir tests + 10 mir tests pass. Full bin suite at 970 passing.

Commit trail on `mir` branch:
- `ab4bdbe` Phase 0 — typed IR + lowering
- `f670404` Phase 1b — scaffold + `QEDGEN_USE_MIR=1` flag
- `60a8a38` Phase 1c-1 — transitions + Operation + applyOp
- `b9be609` Phase 1c-2 — aborts + ensures
- `01578c6` Phase 1c-3 — invariants + frame
- `c403089` docs — sketch progress catchup
- `42bdb06` Phase 1c-4 — constants + helpers + ref_impls
- `040a8b4` Phase 1c-5 — properties + preservation
- (next commit) Phase 1c-6 — cover / liveness / environments / overflow

### Deferred — return in a dedicated Phase 1d session

- **§8 CPI theorems** — `render_cpi_theorems` in legacy lean_gen.rs:`grep -n "^fn render_cpi_theorems"`. Requires populating `Mir.interfaces` from `ParsedSpec.imported_namespaces` + the bundled stdlib registry (SPL Token / System Program / Metaplex). Intersects with Phase 3's `cpi_substitute` MIR→MIR pass. ~1–2 days.
- **§15 proof scripts** — Phase 1c-6 emits cover / liveness / overflow theorems with `:= sorry` / `:= by sorry` bodies. Legacy lean_gen.rs has three auto-proof helpers — `cover_trace_proof` (witness construction over state-field defaults), `liveness_proof_script` (lifecycle-graph walk via `find_liveness_path` + `subst h_apply; rfl`), `overflow_proof_script` (`unfold + split + omega`) — that close many trivial cases. Environment theorems already auto-discharge via `unfold + dsimp + exact h_inv` when mutated fields don't appear in the property body. ~half to one day total when needed.
- **Multi-variant ADT path (`render_single_account_adt`)** — currently lean_gen.rs takes this branch for `escrow` (Uninitialized | Open | Closed); the MIR path emits the flat-state form, which diverges from legacy. Byte-equivalence for escrow requires implementing the inductive-State emission. ~2–3 days. Largest single deferred item.
- **Preservation proof scripts** — Phase 1c-5 emits property preservation theorems as `:= sorry`. legacy lean_gen.rs has a `preservation_proof_script` helper that discharges via `if_neg` / `dsimp + omega` projection. ~half day.
- **`rewrite_subscripts_lean` pass for ref_impls** — Phase 1c-4 emits ref_impl bodies verbatim; legacy applies a `m[i]` → `(m i)` rewrite for Map-typed params. Triggers when a fixture uses ref_impls with Map subscripts — no pilot fixture does today. ~half day when needed.

### Phase 1d — snapshot equivalence — **shipped**

Snapshot tests live at `crates/qedgen/tests/mir_snapshot.rs` with
per-fixture `Spec.lean` snapshots under
`crates/qedgen/tests/snapshots/`. Each test regenerates the MIR
output (`QEDGEN_USE_MIR=1 qedgen codegen --lean`) into an isolated
`git init`'d tempdir and asserts byte-equality against the snapshot.
Drift fails the test with a unified diff; intentional updates run
through `UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot`.

The snapshots lock the MIR output (not vs legacy). MIR ↔ legacy
parity per fixture is documented in
`crates/qedgen/tests/snapshots/README.md`:

| Fixture | Path | MIR ⇆ legacy |
|---|---|---|
| `bundled-stdlib-demo` | ADT | byte-identical |
| `cross-program-vault` | ADT | byte-identical |
| `escrow-split` | ADT | identical modulo deferred §15 `cover_trace_proof` witnesses (~13 lines) |
| `escrow` | flat | pre-existing transition body / `inductive Status` deriving order divergence |
| `lending` | flat | same |
| `multisig` | flat | same |

ADT-path byte-equivalence is the Phase 1c-8 deliverable; the flat-
path differences pre-date Phase 1d and are the remaining v2.30
follow-on (see "Deferred" below).

### Honest scoping

Phase 1 to byte-equivalence is ~4–7 more focused days. The pattern is locked in — each remaining gap is mechanical translation from a known `lean_gen.rs` section. Multi-variant ADT path is the biggest single item.

## Next-session handoff

For the next session picking up this work:

**Branch & toolchain:**
- Branch: `mir` (8 commits ahead of `main`; `main` is v2.29.2).
- Local: `.cargo/config.toml` carries `rustflags = ["-C", "symbol-mangling-version=v0"]` for the macOS linker workaround. See [[reference-macos-linker-workaround]].

**Smoke commands:**
- `cargo test -p qedgen-solana-skills --bins lean_gen_mir::tests` — MIR-codegen unit tests.
- `cargo test -p qedgen-solana-skills --bins mir::tests` — MIR lowering tests.
- `cargo test -p qedgen-solana-skills --test mir_snapshot` — Phase 1d snapshot equivalence over every pilot fixture. Use `UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot` to refresh after an intentional codegen change.
- `cargo fmt --check` + `cargo clippy -p qedgen-solana-skills -- -D warnings` — CI gates.
- `QEDGEN_USE_MIR=1 qedgen codegen --spec examples/rust/bundled-stdlib-demo/pool.qedspec --lean` — run the new path end-to-end on an ADT fixture. The bundled-stdlib-demo is byte-identical to legacy and exercises §8 CPI theorems + §S5 inductive State; restore with `git checkout -- examples/rust/bundled-stdlib-demo/` after eyeballing — codegen rewrites `programs/` too.

**Where the pieces live:**
- `crates/qedgen/src/mir.rs` — typed IR + lowering. Section dividers (`// ---- ----`) split the file. Search anchors: `pub struct Mir`, `pub enum Stmt`, `pub fn lower`.
- `crates/qedgen/src/lean_gen_mir.rs` — Lean emission. Section emitters are `emit_*` fns; the order in `render_single_account` mirrors `lean_gen.rs::render_single_account` (line 1177).
- `crates/qedgen/src/main.rs:3194` — dispatch gate (`if QEDGEN_USE_MIR { mir::lower → lean_gen_mir } else { lean_gen }`).

**Suggested first move in the next session:**
1. **§15 + §11 auto-proof scripts** — port `cover_trace_proof`,
   `liveness_proof_script`, `overflow_proof_script`,
   `preservation_proof_script` from legacy. Each replaces a `:=
   sorry` body with a real auto-discharge that closes trivial
   cases. ~1 day. Closes the ~13-line `escrow-split` MIR ↔ legacy
   diff entirely.
2. **Flat-path emitter alignment** — the MIR flat-state shape
   diverges from legacy in three families: (a) `inductive Status`
   `deriving` order + per-variant `: Status` annotation; (b)
   transition body lacks signer-equality / lifecycle-gate
   conjuncts and emits an unconditional auth alias; (c) cover
   trace proofs emit `:= sorry` rather than legacy's `decide`
   witnesses. ~2–4 days; closes `escrow` / `lending` / `multisig`
   byte-equivalence.
3. **Phase 2 — multi-account codegen** (`render_multi_account`
   stub today). Out of Phase 1 scope; the design note in
   `qedgen-mir-sketch.md` §"Phase ordering implication" budgets it
   at ~1 week.

**What NOT to do without revisiting:**
- Don't try to refactor the flat-path emitter into a "deriving
  preference" parameter shared with the ADT path — the ADT and flat
  emitters have legitimately different goals (variant pattern-match
  vs flat-struct guards) and the byte-shape mismatch isn't just
  formatting drift. Port the legacy emitter behavior section-by-
  section like Phase 1c-8 did.
- Don't add a parallel `Mir.interfaces` lift alongside
  `Mir.imports` — the unified shape resolved in
  [`mir-unified-imports.md`](mir-unified-imports.md) makes
  `Mir.imports` canonical. Re-introducing the split would re-
  create the exact debt this MIR exercise pays down.

## What the companion docs validate

| Claim in this sketch | Evidence |
| --- | --- |
| Four primary codegens, not five | `codegen-baseline.md` "5 codegens framing is slightly inflated" |
| ParsedEffectBranches divergence is real (Lean-only) | `codegen-baseline.md` table + `codegen-divergence.md` A1 |
| Variant-promotion gap in Kani/proptest | `codegen-divergence.md` A2 |
| Abort semantics divergence | `codegen-divergence.md` A3 |
| Lifecycle gating is the highest-weight cross-cutting concern | `codegen-baseline.md` (60–76 refs/codegen) + `codegen-divergence.md` B2 |
| TokenTransfer is the only meaningful CPI shape | `intrinsic-fixture-map.md` (8 of 9 CPI occurrences) |
| `RequireOrAbort` is the most-used non-arithmetic node | `intrinsic-fixture-map.md` (15 fixtures, 96 uses) |
| AccountTable carries the largest cross-codegen surface | `intrinsic-fixture-map.md` "Implications" §2 |
| Half the proposal's intrinsic list lacks fixture evidence | `intrinsic-fixture-map.md` "What's in the proposal but not in fixtures" table |
| Phase 3 estimate was off by ~5× | `cross-cutting-transforms.md` "Phase ordering implication" |

## Cross-references

- `docs/design/mir-unified-imports.md` — Phase 1c-7 design note. `Mir.imports` collapses the parallel `ParsedSpec.interfaces` + `ParsedSpec.imported_namespaces` surfaces into one canonical lifted structure; sequencing + open questions + validation plan.
- `docs/design/spec-composition.md` — Tier 1/2/3 interface composition (relates to `Mir.imports[*].interfaces`).
- `docs/design/pre-post-property-lowering.md` — current pre/post handling at the ParsedSpec level; lowering moves into parser→MIR.
- `crates/qedgen/src/lean_gen.rs` — current Lean codegen; Phase 1 rewrites this on top of MIR.
- `crates/qedgen/src/codegen.rs` — current Anchor/Quasar codegen; Phase 2 rewrites.
- `crates/qedgen/src/cpi_substitute.rs` — current CPI substitution; Phase 3 ports to MIR construction time.
- Issue #66 (the original proposal) — this sketch is the qedgen-local refinement of #66 after measurement.
- Issue #67 (`.qedspec` evolution: rules vs invariants, ghost vars, hooks, quantifiers, havoc) — items 1, 2, 4, 5 land above MIR (parser changes only); item 4 (hooks) is gated on `Stmt`-boundary instrumentation which MIR makes possible.
