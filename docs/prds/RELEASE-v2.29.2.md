# Release v2.29.2

**Date:** 2026-05-24
**Type:** Patch — two lint refinements + three codegen fixes surfaced
by re-verifying the v2.26-era friction reports
(gists `209ef1ee5386e72eb59c3a9f064f4841` and
`d8ef6fb986db2eb1b6ac7a37303e9670`) end-to-end against the v2.29.1
binary.

The v2.29.1 release closed the surface area the bundled
`cross-program-vault` regression exercises. Re-running real-world
specs (the friction reports' distilled defi pool with 4 ref_impls,
9 handlers, multi-variant ADT lifecycle, ~10-account handlers using
Anchor's `init, associated_token::*` constraint sugar) surfaced
five gaps in shapes the narrow fixture missed. v2.29.2 closes
them and extends the regression set to cover the broader-shape
coverage gap so the next attempt actually fails when these break.

## What's in

### Fix — `unbound_auth` false positive on dotted-auth desugar

`check_unbound_auth` (`crates/qedgen/src/check.rs:5279`) had an
escape for "auth name manually bound via `requires` clause" that
only matched `s.<field>` state references. v2.29.1's dotted
`auth <acct>.<field>` sugar synthesizes a `requires <acct>.<field>
== <signer>.pubkey else Unauthorized` clause and rewrites `who` to
the lone signer — the synthesized clause references an imported
account, not `s.`, so the escape missed and the lint falsely fired
on the v2.29.1 feature it was meant to showcase. The bundled
`examples/rust/cross-program-vault/` tripped the warning on its own
`emergency_close` handler.

The fix extends the "manually bound" escape to also recognize the
shape `<acct>.<field> == <who>.pubkey` where `<acct>` is a non-
signer account on the handler. Covers both the v2.29.1 dotted-auth
sugar and the equivalent hand-written `requires` longhand uniformly.

### Fix — `missing_cpi_for_token_context` shape-based init detection

Two iterations rolled into one:

1. **First pass** (originally cut as v2.29.2): the v2.29 suppression
   (`check.rs:3658`) keyed on `pre_status in {"Uninitialized",
   "Empty"}` — a hardcoded two-name allowlist. User specs naming
   the pre-init variant differently (`Uninit`, `Created`,
   `NotInitialized`, `Setup`, etc.) tripped the lint spuriously
   despite shipping the exact Anchor `#[account(init, …)]` shape
   the suppression was meant to cover. Replaced with a shape
   predicate: the handler is a lifecycle-init iff `pre_status`
   names a unit (no-payload) variant declared on the spec's State
   ADT (or any sum type). Walks both `account_types[*].variants`
   (where multi-variant ADT state lives) and `sum_types[*].variants`.

2. **Second pass** (from the friction-report re-verification): the
   shape predicate added an extra `&& has_writable_token_account`
   sub-condition requiring a writable account explicitly declared
   `type token`. Real specs frequently leave token accounts bare-
   typed (`stablecoin_pool : writable`) and rely on Anchor's
   `#[account(init, associated_token::mint = X,
   associated_token::authority = Y)]` to resolve the type at
   scaffold time — those specs still tripped the lint. Dropped the
   sub-condition; `is_lifecycle_init && !has_calls()` is enough
   signal.

### Fix — `anchor-spl` deps detection scans `call Token.*` / `transfers`

`render_qedgen_cargo_toml` (`codegen.rs:4762`) keyed the `needs_spl`
gate on `handler.has_token_accounts()` only — accounts-block typing.
Specs that issue Token CPIs (`call Token.transfer(...)`,
`call Token.mint_to(...)`, etc.) without declaring any account
`type token` got a Cargo.toml without `anchor-spl`, but the handler
stubs still emitted `use anchor_spl::token::{self, Transfer};` for
the CPI bodies — producing `unresolved import anchor_spl` compile
errors. Extended the gate to also detect:

- any handler with `target_interface == "Token"` in its calls
- any handler with a non-empty `transfers { ... }` block (the
  `transfers` sugar desugars to Token CPIs internally)

### Fix — let-binding state refs route through the correct accessor

The biggest fix. Handler-body `let X = ref_impl(state.<field>, ...)`
emissions in `codegen.rs:3666` previously printed `let X = ref_impl(
s.<field>, ...);` verbatim, with `s` never bound in the handler-body
scope (the binder is `self`). Compounded by `guards.rs` referencing
the same let bindings in `requires X > 0` clauses where neither `s`
nor `X` were in scope.

Three orchestrated changes:

1. **New `rewrite_state_refs_for_self`** (`codegen.rs:2453`) — token-
   bounded word-level rewrite of `s.<field>` patterns to the
   handler-body equivalent. Multi-variant ADT fields route through
   the v2.29 Slice B accessor (`(*self.<state>.inner.<field>())`);
   flat-struct fields go to bare `self.<state>.<field>`. Mirror of
   the existing `bind_state` Step 2 logic in `generate_guards` but
   parameterized on `self` instead of `ctx`.

2. **Canonical-state fallback** (`codegen.rs:2417`,
   `resolve_handler_state_account` + `find_canonical_state_account_name`)
   — when a handler has multiple writable candidates and no PDA /
   `on_account` disambiguator (frequent shape: read-heavy handlers
   declare the state account `readonly` while several token / mint
   accounts are writable), pre-v2.29.2 `find_state_account` bailed
   to `None` and downstream rewrites no-opped. The new helper looks
   at the spec-wide canonical state account name (heuristic: highest
   `(writable_count, total_mention_count)` pair across all handlers,
   ties broken alphabetically) and reuses it for ambiguous handlers.
   The same resolver also fires at the accounts-struct generation
   site (`codegen.rs:3510`) so the canonical state account gets
   typed as `Account<'info, <StateStruct>>` instead of
   `AccountInfo<'info>`, making the field-access lowering type-
   check.

3. **Spec-level `let` emissions in guards.rs** (`codegen.rs:4782`) —
   guards.rs now emits the same spec-level let bindings the handler
   body emits, BEFORE the requires checks. Without this, a
   `requires lp_out > 0` clause references a name that's only bound
   in the user-owned handler stub (which runs AFTER the guard), and
   rustc rejects with `cannot find value 'lp_out'`.

## Regression coverage

Five new tests + one updated:

- `lint_unbound_auth_silent_on_dotted_auth_desugar` (`check.rs`) —
  distilled `cross-program-vault::emergency_close` shape; asserts
  no `unbound_auth` warning on the dotted-auth desugar path.
- `test_missing_cpi_for_token_context_suppressed_on_non_canonical_init_name`
  (`check.rs`) — mirrors the canonical-name suppression test with
  `Uninit` substituted.
- `test_missing_cpi_for_token_context_suppressed_when_no_typed_token_account`
  (`check.rs`) — lifecycle-init handler with `token_program` but no
  writable `type token` account; asserts the lint stays silent.
- `cargo_toml_includes_anchor_spl_when_token_cpi_without_typed_account`
  (`codegen.rs`) — minimal spec with `call Token.transfer(...)` but
  no `type token` accounts; asserts `anchor-spl` lands in the
  rendered Cargo.toml.
- `rewrite_state_refs_uses_canonical_fallback_when_handler_state_acct_is_readonly`
  (`codegen.rs`) — two-handler spec where the per-handler state
  resolver returns `None`; asserts the canonical fallback picks the
  expected state account and the rewriter binds `s.<field>` to
  `self.<canonical>.<field>`.

Plus the existing `test_missing_cpi_for_token_context_suppressed_on_lifecycle_init`
was updated to populate `account_types[*].variants` so the shape
predicate sees the State ADT (pre-v2.29.2 it passed by accident —
hardcoded name match didn't need ADT context).

## Verification

- `cargo fmt --check` — clean
- `cargo clippy -- -D warnings` — clean
- `cargo test` — all suites pass (945 in main bin + 24 macros + 5
  integration; full suite green)
- `scripts/check-version-consistency.sh` — Cargo.toml 2.29.2 ↔
  package.json 2.29.2
- `scripts/check-readme-drift.sh` — 19/19 CLI commands documented
- `qedgen check --frozen` on all bundled examples — no frozen
  failures
- End-to-end: the friction-report defi pool spec (9 handlers, 4
  ref_impls, multi-variant ADT state, two-CPI handlers, implicit
  Anchor init constraints) now `cargo check`s clean — 0 errors, 15
  cosmetic warnings.

## Out of scope

- General const-expression evaluation (`const X = 100 * 100`, `1 <<
  20`) — deferred to v2.30 per PRD-v2.29. The literal report case
  (`const N6 = 0 - 6`) works since v2.29.0.
- The canonical-state-account heuristic is intentionally simple.
  Specs with no clear "state-bearing account" pattern get
  alphabetic tiebreaking — deterministic but may not always match
  the spec author's intent. The right long-term fix is an explicit
  `state <account-name>` declaration in the spec; v3.0 surface
  candidate.
- Pre-existing `sorry` markers in 8 `examples/**/*.lean` files
  (pre-dating v2.29.1) remain as-is — out of scope for a lint-
  and-codegen patch release.
