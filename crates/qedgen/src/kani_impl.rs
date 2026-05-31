//! Impl-targeted Kani harness emission (v2.26 Batch 2 — Track H).
//!
//! The v2.25 ensures-preservation harness (`kani.rs`) verifies the spec's
//! own translated transition fn against its declared `ensures` clauses. That
//! catches spec-internal inconsistency — useful, but doesn't validate that
//! the *user's Rust handler* satisfies the contract.
//!
//! This module emits a parallel harness shape that calls the user's REAL
//! Anchor handler against a symbolic `Accounts` context. Pre/post account-
//! field snapshots replace the spec-model `pre = s.clone()`; the assertion
//! body reuses `ParsedEnsures.rust_expr_binary` so the same `pre.x` / `post.x`
//! rendering applies — but `pre`/`post` are now flat `pre_<field>` /
//! `post_<field>` locals reading from account data instead of `State` copies.
//!
//! ## Triggers (opt-in)
//!
//! 1. User passes `--kani-impl` to `qedgen codegen`.
//! 2. Auto-trigger: any handler has `modifies` listing fields not present in
//!    the effect block's LHS (the v2.25 LP-shape signal indicating the impl
//!    is expected to fill those fields via the agent-fill `todo!()` site).
//!    When auto-triggered, the file header carries a comment naming the
//!    triggering handler(s).
//!
//! ## CPI ensures-as-fact (Track I)
//!
//! When the handler does `call Foo.bar(...)` and the callee declares
//! `ensures`, we splice `kani::assume(<callee_ensures, substituted>)` lines
//! between `if result.is_ok()` and the first caller `assert!`. The
//! substitution maps each callee param to the caller's call-site expression
//! via `crate::cpi_substitute::substitute_callee_ensures_rust_binary` — the
//! same helper `kani.rs`'s spec-model harness uses. The substituted clauses
//! come back in `pre.X` / `post.X` form (from `rust_expr_binary`); we then
//! flatten those to the harness-local `pre_X` / `post_X` snapshots via
//! `rewrite_pre_post_paths`.
//!
//! Tier-0 callees (no `ensures` declared) emit nothing — same fallback as
//! the spec-model variant and `lean_gen.rs::render_cpi_theorems`'s
//! `:= by sorry`. The `cpi_no_callee_ensures` lint surfaces the gap at
//! check time.
//!
//! ## Per-target shapes
//!
//! - Anchor / Quasar — struct-based: build a symbolic `crate::<Pascal>`
//!   accounts struct and call its `handler(&mut self, …)` method (the
//!   Quasar `#[program]` / `Ctx<X>` dispatcher just forwards to that same
//!   method, so the two share `emit_symbolic_accounts_module` +
//!   `emit_handler_harness`). See `emit_kani_impl_quasar` (slice 5).
//! - Pinocchio — stack-allocated `AccountInfo` via a `#[repr(C)]` layout
//!   mirror + transmute, calling the real `process_<handler>` against raw
//!   account slices. See `emit_kani_impl_pinocchio` (slice 8).
//! - native targets — not yet emitted; the per-target dispatch in
//!   `generate_from_spec` is the seam a future arm plugs into.

use anyhow::Result;
use std::path::Path;

use crate::check::{self, ParsedHandler, ParsedHandlerAccount, ParsedSpec};
use crate::codegen::{map_type, to_pascal_case, to_snake_case};
use crate::Target;

/// Predicate: a handler triggers auto-emission of an impl-targeted harness
/// when its `modifies` clause lists at least one field that does NOT appear
/// as the LHS of any effect in its `effect` block. This is the LP-shape
/// signal — the agent-fill `todo!()` site expects the user's Rust impl to
/// satisfy the contract for that field.
///
/// Mirrors the diff logic in `codegen.rs` Phase A so the trigger here and
/// the agent-fill emission there stay in lock step.
pub fn handler_triggers_impl_harness(handler: &ParsedHandler) -> bool {
    let Some(modifies) = &handler.modifies else {
        return false;
    };
    let effect_lhs: std::collections::BTreeSet<String> = handler
        .effects
        .iter()
        .map(|(lhs, _, _)| {
            // Strip array index suffix the same way Phase A does, so
            // `lp_supply[i]` doesn't false-positive against a bare
            // `lp_supply` in `modifies`.
            let bare = crate::rust_codegen_util::effect_target_base(lhs);
            bare.to_string()
        })
        .collect();
    modifies.iter().any(|f| !effect_lhs.contains(f))
}

/// Predicate: any handler in the spec triggers the auto-emission. The CLI
/// consults this before emitting the impl harness file when `--kani-impl`
/// was NOT passed explicitly.
///
/// Two trigger conditions:
///   1. Handler `modifies ⊋ effect.lhs` — the LP-shape signal (Track H).
///   2. Any `ref_impl` carries potentially-overflowing arithmetic over
///      bounded-numeric params (`ref_impl_has_overflow_risk`). Lean
///      proves on unbounded `Nat`/`Int`; Kani is the only verification
///      surface that catches the `u64`/`i64` overflow.
pub fn spec_triggers_impl_harness(spec: &ParsedSpec) -> bool {
    spec.handlers.iter().any(handler_triggers_impl_harness)
        || spec
            .ref_impls
            .iter()
            .any(crate::check::ref_impl_has_overflow_risk)
}

/// Names of handlers whose `modifies ⊋ effect.lhs` causes the auto-trigger.
/// Surfaces in the generated file's header so the user understands why an
/// impl harness appeared without `--kani-impl`.
fn auto_triggered_handlers(spec: &ParsedSpec) -> Vec<&str> {
    spec.handlers
        .iter()
        .filter(|h| handler_triggers_impl_harness(h))
        .map(|h| h.name.as_str())
        .collect()
}

/// Emit `programs/tests/kani_impl.rs` against the user's real Anchor
/// handlers. `explicit_flag` is true when `--kani-impl` was passed; auto-
/// triggered emission stamps a header comment naming the triggering
/// handlers. Non-Anchor targets (Quasar, Pinocchio) are a clean no-op
/// (no file written) — see the target gate in `generate_from_spec`.
///
/// Per-handler emission is gated on the handler having at least one
/// `ensures` clause — without ensures there's nothing to assert.
pub fn generate(
    spec_path: &Path,
    output_path: &Path,
    explicit_flag: bool,
    target: Target,
) -> Result<()> {
    let spec = check::parse_spec_file(spec_path)?;
    generate_from_spec(&spec, output_path, explicit_flag, target)
}

/// Same as `generate` but takes a pre-parsed spec. Used by the CLI when
/// it already has a `ParsedSpec` in hand (avoids the second parse).
pub fn generate_from_spec(
    spec: &ParsedSpec,
    output_path: &Path,
    explicit_flag: bool,
    target: Target,
) -> Result<()> {
    // Target gate. All three
    // targets now emit; only the harness body shape differs per framework
    // (see the `match target` dispatch below). The gating that follows
    // (auto-trigger, ensures-present, emit-targets) is target-agnostic.
    //   - Anchor    → `crate::<Pascal>` accounts struct + `.handler()`
    //   - Quasar    → same struct-based `.handler()` shape (slice 5); the
    //     `#[program]` / `Ctx<X>` dispatcher just forwards to that method
    //   - Pinocchio → stack-allocated `AccountInfo` via `#[repr(C)]`
    //     layout mirror + transmute (slice 8 M3; the shape validated
    //     by examples/pinocchio-fixtures/ptoken-transfer/src/kani_impl.rs)
    let auto_handlers = auto_triggered_handlers(spec);

    // Skip emission entirely if neither the explicit flag NOR an auto-
    // trigger applies. Belt-and-suspenders check — the CLI's `want_kani_impl`
    // already gates the call, but keeping the check here lets the regen-drift
    // path call `generate` unconditionally without producing stale files on
    // specs that wouldn't normally emit.
    if !explicit_flag && auto_handlers.is_empty() {
        return Ok(());
    }

    let handlers_with_ensures: Vec<&ParsedHandler> = spec
        .handlers
        .iter()
        .filter(|h| !h.ensures.is_empty())
        .collect();

    // No ensures anywhere → nothing to assert. Auto-trigger could still
    // fire (modifies-only fill without ensures is its own lint), but the
    // harness body asserts ensures specifically; skip the file.
    if handlers_with_ensures.is_empty() {
        return Ok(());
    }

    // Restrict per-handler emission to handlers that BOTH have ensures
    // AND either (a) the explicit flag is on OR (b) the handler itself
    // triggers auto-emission. Without (b), a flag-less invocation with
    // one LP-shape handler in a spec full of other handlers would emit
    // a harness for every handler — noise.
    let emit_targets: Vec<&ParsedHandler> = handlers_with_ensures
        .iter()
        .copied()
        .filter(|h| explicit_flag || handler_triggers_impl_harness(h))
        .collect();

    if emit_targets.is_empty() {
        return Ok(());
    }

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    // Body dispatch. The gating above (auto-trigger, ensures-present,
    // emit-targets) is target-agnostic; only the harness body shape
    // differs per framework.
    match target {
        Target::Anchor => emit_kani_impl_anchor(
            spec,
            output_path,
            &emit_targets,
            &auto_handlers,
            explicit_flag,
        ),
        Target::Pinocchio => emit_kani_impl_pinocchio(
            spec,
            output_path,
            &emit_targets,
            &auto_handlers,
            explicit_flag,
        ),
        Target::Quasar => emit_kani_impl_quasar(
            spec,
            output_path,
            &emit_targets,
            &auto_handlers,
            explicit_flag,
        ),
    }
}

/// Emit the Anchor impl-targeted harness: symbolic `Context<X>` +
/// `crate::<Pascal>` accounts struct, calling the user's real handler.
fn emit_kani_impl_anchor(
    spec: &ParsedSpec,
    output_path: &Path,
    emit_targets: &[&ParsedHandler],
    auto_handlers: &[&str],
    explicit_flag: bool,
) -> Result<()> {
    let fp = crate::fingerprint::compute_fingerprint(spec);
    let hash = fp
        .file_hashes
        .get("tests/kani_impl.rs")
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();

    // ── File header ──────────────────────────────────────────────────────
    out.push_str(&crate::banner::banner(None, &hash));
    out.push_str("//\n");
    out.push_str("// Impl-targeted Kani harnesses — call the user's real Anchor handler\n");
    out.push_str("// against a symbolic `Accounts` context and assert the spec's\n");
    out.push_str("// `ensures` clauses against pre/post account-field snapshots.\n");
    out.push_str("//\n");
    out.push_str("// Pairs with `tests/kani.rs` (spec-model harness) — that file checks\n");
    out.push_str("// the spec's effect block satisfies its own ensures; this file checks\n");
    out.push_str("// the user's Rust impl does. A counterexample here blames the impl,\n");
    out.push_str("// not the spec.\n");
    if !explicit_flag {
        out.push_str("//\n");
        out.push_str("// Auto-triggered: the following handlers declare `modifies` fields\n");
        out.push_str("// that are NOT written in their `effect` block (the v2.25 LP-shape\n");
        out.push_str("// signal). The agent-fill `todo!()` site is expected to compute\n");
        out.push_str("// those fields against the spec's ensures; this harness verifies\n");
        out.push_str("// the result.\n");
        for name in auto_handlers {
            out.push_str(&format!("//   - {}\n", name));
        }
        out.push_str("//\n");
        out.push_str("// Pass `--kani-impl` to `qedgen codegen` to force emission for\n");
        out.push_str("// every handler with `ensures`, regardless of the modifies-diff.\n");
    }
    out.push_str("//\n");
    out.push_str("// To run:  cargo kani --harness <name>   (requires cargo-kani)\n");
    out.push_str("// ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----\n");
    out.push_str("#![cfg(kani)]\n\n");

    // ── Symbolic-accounts builder module ────────────────────────────────
    //
    // One `build_<handler>()` per emit target. Each builds a fully-symbolic
    // Anchor accounts context: PDA-derived addresses bind to the spec's
    // declared `pda <name> [seeds]`; account-data fields are `kani::any()`.
    //
    // The shape mirrors what `tests/integration_tests.rs` builds at runtime,
    // but with `kani::any()` substituted for concrete keypair init. The
    // user's handler is called via `accounts.handler(<params>)` — same
    // method signature the integration test invokes.
    emit_symbolic_accounts_module(&mut out, spec, emit_targets, "Anchor")?;

    // ── Per-handler proof harnesses ─────────────────────────────────────
    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Impl-targeted ensures-preservation proofs\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    let mut emitted_count = 0;
    for handler in emit_targets {
        for (idx, ensures) in handler.ensures.iter().enumerate() {
            emit_handler_harness(&mut out, handler, idx, ensures, spec)?;
            emitted_count += 1;
        }
    }

    out.push_str("// ---- GENERATED BY QEDGEN — DO NOT EDIT BELOW THIS LINE ----\n");

    std::fs::write(output_path, &out)?;

    eprintln!(
        "Generated {} impl-targeted Kani harness(es) in {}",
        emitted_count,
        output_path.display()
    );

    Ok(())
}

// ============================================================================
// Quasar impl-targeted harness (slice 5)
// ============================================================================

/// Emit the Quasar impl-targeted harness. The Quasar scaffold emits the
/// user's handler as `impl <Pascal> { pub fn handler(&mut self, …) ->
/// Result<(), ProgramError> }` — byte-identical in shape to the Anchor
/// scaffold — and the `#[program]` mod's `Ctx<X>` dispatcher just forwards
/// `ctx.accounts.handler(…)`. So this harness reuses the same struct-based
/// symbolic-accounts builder (`emit_symbolic_accounts_module`) and
/// per-handler proof emitter (`emit_handler_harness`) as the Anchor
/// branch; only the file header (framework name, placement note, the
/// `Ctx<X>` forwarding context) differs.
///
/// **Placement** (per design doc §11a, target-independent): the harness
/// lives in the program crate's `src/` as `mod kani_impl`, NOT `tests/`.
/// `cargo kani` only discovers `#[kani::proof]` in the lib, and the
/// harness's `crate::<Pascal>` references only resolve from inside the lib
/// crate. The CLI's `redirect_kani_impl_to_src` rewrites the default
/// `…/tests/kani_impl.rs` path to `…/src/kani_impl.rs` for Quasar the same
/// way it does for Pinocchio. Unlike Pinocchio, a Quasar crate is `std`
/// on the host target Kani builds for (its `no_std` cfg gates on
/// `target_os = "solana"`), so no `extern crate kani` is required.
fn emit_kani_impl_quasar(
    spec: &ParsedSpec,
    output_path: &Path,
    emit_targets: &[&ParsedHandler],
    auto_handlers: &[&str],
    explicit_flag: bool,
) -> Result<()> {
    let fp = crate::fingerprint::compute_fingerprint(spec);
    let hash = fp
        .file_hashes
        .get("src/kani_impl.rs")
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();

    // ── File header ──────────────────────────────────────────────────────
    out.push_str(&crate::banner::banner(None, &hash));
    out.push_str("//\n");
    out.push_str("// Impl-targeted Kani harnesses for a Quasar (`#[program]`) program.\n");
    out.push_str("// Calls the user's real handler — the `impl <Pascal> { fn handler\n");
    out.push_str("// (&mut self, …) }` method that the `Ctx<X>` dispatcher forwards to —\n");
    out.push_str("// against a symbolic accounts struct, asserting the spec's `ensures`\n");
    out.push_str("// clauses against pre/post account-field snapshots.\n");
    out.push_str("//\n");
    out.push_str("// Pairs with `tests/kani.rs` (spec-model harness) — that file checks\n");
    out.push_str("// the spec's effect block satisfies its own ensures; this file checks\n");
    out.push_str("// the user's Rust impl does. A counterexample here blames the impl,\n");
    out.push_str("// not the spec.\n");
    out.push_str("//\n");
    out.push_str("// PLACEMENT (per the M1 smoke-test findings, design doc §11a):\n");
    out.push_str("//   1. This file lives in the program crate's `src/` (NOT `tests/`):\n");
    out.push_str("//      `cargo kani` only discovers `#[kani::proof]` in the lib, and\n");
    out.push_str("//      the `crate::<Pascal>` references below only resolve there.\n");
    out.push_str("//   2. Add this line to the crate root (`src/lib.rs`):\n");
    out.push_str("//        #[cfg(kani)] mod kani_impl;\n");
    out.push_str("//      A Quasar crate is `std` on the host target Kani builds for, so\n");
    out.push_str("//      — unlike Pinocchio — no `extern crate kani` is needed.\n");
    if !explicit_flag {
        out.push_str("//\n");
        out.push_str("// Auto-triggered: the following handlers declare `modifies` fields\n");
        out.push_str("// that are NOT written in their `effect` block (the v2.25 LP-shape\n");
        out.push_str("// signal). The agent-fill `todo!()` site is expected to compute\n");
        out.push_str("// those fields against the spec's ensures; this harness verifies\n");
        out.push_str("// the result.\n");
        for name in auto_handlers {
            out.push_str(&format!("//   - {}\n", name));
        }
        out.push_str("//\n");
        out.push_str("// Pass `--kani-impl` to `qedgen codegen` to force emission for\n");
        out.push_str("// every handler with `ensures`, regardless of the modifies-diff.\n");
    }
    out.push_str("//\n");
    out.push_str("// To run:  cargo kani --harness <name>   (requires cargo-kani)\n");
    out.push_str("// ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----\n");
    out.push_str("#![cfg(kani)]\n\n");

    // ── Symbolic-accounts builder module (shared with the Anchor branch) ──
    emit_symbolic_accounts_module(&mut out, spec, emit_targets, "Quasar")?;

    // ── Per-handler proof harnesses (shared with the Anchor branch) ──────
    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Impl-targeted ensures-preservation proofs\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    let mut emitted_count = 0;
    for handler in emit_targets {
        for (idx, ensures) in handler.ensures.iter().enumerate() {
            emit_handler_harness(&mut out, handler, idx, ensures, spec)?;
            emitted_count += 1;
        }
    }

    out.push_str("// ---- GENERATED BY QEDGEN — DO NOT EDIT BELOW THIS LINE ----\n");

    std::fs::write(output_path, &out)?;

    eprintln!(
        "Generated {} impl-targeted Kani harness(es) in {}",
        emitted_count,
        output_path.display()
    );

    Ok(())
}

// ============================================================================
// Pinocchio impl-targeted harness (slice 8 M3)
// ============================================================================

/// Emit the Pinocchio impl-targeted harness. Unlike the Anchor branch
/// (symbolic `Context<X>` + accounts struct), Pinocchio handlers take
/// raw `&[AccountInfo]`, so the harness builds `Account`-layout structs
/// directly on the stack and transmutes pointers into `AccountInfo`.
///
/// The shape is validated by
/// `examples/pinocchio-fixtures/ptoken-transfer/src/kani_impl.rs` (M2),
/// which caught a real token-overflow bug in 1.1s. The design pivots to
/// stack allocation over the wire-format approach that blew BMC budget.
///
/// **Key correctness lever**: Kani's *automatic* arithmetic-overflow /
/// UB checks run on every path through the real handler. The M2 bug
/// fired as "attempt to add with overflow" before any explicit
/// assertion. So the base harness — build symbolic accounts, call the
/// handler — already catches arithmetic bugs without an explicit
/// `ensures` translation. The spec's `ensures` clauses are emitted as
/// reference comments + an optional agent-fill assertion block.
fn emit_kani_impl_pinocchio(
    spec: &ParsedSpec,
    output_path: &Path,
    emit_targets: &[&ParsedHandler],
    auto_handlers: &[&str],
    explicit_flag: bool,
) -> Result<()> {
    let fp = crate::fingerprint::compute_fingerprint(spec);
    let hash = fp
        .file_hashes
        .get("src/kani_impl.rs")
        .cloned()
        .unwrap_or_default();

    let mut out = String::new();

    // ── File header ──────────────────────────────────────────────────────
    out.push_str(&crate::banner::banner(None, &hash));
    out.push_str("//\n");
    out.push_str("// Impl-targeted Kani harnesses for a Pinocchio (#![no_std]) program.\n");
    out.push_str("// Builds symbolic `AccountInfo` values on the stack and calls the\n");
    out.push_str("// user's real `process_<handler>` against them. Kani's automatic\n");
    out.push_str("// overflow / underflow / UB checks run on every path through the\n");
    out.push_str("// real handler — a counterexample blames the impl.\n");
    out.push_str("//\n");
    out.push_str("// PLACEMENT (per the M1 smoke-test findings, design doc §11a):\n");
    out.push_str("//   1. This file lives in the program crate's `src/` (NOT `tests/`):\n");
    out.push_str("//      `cargo kani` only discovers `#[kani::proof]` in the lib, not\n");
    out.push_str("//      in integration tests.\n");
    out.push_str("//   2. Add these two lines to the crate root (`src/lib.rs`):\n");
    out.push_str("//        #[cfg(kani)] extern crate kani;\n");
    out.push_str("//        #[cfg(kani)] mod kani_impl;\n");
    out.push_str("//      The `extern crate kani` is required for `cargo kani` to find\n");
    out.push_str("//      its instrumentation entry in a #![no_std] crate.\n");
    if !explicit_flag {
        out.push_str("//\n");
        out.push_str("// Auto-triggered for handlers with modifies-not-in-effect:\n");
        for name in auto_handlers {
            out.push_str(&format!("//   - {}\n", name));
        }
    }
    out.push_str("//\n");
    out.push_str("// To run:  cargo kani --harness <name>   (requires cargo-kani)\n");
    out.push_str("// ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----\n");
    out.push_str("#![cfg(kani)]\n");
    out.push_str("#![allow(dead_code, unused_imports, unused_variables)]\n\n");

    emit_pinocchio_scaffold(&mut out);

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Per-handler proof harnesses\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    let mut emitted_count = 0;
    for handler in emit_targets {
        emit_pinocchio_handler_harness(&mut out, handler, spec)?;
        emitted_count += 1;
    }

    out.push_str("// ---- GENERATED BY QEDGEN — DO NOT EDIT BELOW THIS LINE ----\n");

    std::fs::write(output_path, &out)?;
    eprintln!(
        "Generated {} Pinocchio impl-targeted Kani harness(es) in {}",
        emitted_count,
        output_path.display()
    );
    Ok(())
}

/// Emit the deterministic shared scaffold: the `Account`-layout mirror,
/// the stack-account container, the SPL Token layout constants, and the
/// build / transmute / read helpers. Byte-for-byte the same regardless
/// of the spec (the per-handler harnesses below reference these).
fn emit_pinocchio_scaffold(out: &mut String) {
    out.push_str(PINOCCHIO_SCAFFOLD);
    out.push('\n');
}

/// The shared scaffold, validated against the M2 reference harness.
const PINOCCHIO_SCAFFOLD: &str = r#"extern crate alloc;

use core::mem::ManuallyDrop;
use pinocchio::account_info::AccountInfo;

/// Layout-mirror of `pinocchio::account_info::Account` (pinocchio 0.8.x).
/// Drift causes immediate UB on first field access; the size assertion
/// catches the common add/remove-field form.
#[repr(C)]
struct AccountLayout {
    borrow_state: u8,
    is_signer: u8,
    is_writable: u8,
    executable: u8,
    original_data_len: u32,
    key: [u8; 32],
    owner: [u8; 32],
    lamports: u64,
    data_len: u64,
}
const _: () = assert!(core::mem::size_of::<AccountLayout>() == 88);

/// 88-byte header followed contiguously by the account's data region.
#[repr(C, align(8))]
struct StackAccount<const DATA_LEN: usize> {
    hdr: AccountLayout,
    data: [u8; DATA_LEN],
}

// SPL Token `TokenAccount` data-region offsets (pinocchio-token 0.3.0).
const TOKEN_OWNER_OFF: usize = 32;
const TOKEN_AMOUNT_OFF: usize = 64;
const TOKEN_STATE_OFF: usize = 108;
const TOKEN_DATA_LEN: usize = 165;

/// SPL Token program ID — the `from_account_info` owner check target.
const SPL_TOKEN_PROGRAM_ID: [u8; 32] = [
    0x06, 0xdd, 0xf6, 0xe1, 0xd7, 0x65, 0xa1, 0x93, 0xd9, 0xcb, 0xe1, 0x46, 0xce, 0xeb, 0x79,
    0xac, 0x1c, 0xb4, 0x85, 0xed, 0x5f, 0x5b, 0x37, 0x91, 0x3a, 0x8c, 0xf5, 0x85, 0x7e, 0xff,
    0x00, 0xa9,
];
const STATE_INITIALIZED: u8 = 1;
const BORROW_STATE_CLEAR: u8 = 0;

/// Build a stack-resident SPL Token account. `amount` is the field a
/// harness wires up as `kani::any()`.
fn build_token_account(
    key: [u8; 32],
    is_writable: bool,
    is_signer: bool,
    owner_in_data: [u8; 32],
    amount: u64,
) -> StackAccount<TOKEN_DATA_LEN> {
    let mut acct = StackAccount {
        hdr: AccountLayout {
            borrow_state: BORROW_STATE_CLEAR,
            is_signer: is_signer as u8,
            is_writable: is_writable as u8,
            executable: 0,
            original_data_len: 0,
            key,
            owner: SPL_TOKEN_PROGRAM_ID,
            lamports: 0,
            data_len: TOKEN_DATA_LEN as u64,
        },
        data: [0u8; TOKEN_DATA_LEN],
    };
    acct.data[TOKEN_OWNER_OFF..TOKEN_OWNER_OFF + 32].copy_from_slice(&owner_in_data);
    acct.data[TOKEN_AMOUNT_OFF..TOKEN_AMOUNT_OFF + 8].copy_from_slice(&amount.to_le_bytes());
    acct.data[TOKEN_STATE_OFF] = STATE_INITIALIZED;
    acct
}

/// Build a non-token account (mint / authority / signer slot). No data
/// region — the handler only reads `is_signer` / `key`.
fn build_minimal_account(key: [u8; 32], is_signer: bool, is_writable: bool) -> StackAccount<0> {
    StackAccount {
        hdr: AccountLayout {
            borrow_state: BORROW_STATE_CLEAR,
            is_signer: is_signer as u8,
            is_writable: is_writable as u8,
            executable: 0,
            original_data_len: 0,
            key,
            owner: [0u8; 32],
            lamports: 0,
            data_len: 0,
        },
        data: [],
    }
}

/// Transmute a `*mut StackAccount<N>::hdr` to `AccountInfo`.
///
/// SAFETY: `AccountInfo` is `#[repr(C)] struct { raw: *mut Account }` —
/// a single-field pointer wrapper. `StackAccount<N>::hdr` mirrors
/// `Account`'s layout (asserted above). The caller must keep `stack`
/// alive for the lifetime of the returned `AccountInfo`.
unsafe fn account_info_from_stack<const N: usize>(stack: &mut StackAccount<N>) -> AccountInfo {
    let hdr_ptr: *mut AccountLayout = &mut stack.hdr;
    core::mem::transmute::<*mut AccountLayout, AccountInfo>(hdr_ptr)
}

/// Read the `amount` from a stack token account's data region.
fn read_token_amount<const N: usize>(stack: &StackAccount<N>) -> u64 {
    u64::from_le_bytes(
        stack.data[TOKEN_AMOUNT_OFF..TOKEN_AMOUNT_OFF + 8]
            .try_into()
            .unwrap(),
    )
}
"#;

/// Normalize a spec handler name into `(module, fn_name)` for the
/// `crate::<module>::<fn_name>` call path. Pinocchio convention is
/// `src/<module>.rs` exposing `pub fn process_<base>`. We strip a
/// leading `process_` from the spec name to recover the base, then
/// reattach it for the fn while using the base as the module.
///
///   "transfer"          → ("transfer", "process_transfer")
///   "process_transfer"  → ("transfer", "process_transfer")
fn pinocchio_call_path(handler_name: &str) -> (String, String) {
    let snake = to_snake_case(handler_name);
    let base = snake.strip_prefix("process_").unwrap_or(&snake).to_string();
    let fn_name = format!("process_{}", base);
    (base, fn_name)
}

/// Rust primitive for a numeric DSL type, used to declare the symbolic
/// param so `to_le_bytes()` packs the right width. Returns None for
/// non-numeric / unsupported types (the harness emits a `todo!()` for
/// those params).
fn numeric_param_rust_type(dsl_type: &str) -> Option<&'static str> {
    match dsl_type {
        "U8" => Some("u8"),
        "I8" => Some("i8"),
        "U16" => Some("u16"),
        "I16" => Some("i16"),
        "U32" => Some("u32"),
        "I32" => Some("i32"),
        "U64" => Some("u64"),
        "I64" => Some("i64"),
        "U128" => Some("u128"),
        "I128" => Some("i128"),
        _ => None,
    }
}

/// Emit one `#[kani::proof]` harness for a Pinocchio handler. Builds
/// symbolic stack accounts from the handler's `accounts {}` block,
/// packs symbolic params into instruction data, and calls the real
/// `process_<name>`. Kani's automatic overflow / UB checks do the
/// verification; spec `ensures` clauses are emitted as reference
/// comments.
fn emit_pinocchio_handler_harness(
    out: &mut String,
    handler: &ParsedHandler,
    _spec: &ParsedSpec,
) -> Result<()> {
    let (module, fn_name) = pinocchio_call_path(&handler.name);
    let snake = to_snake_case(&handler.name);

    // Classify accounts: writable non-signer → SPL Token account (holds
    // mutable state); everything else → minimal account. Heuristic;
    // documented so the user can adjust if their program shape differs.
    let is_token = |a: &ParsedHandlerAccount| a.is_writable && !a.is_signer;

    // The first signer's key threads through as the token-account owner
    // so the handler's owner check passes. Falls back to a fixed key.
    let authority_idx = handler.accounts.iter().position(|a| a.is_signer);

    out.push_str(&format!(
        "/// Impl-targeted harness for `{}`. Kani's automatic\n",
        fn_name
    ));
    out.push_str("/// overflow / underflow / UB checks run on every path through the\n");
    out.push_str("/// real handler. `#[kani::unwind(34)]` bounds the 32-byte Pubkey\n");
    out.push_str("/// memcmp loops in pinocchio's owner checks (+2 slack).\n");
    out.push_str("#[kani::proof]\n");
    out.push_str("#[kani::unwind(34)]\n");
    out.push_str(&format!("fn verify_{}_impl() {{\n", snake));

    // Authority key (concrete) — threaded as token owner.
    out.push_str("    let authority_key: [u8; 32] = [7u8; 32];\n");

    // Symbolic amounts for token accounts.
    for (i, a) in handler.accounts.iter().enumerate() {
        if is_token(a) {
            out.push_str(&format!(
                "    let {}_amount: u64 = kani::any();\n",
                to_snake_case(&a.name)
            ));
        }
        let _ = i;
    }

    // Symbolic params, declared with the primitive matching the spec
    // type so `to_le_bytes()` packs the right width below.
    for (pname, ptype) in &handler.takes_params {
        match numeric_param_rust_type(ptype) {
            Some(rust_ty) => {
                out.push_str(&format!(
                    "    let {}: {} = kani::any(); // spec type: {}\n",
                    to_snake_case(pname),
                    rust_ty,
                    ptype
                ));
            }
            None => {
                out.push_str(&format!(
                    "    // TODO: declare symbolic param `{}` (spec type {})\n",
                    pname, ptype
                ));
            }
        }
    }
    out.push('\n');

    // Build accounts in declared order.
    let mut acct_idents: Vec<String> = Vec::with_capacity(handler.accounts.len());
    for (i, a) in handler.accounts.iter().enumerate() {
        let ident = to_snake_case(&a.name);
        acct_idents.push(ident.clone());
        let key_byte = (i + 1) as u8;
        if is_token(a) {
            // owner_in_data: authority's key for the slot the authority
            // signs for (heuristic: the first token account), else junk.
            let owner_expr = if authority_idx.is_some() && i == 0 {
                "authority_key".to_string()
            } else {
                "[9u8; 32]".to_string()
            };
            out.push_str(&format!(
                "    let mut {ident} = build_token_account([{key_byte}u8; 32], {writable}, {signer}, {owner_expr}, {ident}_amount);\n",
                ident = ident,
                key_byte = key_byte,
                writable = a.is_writable,
                signer = a.is_signer,
                owner_expr = owner_expr,
            ));
        } else {
            let key_expr = if a.is_signer {
                "authority_key".to_string()
            } else {
                format!("[{}u8; 32]", key_byte)
            };
            out.push_str(&format!(
                "    let mut {ident} = build_minimal_account({key_expr}, {signer}, {writable});\n",
                ident = ident,
                key_expr = key_expr,
                signer = a.is_signer,
                writable = a.is_writable,
            ));
        }
    }
    out.push('\n');

    // Assemble the AccountInfo array.
    let n = handler.accounts.len();
    out.push_str(&format!(
        "    let accounts: [ManuallyDrop<AccountInfo>; {n}] = unsafe {{\n        [\n"
    ));
    for ident in &acct_idents {
        out.push_str(&format!(
            "            ManuallyDrop::new(account_info_from_stack(&mut {ident})),\n"
        ));
    }
    out.push_str("        ]\n    };\n");
    out.push_str(&format!(
        "    let accounts_slice: &[AccountInfo] = unsafe {{\n        core::slice::from_raw_parts(&accounts as *const _ as *const AccountInfo, {n})\n    }};\n\n"
    ));

    // Pack instruction data from params.
    out.push_str("    let mut instruction_data = alloc::vec::Vec::new();\n");
    for (pname, ptype) in &handler.takes_params {
        match numeric_param_rust_type(ptype) {
            Some(_) => {
                out.push_str(&format!(
                    "    instruction_data.extend_from_slice(&{}.to_le_bytes());\n",
                    to_snake_case(pname)
                ));
            }
            None => {
                out.push_str(&format!(
                    "    // TODO: pack param `{}` (spec type {}) into instruction_data\n",
                    pname, ptype
                ));
            }
        }
    }
    out.push('\n');

    // Call the real handler.
    out.push_str("    // Call the user's real handler. Kani's automatic checks\n");
    out.push_str("    // (overflow / underflow / pointer UB) verify this path.\n");
    out.push_str(&format!(
        "    let _result = crate::{module}::{fn_name}(accounts_slice, &instruction_data);\n",
        module = module,
        fn_name = fn_name,
    ));

    // Reference: spec ensures clauses. Kani's built-in checks already
    // cover arithmetic UB; explicit cross-field invariant assertions go
    // here if the ensures expresses something Kani can't infer.
    if !handler.ensures.is_empty() {
        out.push('\n');
        out.push_str("    // Spec ensures (reference). Kani's automatic checks cover\n");
        out.push_str("    // arithmetic UB; add explicit assertions below for cross-field\n");
        out.push_str("    // invariants (read post-state via read_token_amount(&<acct>)).\n");
        for e in &handler.ensures {
            out.push_str(&format!("    //   ensures {}\n", e.rust_expr_binary.trim()));
        }
    }

    out.push_str("}\n\n");
    Ok(())
}

/// Emit `mod symbolic_accounts { ... }` with one `build_<handler>` ctor per
/// emit target. The body is a `todo!()` skeleton that lists each account
/// field with its derivation rule (PDA seed expression vs `kani::any()`)
/// as inline comments. The agent (or user) replaces the body with the
/// concrete `crate::<HandlerPascal> { ... }` construction.
fn emit_symbolic_accounts_module(
    out: &mut String,
    spec: &ParsedSpec,
    targets: &[&ParsedHandler],
    framework: &str,
) -> Result<()> {
    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str(&format!(
        "// Symbolic {} `Accounts` context builders.\n",
        framework
    ));
    out.push_str("//\n");
    out.push_str("// Each ctor returns a context with:\n");
    out.push_str("//   - PDA-derived pubkeys computed from the spec's `pda` declarations\n");
    out.push_str("//   - `kani::any()` for non-PDA addresses + account-data fields\n");
    out.push_str("//   - Well-known program IDs for `token_program`, `system_program`, etc.\n");
    out.push_str("//\n");
    out.push_str("// The ctors are AGENT-FILL skeletons: the data-bearing fields\n");
    out.push_str("// (state struct contents, token amounts, mints) get populated to\n");
    out.push_str("// match the user's handler signature. Without that fill, the file\n");
    out.push_str("// won't compile — by design — so it surfaces as a `todo!()` to address.\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    out.push_str("mod symbolic_accounts {\n");
    out.push_str(&format!(
        "    // The user's program crate is the host for this harness. {}\n",
        framework
    ));
    out.push_str("    // re-exports `#[derive(Accounts)]` structs at crate root via\n");
    out.push_str("    // `#[program]`, so the handler's accounts struct resolves via\n");
    out.push_str("    // `crate::<HandlerPascal>`.\n");
    out.push_str("    #![allow(unused_imports, dead_code)]\n");

    for handler in targets {
        emit_symbolic_accounts_ctor(out, handler, spec, framework)?;
    }

    out.push_str("} // mod symbolic_accounts\n\n");
    Ok(())
}

/// Emit a single `pub fn build_<handler>() -> crate::<Pascal>` constructor.
/// Body is a `todo!()` skeleton with per-account-field derivation comments
/// for agent fill-in.
fn emit_symbolic_accounts_ctor(
    out: &mut String,
    handler: &ParsedHandler,
    spec: &ParsedSpec,
    framework: &str,
) -> Result<()> {
    let pascal = to_pascal_case(&handler.name);
    out.push_str(&format!(
        "\n    /// Symbolic `Accounts` context for the user's `{}` handler.\n",
        handler.name
    ));
    out.push_str("    ///\n");
    out.push_str("    /// AGENT-FILL: replace the `todo!()` body with the concrete\n");
    out.push_str("    /// construction. Each account field is annotated with its\n");
    out.push_str("    /// derivation rule below.\n");
    out.push_str(&format!(
        "    pub fn build_{}() -> crate::{} {{\n",
        handler.name, pascal
    ));

    if handler.accounts.is_empty() {
        if handler.who.is_some() {
            out.push_str(
                "        // No explicit accounts; spec declares an `auth` actor → signer.\n",
            );
            out.push_str(&format!(
                "        todo!(\"Construct crate::{} with a symbolic signer\")\n",
                pascal
            ));
        } else {
            out.push_str("        // No accounts declared on this handler.\n");
            out.push_str(&format!(
                "        todo!(\"Construct crate::{} with the handler's account context\")\n",
                pascal
            ));
        }
    } else {
        for acct in &handler.accounts {
            emit_account_field_skeleton(out, acct, handler, spec);
        }
        out.push_str("        //\n");
        out.push_str("        // AGENT: assemble the fields above into the concrete\n");
        out.push_str(&format!(
            "        // `crate::{}` struct. The {} `#[derive(Accounts)]`\n",
            pascal, framework
        ));
        out.push_str("        // expansion gives the exact field layout.\n");
        out.push_str(&format!("        todo!(\"assemble crate::{}\")\n", pascal));
    }

    out.push_str("    }\n");
    Ok(())
}

/// `true` for the DSL's integer scalar types — these serialize to a seed
/// via `to_le_bytes()` rather than `as_ref()`.
fn is_integer_dsl_type(ty: &str) -> bool {
    matches!(
        ty,
        "U8" | "U16" | "U32" | "U64" | "U128" | "I8" | "I16" | "I32" | "I64" | "I128" | "Nat"
    )
}

/// Emit one commented-out line per account field with its derivation rule.
/// PDA-bound accounts get a `Pubkey::find_program_address` template using
/// the spec's `pda <name> [seeds]` declaration; non-PDA fields default to
/// `kani::any()`; programs use their well-known IDs.
fn emit_account_field_skeleton(
    out: &mut String,
    acct: &crate::check::ParsedHandlerAccount,
    handler: &ParsedHandler,
    spec: &ParsedSpec,
) {
    if acct.is_program {
        out.push_str(&format!(
            "        // `{}`: well-known program ID (e.g. token / system / rent)\n",
            acct.name
        ));
        return;
    }
    if let Some(seeds) = &acct.pda_seeds {
        // Prefer the top-level `pda <name> [seeds]` declaration when it
        // matches by name; fall back to the inline seeds otherwise.
        let pda_seeds: Vec<String> = spec
            .pdas
            .iter()
            .find(|p| p.name == acct.name)
            .map(|p| p.seeds.clone())
            .unwrap_or_else(|| seeds.clone());
        let seed_exprs: Vec<String> = pda_seeds
            .iter()
            .map(|s| {
                if (s.starts_with('"') && s.ends_with('"'))
                    || (s.starts_with('\'') && s.ends_with('\''))
                {
                    let inner = &s[1..s.len() - 1];
                    format!("b\"{}\"", inner)
                } else if handler
                    .takes_params
                    .iter()
                    .any(|(n, t)| n == s && is_integer_dsl_type(t))
                {
                    // An integer handler param used as a seed must be
                    // serialized to bytes — `u64::as_ref()` doesn't exist.
                    // (`Pubkey` params / account keys keep `.as_ref()`.)
                    format!("{}.to_le_bytes().as_ref()", s)
                } else {
                    format!("{}.as_ref()", s)
                }
            })
            .collect();
        out.push_str(&format!(
            "        // `{}`: PDA derived from `[{}]`\n",
            acct.name,
            seed_exprs.join(", ")
        ));
        out.push_str(&format!(
            "        //   let ({0}_key, _bump) = solana_program::pubkey::Pubkey::find_program_address(&[{1}], &crate::ID);\n",
            acct.name,
            seed_exprs.join(", ")
        ));
        return;
    }
    if acct.is_signer {
        out.push_str(&format!(
            "        // `{}`: signer — symbolic address via `kani::any()`\n",
            acct.name
        ));
        return;
    }
    out.push_str(&format!(
        "        // `{}`: non-PDA account — symbolic address + data via `kani::any()`\n",
        acct.name
    ));
}

/// Emit one `#[kani::proof]` for a (handler, ensures) pair. Shape:
///   1. Build symbolic accounts context via the `symbolic_accounts` module.
///   2. Snapshot pre-state fields (the modifies set, plus any field the
///      ensures' `rust_expr_binary` reads via `pre.<field>`).
///   3. Declare symbolic params + `kani::assume` the handler's requires.
///   4. Call the user's real handler method.
///   5. On `Ok`, snapshot post-state fields, splice CPI ensures-as-fact
///      `kani::assume` lines for each `call Iface.foo(...)` whose callee
///      declares ensures (Track I), then assert the caller's own ensures.
fn emit_handler_harness(
    out: &mut String,
    handler: &ParsedHandler,
    idx: usize,
    ensures: &crate::check::ParsedEnsures,
    spec: &ParsedSpec,
) -> Result<()> {
    out.push_str("#[kani::proof]\n");
    out.push_str("#[kani::unwind(2)]\n");
    out.push_str("#[kani::solver(cadical)]\n");
    out.push_str(&format!(
        "fn verify_{}_impl_ensures_{}() {{\n",
        handler.name, idx
    ));

    // 1. Build the symbolic accounts context.
    out.push_str(&format!(
        "    let mut accounts = symbolic_accounts::build_{}();\n",
        handler.name
    ));

    // 2. Pre-snapshot. Snapshot every field the ensures clause may compare
    //    across the call (union of `modifies` and effect-LHS bare field
    //    names). Path is `accounts.<state_account>.<field>` when the
    //    state account is uniquely identifiable; otherwise the snapshot
    //    falls back to a `todo!()` placeholder for the agent.
    let state_acct = find_state_account_name(handler);
    let snapshot_fields = collect_snapshot_fields(handler);
    if !snapshot_fields.is_empty() {
        out.push_str(
            "    // Pre-state snapshot — fields the ensures clause reads via `pre.<x>`.\n",
        );
        for field in &snapshot_fields {
            match state_acct {
                Some(acct) => {
                    out.push_str(&format!(
                        "    let pre_{0} = accounts.{1}.{0};\n",
                        field, acct
                    ));
                }
                None => {
                    out.push_str(&format!(
                        "    let pre_{0} = todo!(\"snapshot pre.{0} from the symbolic accounts context\");\n",
                        field
                    ));
                }
            }
        }
    }

    // 3. Symbolic params + preconditions.
    for (pname, ptype) in &handler.takes_params {
        out.push_str(&format!(
            "    let {}: {} = kani::any();\n",
            pname,
            map_type(ptype, spec)?
        ));
    }
    // Apply the handler's `requires` clauses as Kani assumptions so we
    // explore inputs the user's handler would actually accept (otherwise
    // it returns Err and the ensures don't fire — vacuous pass).
    if let Some(full_guard) = crate::rust_codegen_util::collect_full_guard(handler, false) {
        out.push_str(&format!("    kani::assume({});\n", full_guard));
    }

    // 4. Call the user's real handler. Anchor handler methods take
    //    `&mut self` and the param list — same shape `cargo build`
    //    expands `#[derive(Accounts)]` + `#[program]` into.
    let args: String = handler
        .takes_params
        .iter()
        .map(|(n, _)| n.as_str())
        .collect::<Vec<_>>()
        .join(", ");
    out.push_str(&format!("    let result = accounts.handler({});\n", args));

    // 5. Post-snapshot + assertion. The Track I splice point sits between
    //    `if result.is_ok()` and the `assert!` so CPI ensures can be
    //    layered in as `kani::assume` facts.
    out.push_str("    if result.is_ok() {\n");
    if !snapshot_fields.is_empty() {
        out.push_str(
            "        // Post-state snapshot — same fields, read from post-call accounts.\n",
        );
        for field in &snapshot_fields {
            match state_acct {
                Some(acct) => {
                    out.push_str(&format!(
                        "        let post_{0} = accounts.{1}.{0};\n",
                        field, acct
                    ));
                }
                None => {
                    out.push_str(&format!(
                        "        let post_{0} = todo!(\"snapshot post.{0} from the symbolic accounts context\");\n",
                        field
                    ));
                }
            }
        }
    }

    // ── CPI ensures-as-fact (Track I) ──────────────────────────────────
    // For every `call Iface.foo(args)` site whose callee declares its own
    // `ensures`, splice a `kani::assume(<callee_ensures, substituted>)`
    // line so the caller's downstream assert! can rely on the CPI's
    // contract. Tier-0 callees (no ensures declared) emit nothing —
    // matching the spec-model harness behavior in `kani.rs` and the
    // `lean_gen.rs::render_cpi_theorems` `:= by sorry` fallback.
    //
    // The substituted clauses come back in `pre.X` / `post.X` form (from
    // `rust_expr_binary`); we flatten those to the harness-local
    // `pre_X` / `post_X` snapshots via the same `rewrite_pre_post_paths`
    // helper used on the caller's own ensures below.
    emit_cpi_ensures_as_assume(out, handler, spec);

    // The ensures clause's `rust_expr_binary` uses `pre.<field>` and
    // `post.<field>` paths. Our snapshots are flat `pre_<field>` /
    // `post_<field>` locals (no struct), so we rewrite the path
    // separators. The chumsky_adapter renders `state.x` / `old(state.x)`
    // into exactly `post.x` / `pre.x` — no other source produces these
    // tokens in `rust_expr_binary`, so a string-replace is safe.
    let lowered = rewrite_pre_post_paths(&ensures.rust_expr_binary);
    out.push_str(&format!("        assert!(\n            {},\n", lowered));
    out.push_str(&format!(
        "            \"ensures clause {} on {} (impl) violated\"\n",
        idx, handler.name
    ));
    out.push_str("        );\n");
    out.push_str("    }\n");
    out.push_str("}\n\n");
    Ok(())
}

/// Walk `handler.calls` and, for each CPI whose callee declares ensures,
/// emit a `// CPI ensures-as-fact (Iface.handler):` comment followed by one
/// `kani::assume(<substituted_clause>);` per ensures clause. Tier-0 callees
/// (empty ensures) emit nothing — same fallback as the spec-model harness
/// in `kani.rs` and `lean_gen.rs::render_cpi_theorems`'s `:= by sorry`.
///
/// Substitution reuses `crate::cpi_substitute::substitute_callee_ensures_rust_binary`
/// — the same helper the spec-model harness uses, so the two backends
/// agree on the `let X = call ...` `result` convention and word-boundary
/// param matching. After substitution we apply `rewrite_pre_post_paths`
/// (same transformation step the caller's own `assert!` emission uses)
/// to flatten `pre.X` / `post.X` paths to the harness-local
/// `pre_X` / `post_X` snapshots.
///
/// **Track J breadcrumb**: when `check::multi_cpi_shared_fields` reports any
/// shared `pre.X` / `post.X` reference across two callees of this handler,
/// emit a WARNING comment above the assume block. The lint
/// `multi_cpi_same_field` carries the structured guidance; this is a
/// reader-of-generated-code breadcrumb so the harness itself flags the
/// over-constraint risk without the user needing to cross-reference the
/// lint output.
fn emit_cpi_ensures_as_assume(out: &mut String, handler: &ParsedHandler, spec: &ParsedSpec) {
    // Track J — emit the breadcrumb once, above the entire CPI assume block,
    // when the lint predicate fires for this handler.
    let shared = check::multi_cpi_shared_fields(spec, handler);
    if !shared.is_empty() {
        out.push_str("        // WARNING: multi-CPI ordering — this handler has ≥2 calls whose\n");
        out.push_str(
            "        // ensures reference the same caller-state field. Both kani::assume\n",
        );
        out.push_str("        // lines fire at the same splice point against one (pre, post)\n");
        out.push_str("        // snapshot pair, which may over-constrain. See lint\n");
        out.push_str("        // `multi_cpi_same_field` for context.\n");
    }
    for call in &handler.calls {
        let Some(iface) = spec
            .interfaces
            .iter()
            .find(|i| i.name == call.target_interface)
        else {
            continue;
        };
        let Some(callee) = iface
            .handlers
            .iter()
            .find(|h| h.name == call.target_handler)
        else {
            continue;
        };
        if callee.ensures.is_empty() {
            // Tier-0 callee — `cpi_no_callee_ensures` lint surfaces the gap.
            continue;
        }
        out.push_str(&format!(
            "        // CPI ensures-as-fact ({}.{}):\n",
            call.target_interface, call.target_handler,
        ));
        for callee_ens in &callee.ensures {
            let substituted = crate::cpi_substitute::substitute_callee_ensures_rust_binary(
                &callee_ens.rust_expr_binary,
                call,
                &callee.params,
                // v2.26 Track K — propagate the declared return-binder
                // name. `None` keeps the literal "result" convention.
                callee.result_binder.as_deref(),
            );
            let lowered = rewrite_pre_post_paths(&substituted);
            out.push_str(&format!("        kani::assume({});\n", lowered));
        }
    }
}

/// Find the handler's writable state account by name. v2.26 Slice 1 uses a
/// simple heuristic: the unique writable non-program, non-signer, non-token,
/// non-mint account. Matches the integration_test scaffolding convention
/// (the program's state PDA is the canonical "state" account; signers /
/// mints / token accounts are separate). Returns `None` when the heuristic
/// can't pick a unique state account — the harness then emits per-field
/// `todo!()` snapshot placeholders for the agent to resolve.
fn find_state_account_name(handler: &ParsedHandler) -> Option<&str> {
    let candidates: Vec<&crate::check::ParsedHandlerAccount> = handler
        .accounts
        .iter()
        .filter(|a| {
            a.is_writable
                && !a.is_program
                && !a.is_signer
                && a.account_type.as_deref() != Some("token")
                && a.account_type.as_deref() != Some("mint")
        })
        .collect();
    if candidates.len() == 1 {
        Some(candidates[0].name.as_str())
    } else {
        None
    }
}

/// The union of `modifies`, effect-LHS bare field names, and v2.27
/// Track A state-binder caller fields — every field the ensures clause
/// might read across the pre/post boundary. Used to drive snapshot
/// emission.
///
/// Track A: when a `call X.y(state_binders { from_balance = state.X })`
/// is present, the CPI assume splice references `pre.X` / `post.X`
/// (the substitution rewrote `pre.from_balance` → `pre.X`). The
/// `rewrite_pre_post_paths` flatten then turns those into `pre_X` /
/// `post_X` locals — which only exist if the snapshot emitter
/// captured `X`. Including binder caller fields here closes that loop.
fn collect_snapshot_fields(handler: &ParsedHandler) -> Vec<String> {
    let mut fields: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    if let Some(modifies) = &handler.modifies {
        for f in modifies {
            fields.insert(f.clone());
        }
    }
    for (lhs, _, _) in &handler.effects {
        let bare = crate::rust_codegen_util::effect_target_base(lhs);
        fields.insert(bare.to_string());
    }
    // v2.27 Track A — pick up every caller-side field referenced by a
    // state binder on any CPI call in this handler. Without this the
    // CPI assume splice would reference `pre_X` / `post_X` locals the
    // snapshot block never declared, producing a compile error in the
    // generated harness.
    for call in &handler.calls {
        for binder in &call.state_binders {
            fields.insert(binder.caller_field.clone());
        }
    }
    fields.into_iter().collect()
}

/// Rewrite `pre.<field>` → `pre_<field>` and `post.<field>` → `post_<field>`
/// in the rendered ensures expression. The chumsky_adapter renders
/// `state.x` / `old(state.x)` into exactly `post.x` / `pre.x` in the
/// binary-mode form — no other source produces these tokens — so a plain
/// string replace is safe.
fn rewrite_pre_post_paths(expr: &str) -> String {
    expr.replace("pre.", "pre_").replace("post.", "post_")
}

// ============================================================================
// Tests
// ============================================================================

#[cfg(test)]
mod tests {
    use super::*;
    use crate::chumsky_adapter::parse_str;

    /// Auto-trigger fires when a handler has `modifies` listing a field
    /// that's absent from the effect block's LHS set (the LP-deposit
    /// shape).
    #[test]
    fn auto_trigger_fires_on_lp_shape() {
        let src = r#"spec Pool
state { pool_balance : U64, lp_supply : U64 }
handler deposit (amount : U64) {
  requires amount > 0 else InvalidAmount
  modifies [pool_balance, lp_supply]
  ensures state.pool_balance == old(state.pool_balance) + amount
  effect {
    pool_balance += amount
  }
}"#;
        let spec = parse_str(src).expect("parse");
        let h = &spec.handlers[0];
        assert!(
            handler_triggers_impl_harness(h),
            "modifies = [pool_balance, lp_supply] but effect only writes pool_balance → trigger",
        );
        assert!(spec_triggers_impl_harness(&spec));
    }

    /// Auto-trigger does NOT fire when modifies matches the effect-LHS
    /// set (no LP-shape gap — the spec's effect block covers every
    /// declared write).
    #[test]
    fn auto_trigger_silent_when_modifies_matches_effects() {
        let src = r#"spec Counter
state { count : U64 }
handler bump (delta : U64) {
  requires delta > 0 else InvalidAmount
  modifies [count]
  ensures state.count == old(state.count) + delta
  effect {
    count += delta
  }
}"#;
        let spec = parse_str(src).expect("parse");
        let h = &spec.handlers[0];
        assert!(
            !handler_triggers_impl_harness(h),
            "modifies = [count] = effect LHS = {{count}} → no trigger",
        );
        assert!(!spec_triggers_impl_harness(&spec));
    }

    /// Auto-trigger silent when no `modifies` clause is declared at all.
    /// Bundled examples today take this path.
    #[test]
    fn auto_trigger_silent_without_modifies() {
        let src = r#"spec NoModifies
state { x : U64 }
handler set_x (v : U64) {
  ensures state.x == v
  effect { x := v }
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(!spec_triggers_impl_harness(&spec));
    }

    /// Slice 5: Quasar emits the struct-based impl harness, reusing the
    /// Anchor symbolic-accounts builder + per-handler proof emitter (the
    /// Quasar scaffold's `Ctx<X>` dispatcher forwards to the same
    /// `impl <Pascal> { fn handler(&mut self, …) }` method). The header
    /// must be Quasar-flavored and must NOT leak the Anchor framework
    /// crates or the Pinocchio stack scaffold.
    #[test]
    fn quasar_target_emits_handler_harness() {
        let src = r#"spec QuasarBump
state { x : U64 }
handler bump (delta : U64) {
  ensures state.x == old(state.x) + delta
  effect { x += delta }
}"#;
        let spec = parse_str(src).expect("parse");

        let tmp = std::env::temp_dir().join(format!("kani_impl_quasar_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ true, Target::Quasar)
            .expect("Quasar kani_impl must emit");
        assert!(tmp.is_file(), "Quasar target must write a harness file");
        let body = std::fs::read_to_string(&tmp).unwrap();

        // Quasar-flavored header.
        assert!(
            body.contains("Quasar (`#[program]`) program"),
            "header must name the Quasar framework; got:\n{body}"
        );
        assert!(
            body.contains("#[cfg(kani)] mod kani_impl;"),
            "header must document the src/lib.rs placement line; got:\n{body}"
        );

        // Struct-based shape, shared with Anchor.
        assert!(
            body.contains("mod symbolic_accounts {")
                && body.contains("pub fn build_bump() -> crate::Bump"),
            "must emit the symbolic accounts builder for crate::Bump; got:\n{body}"
        );
        assert!(
            body.contains("fn verify_bump_impl_ensures_0()")
                && body.contains("accounts.handler(delta)"),
            "must emit the per-handler proof calling the real .handler(); got:\n{body}"
        );

        // The shared module comment must say "Quasar", not "Anchor".
        assert!(
            body.contains("host for this harness. Quasar"),
            "symbolic_accounts comment must be Quasar-flavored; got:\n{body}"
        );

        // Must NOT leak the word "Anchor" anywhere — the framework label
        // threads through every shared-emitter comment.
        assert!(
            !body.contains("Anchor"),
            "Quasar harness must not leak the Anchor framework name; got:\n{body}"
        );
        assert!(
            !body.contains("struct AccountLayout") && !body.contains("build_token_account"),
            "Quasar harness must not emit the Pinocchio stack scaffold; got:\n{body}"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// Slice 8 M3: Pinocchio emits a stack-allocated `AccountInfo`
    /// harness. Validates the deterministic scaffold + per-handler
    /// proof shape that the M2 reference
    /// (examples/pinocchio-fixtures/ptoken-transfer/src/kani_impl.rs)
    /// proved catches real overflow bugs.
    #[test]
    fn pinocchio_target_emits_stack_harness() {
        // SPL-transfer-shaped handler: two writable token accounts
        // (source, destination), a readonly mint, a signer authority.
        let src = r#"spec PtokenTransfer
state { dummy : U64 }
handler transfer (amount : U64) {
  accounts {
    source : writable
    mint : readonly
    destination : writable
    authority : signer
  }
  ensures state.dummy == old(state.dummy)
  effect { dummy := dummy }
}"#;
        let spec = parse_str(src).expect("parse");

        let tmp =
            std::env::temp_dir().join(format!("kani_impl_pinocchio_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ true, Target::Pinocchio)
            .expect("Pinocchio kani_impl must emit");
        assert!(tmp.is_file(), "Pinocchio target must write a harness file");
        let body = std::fs::read_to_string(&tmp).unwrap();

        // Deterministic scaffold present.
        assert!(
            body.contains("struct AccountLayout"),
            "must emit the Account layout mirror; got:\n{body}"
        );
        assert!(
            body.contains("assert!(core::mem::size_of::<AccountLayout>() == 88)"),
            "must emit the layout size assertion; got:\n{body}"
        );
        assert!(
            body.contains("fn build_token_account")
                && body.contains("fn build_minimal_account")
                && body.contains("fn account_info_from_stack"),
            "must emit the build + transmute helpers; got:\n{body}"
        );

        // Per-handler proof.
        assert!(
            body.contains("#[kani::proof]") && body.contains("#[kani::unwind(34)]"),
            "must emit the proof attribute + memcmp unwind bound; got:\n{body}"
        );
        assert!(
            body.contains("fn verify_transfer_impl()"),
            "must emit the per-handler proof fn; got:\n{body}"
        );

        // Account classification: writable non-signer → token account;
        // signer/readonly → minimal.
        assert!(
            body.contains("let mut source = build_token_account(")
                && body.contains("let mut destination = build_token_account("),
            "writable non-signer accounts must build as token accounts; got:\n{body}"
        );
        assert!(
            body.contains("let mut mint = build_minimal_account(")
                && body.contains("let mut authority = build_minimal_account("),
            "readonly + signer accounts must build as minimal accounts; got:\n{body}"
        );

        // Param packing + real handler call.
        assert!(
            body.contains("let amount: u64 = kani::any();")
                && body.contains("instruction_data.extend_from_slice(&amount.to_le_bytes());"),
            "U64 param must be symbolic + LE-packed; got:\n{body}"
        );
        assert!(
            body.contains("crate::transfer::process_transfer(accounts_slice, &instruction_data)"),
            "must call the real handler at crate::transfer::process_transfer; got:\n{body}"
        );

        // Must NOT leak the Anchor shape.
        assert!(
            !body.contains("Context<") && !body.contains("symbolic_accounts"),
            "Pinocchio harness must not leak the Anchor Context shape; got:\n{body}"
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// `--kani-impl` flag explicitly forces emission for every handler
    /// with ensures, regardless of the modifies-diff.
    #[test]
    fn explicit_flag_forces_emission_for_handlers_with_ensures() {
        let src = r#"spec ExplicitFlag
state { x : U64 }
handler bump (delta : U64) {
  ensures state.x == old(state.x) + delta
  effect { x += delta }
}"#;
        let spec = parse_str(src).expect("parse");
        // Auto-trigger silent (no modifies declared).
        assert!(!spec_triggers_impl_harness(&spec));

        let tmp =
            std::env::temp_dir().join(format!("kani_impl_explicit_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ true, Target::Anchor).expect("generate");
        assert!(tmp.is_file(), "explicit flag must emit the file");
        let body = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            body.contains("fn verify_bump_impl_ensures_0()"),
            "explicit flag must emit per-handler harness; got:\n{}",
            body
        );
        assert!(
            body.contains("accounts.handler(delta)"),
            "harness must call the user's real handler; got:\n{}",
            body
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// PDA-derived account addresses bind to the spec-declared seeds
    /// rather than `kani::any()`.
    #[test]
    fn pda_derived_accounts_bind_seed_expressions() {
        let src = r#"spec EscrowLite
state { initializer : Pubkey, amount : U64 }
pda escrow ["escrow", initializer]
handler open (deposit_amount : U64) {
  accounts {
    initializer : signer, writable
    escrow      : writable, pda ["escrow", initializer]
  }
  modifies [amount, initializer]
  ensures state.amount == deposit_amount
  effect { amount := deposit_amount }
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_pda_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            body.contains("find_program_address(&[b\"escrow\", initializer.as_ref()]"),
            "PDA derivation must come from the spec's `pda` declaration; got:\n{}",
            body
        );
        assert!(
            body.contains("`initializer`: signer"),
            "signer account must appear in the symbolic builder; got:\n{}",
            body
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// Issue #71: an integer handler param used as a PDA seed must
    /// serialize via `to_le_bytes()` — `u64::as_ref()` does not exist.
    /// Pubkey seeds / account keys keep `.as_ref()`.
    #[test]
    fn integer_param_seed_serializes_via_to_le_bytes() {
        let src = r#"spec Hub
state { lane_count : U64 }
pda hub_authority ["hub-authority", lane_id]
handler swap (lane_id : U64) {
  accounts {
    hub_authority : writable, pda ["hub-authority", lane_id]
    caller        : signer
  }
  modifies [lane_count]
  ensures state.lane_count == lane_id
  effect { lane_count := lane_id }
}"#;
        let spec = parse_str(src).expect("parse");
        let tmp = std::env::temp_dir().join(format!("kani_impl_intseed_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ true, Target::Anchor).expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();
        assert!(
            body.contains("lane_id.to_le_bytes().as_ref()"),
            "integer-param seed must serialize via to_le_bytes; got:\n{}",
            body
        );
        assert!(
            !body.contains("[b\"hub-authority\", lane_id.as_ref()"),
            "must not emit bare `lane_id.as_ref()` for a u64 param; got:\n{}",
            body
        );
        let _ = std::fs::remove_file(&tmp);
    }

    /// No emit when neither the explicit flag is on NOR any handler
    /// triggers auto-emission.
    #[test]
    fn no_emit_when_neither_flag_nor_auto_trigger() {
        let src = r#"spec Silent
state { x : U64 }
handler bump (delta : U64) {
  ensures state.x == old(state.x) + delta
  effect { x += delta }
}"#;
        let spec = parse_str(src).expect("parse");
        let tmp = std::env::temp_dir().join(format!("kani_impl_silent_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        assert!(
            !tmp.is_file(),
            "no flag + no auto-trigger must skip file emission"
        );
    }

    // ========================================================================
    // v2.26 Batch 2 Track I — CPI ensures-as-fact in impl-targeted harness
    // ========================================================================

    /// A handler with its own `ensures` AND a `call Iface.foo(args)` to an
    /// interface that declares ensures must emit `kani::assume(...)` lines
    /// between `if result.is_ok()` and the first caller `assert!`,
    /// substituting the callee's param names with the caller's call-site
    /// expressions. Mirror of `kani.rs`'s
    /// `cpi_ensures_lowers_to_kani_assume_in_preservation_harness` for the
    /// impl-targeted variant.
    #[test]
    fn cpi_ensures_as_assume_emits_at_splice_point() {
        let src = r#"spec CpiImplTest
program_id "11111111111111111111111111111111"

interface Token {
  program_id "11111111111111111111111111111111"
  handler transfer (amount : U64) {
    accounts {
      from      : writable
      to        : writable
      authority : signer
    }
    requires amount > 0
    ensures amount > 0
  }
}

state { pool : U64 }

handler deposit (amt : U64) {
  permissionless
  requires amt > 0 else InvalidAmount
  modifies [pool, lp_supply]
  call Token.transfer(from = 0, to = 0, amount = amt, authority = 0)
  effect { pool += amt }
  ensures state.pool == old(state.pool) + amt
}"#;
        let spec = parse_str(src).expect("parse");
        // The LP-shape diff (modifies = {pool, lp_supply}, effect-LHS = {pool})
        // triggers auto-emission.
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_track_i_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        // 1. The splice-marker comment from Track H must be GONE — Track I
        //    replaces it with the actual emission, no stale marker.
        assert!(
            !body.contains("<Track I CPI ensures-as-fact splice point>"),
            "Track H's splice marker must be removed once Track I has emitted; got:\n{}",
            body
        );

        // 2. The CPI ensures-as-fact comment + assume line must be present,
        //    with `amount` substituted to the caller's `amt` expression.
        assert!(
            body.contains("// CPI ensures-as-fact (Token.transfer):"),
            "missing CPI ensures-as-fact comment for Token.transfer; got:\n{}",
            body
        );
        assert!(
            body.contains("kani::assume(amt > 0)"),
            "missing substituted kani::assume(amt > 0); got:\n{}",
            body
        );

        // 3. Ordering: assume must sit between `if result.is_ok()` and the
        //    caller's first `assert!`.
        let is_ok_pos = body
            .find("if result.is_ok()")
            .expect("harness must have `if result.is_ok()`");
        let assume_pos = body
            .find("kani::assume(amt > 0)")
            .expect("assume present (just asserted above)");
        let assert_pos = body[is_ok_pos..]
            .find("assert!")
            .map(|i| is_ok_pos + i)
            .expect("caller's assert! must follow");
        assert!(
            is_ok_pos < assume_pos && assume_pos < assert_pos,
            "CPI assume must sit between is_ok() and assert!; got:\n{}",
            body
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// v2.26 Track K — impl-targeted variant of the spec-model
    /// `named_return_binder_substitutes_into_kani_assume` test.
    /// `let p = call Oracle.quote(…)` with `-> price : U64` declared
    /// must rewrite `price` to `p` in the emitted `kani::assume`.
    #[test]
    fn named_return_binder_substitutes_in_impl_harness() {
        let src = r#"spec NamedBinderImpl
program_id "11111111111111111111111111111111"

interface Oracle {
  program_id "11111111111111111111111111111111"
  handler quote (base : U64) -> price : U64 {
    ensures price > 0
  }
}

state { last_price : U64, lp_supply : U64 }

handler refresh (b : U64) {
  permissionless
  modifies [last_price, lp_supply]
  let p = call Oracle.quote(base = b)
  effect { last_price := b }
  ensures state.last_price == b
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_track_k_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        assert!(
            body.contains("// CPI ensures-as-fact (Oracle.quote):"),
            "missing CPI ensures-as-fact comment for Oracle.quote; got:\n{}",
            body,
        );
        // The callee uses `price` as its return binder; the caller's
        // `let p = …` makes `p` the substituted form.
        assert!(
            body.contains("kani::assume(p > 0)"),
            "expected `kani::assume(p > 0)` from named binder substitution; got:\n{}",
            body,
        );
        assert!(
            !body.contains("price > 0"),
            "binder name `price` must be substituted away; got:\n{}",
            body,
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// v2.27 Track A — `state_binders { ... }` rewrites
    /// `pre.<callee_field>` / `post.<callee_field>` to the caller's
    /// `pre.<caller_field>` / `post.<caller_field>` in the substituted
    /// `kani::assume`, which then flattens through
    /// `rewrite_pre_post_paths` to the harness-local
    /// `pre_<caller_field>` / `post_<caller_field>` snapshots.
    #[test]
    fn state_binders_rewrite_through_impl_snapshot_locals() {
        let src = r#"spec StateBindersImpl
program_id "11111111111111111111111111111111"

interface Token {
  program_id "11111111111111111111111111111111"
  handler transfer (amount : U64) {
    accounts {
      from      : writable
      to        : writable
      authority : signer
    }
    requires amount > 0
    ensures post.from_balance + amount == pre.from_balance
  }
}

state { pool_balance : U64, lp_supply : U64 }

handler deposit (amt : U64) {
  permissionless
  requires amt > 0 else InvalidAmount
  modifies [pool_balance, lp_supply]
  call Token.transfer(
    from = 0,
    to = 0,
    amount = amt,
    authority = 0,
    state_binders { from_balance = state.pool_balance },
  )
  effect { pool_balance -=! amt }
  ensures state.pool_balance == old(state.pool_balance) - amt
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_track_a_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        // The substitution rewrites `pre.from_balance` /
        // `post.from_balance` → `pre.pool_balance` / `post.pool_balance`
        // (via state_binders), then `rewrite_pre_post_paths` flattens
        // those to the snapshot locals `pre_pool_balance` /
        // `post_pool_balance`.
        assert!(
            body.contains("kani::assume(post_pool_balance + amt == pre_pool_balance)"),
            "expected flat snapshot locals in kani::assume; got:\n{}",
            body,
        );
        // The callee abstract field name must NOT survive.
        assert!(
            !body.contains("from_balance"),
            "abstract callee field `from_balance` must be substituted; got:\n{}",
            body,
        );
        // The snapshot block must capture `pool_balance` (the caller-
        // side binder field) — otherwise `pre_pool_balance` /
        // `post_pool_balance` references the assume emits don't compile.
        assert!(
            body.contains("let pre_pool_balance"),
            "snapshot block must capture pre_pool_balance; got:\n{}",
            body,
        );
        assert!(
            body.contains("let post_pool_balance"),
            "snapshot block must capture post_pool_balance; got:\n{}",
            body,
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// Tier-0 callees (interface declares no `ensures`) must not emit any
    /// `kani::assume` lines in the impl harness. Mirrors the spec-model
    /// variant's `tier0_callee_emits_no_kani_assume_lines` test.
    #[test]
    fn tier0_callee_emits_no_kani_assume_lines() {
        let src = r#"spec Tier0Impl
program_id "11111111111111111111111111111111"

interface Logger {
  program_id "11111111111111111111111111111111"
  handler log (msg : U64) {
    accounts {
      sink : writable
    }
  }
}

state { counter : U64 }

handler tick (val : U64) {
  permissionless
  requires val > 0 else Bad
  modifies [counter, shadow]
  call Logger.log(msg = val)
  effect { counter += val }
  ensures state.counter == old(state.counter) + val
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_tier0_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        assert!(
            !body.contains("CPI ensures-as-fact (Logger.log)"),
            "Tier-0 callee (no ensures) must not emit any CPI assume block; got:\n{}",
            body
        );
        // Caller's own assert! still emits.
        assert!(
            body.contains("assert!("),
            "caller's own assert! must still emit; got:\n{}",
            body
        );
        // And no `kani::assume(` introduced by Track I — the only assumes
        // that may appear are the caller's own requires-guard assume (none
        // here, since `val > 0` is the requires).
        // (We check by counting: the requires-guard assume is `val > 0`,
        // so a Logger-derived assume would appear separately.)
        let assume_count = body.matches("kani::assume(").count();
        // Exactly one assume — the caller's own requires-guard
        // (`val > 0 else Bad`).
        assert_eq!(
            assume_count, 1,
            "Tier-0 callee must not add any kani::assume lines; got {} assumes in:\n{}",
            assume_count, body
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// `let X = call Foo.bar(...)` puts `X` in scope in the substituted
    /// ensures via the `result` convention (v2.24 #11). Mirrors the
    /// spec-model variant's `let_call_binding_participates_in_substitution`
    /// test.
    #[test]
    fn let_binding_participates_in_substitution() {
        let src = r#"spec LetCallImpl
program_id "11111111111111111111111111111111"

interface Pool {
  program_id "11111111111111111111111111111111"
  handler absorb (amount : U64) -> U64 {
    accounts {
      vault : writable
    }
    requires amount > 0
    ensures result <= amount
  }
}

state { total_loss : U64 }

handler liquidate (loss : U64) {
  permissionless
  requires loss > 0 else Bad
  modifies [total_loss, shadow]
  let burned = call Pool.absorb(amount = loss)
  effect { total_loss += loss }
  ensures state.total_loss == old(state.total_loss) + loss
}"#;
        let spec = parse_str(src).expect("parse");
        assert!(spec_triggers_impl_harness(&spec));

        let tmp = std::env::temp_dir().join(format!("kani_impl_letcall_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        assert!(
            body.contains("// CPI ensures-as-fact (Pool.absorb):"),
            "missing CPI ensures-as-fact for Pool.absorb; got:\n{}",
            body
        );
        // `result <= amount` substitutes `amount → loss` and
        // `result → burned`.
        assert!(
            body.contains("kani::assume(burned <= loss)"),
            "let-binding result must substitute to caller's binder; got:\n{}",
            body
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// v2.26 Track J — when `multi_cpi_shared_fields` fires for a
    /// handler, the impl harness emits a WARNING breadcrumb comment
    /// above the CPI assume block so a reader of the generated file
    /// sees the over-constraint risk without cross-referencing the lint
    /// output. The breadcrumb sits between the post-snapshot and the
    /// first `kani::assume` from any CPI.
    #[test]
    fn multi_cpi_breadcrumb_emits_above_assume_block() {
        let src = r#"spec MultiCpiKaniImpl
program_id "11111111111111111111111111111111"

interface Token {
  program_id "11111111111111111111111111111111"
  handler transfer (amount : U64) {
    accounts {
      from      : writable
      to        : writable
      authority : signer
    }
    requires amount > 0
    ensures state.vault_balance == old(state.vault_balance) - amount
  }
}

state { vault_balance : U64 }

handler split (a : U64) (b : U64) {
  permissionless
  requires a > 0 else InvalidAmount
  requires b > 0 else InvalidAmount
  modifies [vault_balance, shadow]
  call Token.transfer(from = 0, to = 1, amount = a, authority = 0)
  call Token.transfer(from = 0, to = 2, amount = b, authority = 0)
  effect { vault_balance -= a }
  ensures state.vault_balance == old(state.vault_balance) - a - b
}"#;
        let spec = parse_str(src).expect("parse");
        // LP-shape gap (modifies = {vault_balance, shadow}, effect-LHS =
        // {vault_balance}) triggers auto-emission.
        assert!(spec_triggers_impl_harness(&spec));

        let tmp =
            std::env::temp_dir().join(format!("kani_impl_multi_cpi_{}.rs", std::process::id()));
        let _ = std::fs::remove_file(&tmp);
        generate_from_spec(&spec, &tmp, /*explicit_flag=*/ false, Target::Anchor)
            .expect("generate");
        let body = std::fs::read_to_string(&tmp).unwrap();

        // 1. Two CPI assume lines must be present (one per call).
        let assume_count = body
            .matches("// CPI ensures-as-fact (Token.transfer):")
            .count();
        assert_eq!(
            assume_count, 2,
            "two CPI assume blocks must emit; got {} in:\n{}",
            assume_count, body
        );

        // 2. The breadcrumb WARNING must appear in the harness body
        //    (above the CPI assume block).
        assert!(
            body.contains("WARNING: multi-CPI ordering"),
            "Track J breadcrumb must emit when multi_cpi_shared_fields fires; got:\n{}",
            body
        );
        assert!(
            body.contains("`multi_cpi_same_field`"),
            "breadcrumb must reference the lint rule name; got:\n{}",
            body
        );

        // 3. Ordering: WARNING sits between the `if result.is_ok()`
        //    branch open and the first `kani::assume` of the CPI block.
        let is_ok_pos = body
            .find("if result.is_ok()")
            .expect("`if result.is_ok()` must be present");
        let warn_pos = body
            .find("WARNING: multi-CPI ordering")
            .expect("breadcrumb present (just asserted)");
        let first_cpi_assume = body[is_ok_pos..]
            .find("// CPI ensures-as-fact")
            .map(|i| is_ok_pos + i)
            .expect("CPI assume block must follow is_ok()");
        assert!(
            is_ok_pos < warn_pos && warn_pos < first_cpi_assume,
            "WARNING breadcrumb must sit between is_ok() and the first \
             CPI ensures-as-fact comment; positions: is_ok={} warn={} cpi={}; got:\n{}",
            is_ok_pos,
            warn_pos,
            first_cpi_assume,
            body
        );

        let _ = std::fs::remove_file(&tmp);
    }

    /// v2.26 fold-in — a spec with no LP-shape handler but a ref_impl
    /// that carries potentially-overflowing arithmetic still auto-triggers
    /// the impl-targeted harness. Lean proves on `Nat`; Kani is the only
    /// verification surface that catches the `u64` overflow.
    #[test]
    fn ref_impl_overflow_risk_auto_triggers_impl_harness() {
        let src = r#"spec Pool
type Error | InvalidAmount
type State = { x : U64 }

ref_impl scaled (a : U64) (b : U64) : U64 = a * b

handler set (amt : U64) {
  requires amt > 0 else InvalidAmount
  effect { x := amt }
  ensures state.x == scaled(old(state.x), amt)
}
"#;
        let spec = crate::chumsky_adapter::parse_str(src).expect("parse");
        // No handler trips the LP-shape signal (`set` declares no modifies).
        assert!(
            !spec.handlers.iter().any(handler_triggers_impl_harness),
            "no handler should trip the modifies-driven trigger in this fixture"
        );
        // But the ref_impl `scaled` has `*` over U64, so the auto-trigger
        // still fires through the ref_impl overflow-risk predicate.
        assert!(
            spec_triggers_impl_harness(&spec),
            "ref_impl with multiplication over bounded-numeric params \
             must auto-trigger the impl harness"
        );
    }

    /// Symmetric negative: ref_impl with only division (no overflow risk)
    /// AND no LP-shape handler — auto-trigger stays quiet.
    #[test]
    fn ref_impl_without_overflow_risk_does_not_auto_trigger() {
        let src = r#"spec Pool
type Error | InvalidAmount
type State = { x : U64 }

ref_impl half (a : U64) : U64 = a / 2

handler set (amt : U64) {
  requires amt > 0 else InvalidAmount
  effect { x := amt }
}
"#;
        let spec = crate::chumsky_adapter::parse_str(src).expect("parse");
        assert!(
            !spec_triggers_impl_harness(&spec),
            "ref_impl with only division must not auto-trigger \
             (no overflow risk, nothing for Kani to catch)"
        );
    }
}
