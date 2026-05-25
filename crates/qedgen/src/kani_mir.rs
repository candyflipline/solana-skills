// Phase 3 (v2.30) MIR-based Kani codegen. Lives in parallel to
// `kani.rs` until snapshot equivalence is validated on every pilot
// fixture; then `kani.rs` is retired.
//
// Dead-code warnings expected during incremental wiring.
#![allow(dead_code)]

//! qedgen Kani codegen — MIR consumer.
//!
//! Phase 3 of the v2.30 refactor. The existing `kani.rs` (2,437 LoC)
//! consumes `ParsedSpec` directly; this module is being ported to
//! consume `mir::Mir` instead, mirroring the `lean_gen` →
//! `lean_gen_mir` carry-through (Phase 1).
//!
//! ## Dispatch
//!
//! `QEDGEN_USE_MIR_KANI=1` switches the `qedgen codegen --kani` call
//! site to this module; without the flag, legacy `kani::generate` runs.
//! Default stays on legacy until snapshot equivalence ratifies every
//! pilot fixture (per the Phase 1 sequencing).
//!
//! ## Phase 3a-3c1 scope
//!
//! Phase 3a (prior commit): scaffold + deterministic structural
//! prefix (file header, math helpers, state-model header, constants).
//!
//! Phase 3b (prior commit): per-account section structural body for
//! single-account specs — records, unit enum sums, `Status` enum,
//! `State` struct, property predicates, invariant predicates,
//! transition fns, ref_impls. All delegated to existing
//! `rust_codegen_util::emit_*` helpers so the output is byte-identical
//! to legacy.
//!
//! Phase 3c1 (this commit): guard enforcement harnesses — one
//! `verify_<handler>_rejects_invalid()` per handler with a guard or
//! requires clause. Calls the newly-promoted
//! `rust_codegen_util::emit_state_init_symbolic` /
//! `emit_pre_status_assume` (both now `pub fn`, shared with kani.rs)
//! plus existing helpers (`collect_full_guard`, `emit_abstract_binders`,
//! `map_type`). Multi-account `mod <name> { ... }` wrapping stays at
//! `MIR-TODO(phase-3e)` marker. The other harness sections (abort /
//! property-preservation / invariant-preservation / effect /
//! overflow + file-level features) stay at `MIR-TODO(phase-3c2+)`.
//!
//! ## What we must replicate from `kani.rs`
//!
//! - `kani::generate(spec_path, output_path)`:
//!   - Parses, validates handler set, ensures parent dir
//!   - Computes fingerprint hash (`tests/kani.rs` slot)
//!   - Emits banner + file header + math helpers + state-model header
//!   - Emits constants (file-scoped — referenced from per-ADT modules)
//!   - Branches on `is_multi = account_types.len() > 1`:
//!     - Multi: per-account `mod <lowercase> { use super::*; ... }`
//!     - Single: flat `emit_kani_account_section` at file root
//!   - Emits file-level features (covers / liveness / env) in single mode
//!   - Writes the `DO NOT EDIT BELOW` footer
//!   - Prints summary counts to stderr
//!
//! Per-account section (`emit_kani_account_section`):
//!   - User-defined records (`emit_record_structs`)
//!   - Unit enum sums (`emit_unit_enum_sums`)
//!   - `Status` enum (per-account lifecycle) — kani::Arbitrary
//!   - `State` struct (per-account fields + optional `status` field)
//!   - Transition fns — pure mutators returning `bool` (guard outcome)
//!   - Guard enforcement harnesses (one per handler with a guard)
//!   - Property preservation harnesses (one per (property, handler) cross
//!     filtered by `preserved_by`)
//!   - Invariant preservation harnesses
//!   - Effect conformance harnesses (per-effect, per-handler)
//!   - Overflow detection harnesses (auto-add fields)
//!   - Abort condition harnesses (per `requires X else Err`)
//!
//! ## Open question: shared `rust_codegen_util` helpers
//!
//! `kani.rs` reaches into `crate::rust_codegen_util` for
//! `mutable_fields`, `resolve_state_fields`, `emit_constants`,
//! `emit_record_structs`, `emit_unit_enum_sums`, etc. These helpers
//! consume `ParsedSpec` / `ParsedAccountType` / `&[(String, String)]`
//! directly. The Phase 3 port has a choice:
//!   1. Pass `&ParsedSpec` alongside `&Mir` so helpers stay shared.
//!   2. Port each helper to consume MIR fragments.
//!
//! Phase 3a takes (1) — `generate(&Mir, &ParsedSpec, &Path)` — to
//! match the lean_gen_mir pattern. Future slices may migrate helpers
//! incrementally; until then this preserves byte-equivalence with the
//! legacy path on shared codegen primitives.

use anyhow::Result;
use std::path::Path;

use crate::check::ParsedSpec;
use crate::mir::Mir;

/// Generate the Kani harness file at `output_path`, consuming MIR.
/// Mirrors `kani::generate(spec_path, output_path)` shape but accepts
/// a pre-lowered `Mir` + the originating `ParsedSpec` (the latter is
/// passed through to helpers that haven't been MIR-ported yet — see
/// the open-question note in this module's header).
pub fn generate(mir: &Mir, parsed: &ParsedSpec, output_path: &Path) -> Result<()> {
    if mir.handlers.is_empty() {
        anyhow::bail!("No operations found in the spec — is this a valid qedspec file?");
    }

    crate::rust_codegen_util::check_effect_targets(parsed)?;

    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }

    let content = render(mir, parsed);
    std::fs::write(output_path, &content)?;

    eprintln!(
        "[MIR-pilot] Generated Kani harness scaffold at {} (Phase 3a — structural sections only)",
        output_path.display()
    );
    Ok(())
}

/// Pure render. Phase 3a-3b emits the deterministic structural
/// prefix (banner / math helpers / state-model header / constants),
/// the per-account structural body (records / enums / Status /
/// State / property predicates / invariant predicates / transitions),
/// and a `MIR-TODO` marker where future slices (harness emissions
/// and multi-account wrapping) pick up.
pub fn render(mir: &Mir, parsed: &ParsedSpec) -> String {
    let mut out = String::new();
    emit_header(&mut out, parsed);
    emit_math_helpers(&mut out, parsed);
    emit_state_model_header(&mut out);
    emit_constants(&mut out, mir);

    // Multi-account specs route through `mod <lowercase> { ... }`
    // wrapping per account in legacy. Phase 3e ports that; until
    // then, fall through to the single-account body which emits the
    // primary account's view (still useful to eyeball but won't
    // compile against multi-account state).
    if parsed.account_types.len() > 1 {
        out.push_str(
            "// MIR-TODO(phase-3e): multi-account `mod <name> { use super::*; ... }` \
             wrapping not ported yet — only the primary account emits below.\n\n",
        );
    }

    if let Err(e) = emit_account_section_structural(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: account-section emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_guard_enforcement_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: guard-enforcement emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_abort_condition_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: abort-condition emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_property_preservation_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: property-preservation emit failed: {}\n",
            e
        ));
    }

    // Section order matches legacy `emit_kani_account_section`:
    // property → ensures → invariant → effect → overflow. Slotting
    // ensures between property and invariant is load-bearing for
    // byte-equivalence with percolator (which has ensures clauses);
    // any other ordering breaks the diff.
    if let Err(e) = emit_ensures_preservation_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: ensures-preservation emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_invariant_preservation_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: invariant-preservation emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_effect_conformance_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: effect-conformance emit failed: {}\n",
            e
        ));
    }

    if let Err(e) = emit_overflow_detection_harnesses(&mut out, parsed) {
        out.push_str(&format!(
            "// MIR-ERROR: overflow-detection emit failed: {}\n",
            e
        ));
    }

    // Phase 3d — file-level features (covers / liveness /
    // environment). Single-mode only, matching legacy
    // `emit_file_level_features` gating. Multi-account specs skip
    // these (per-ADT lifting is v2.22 scope per the legacy comment;
    // Phase 3e closes the multi-account wrapping).
    if parsed.account_types.len() <= 1 {
        if let Err(e) = emit_file_level_features(&mut out, parsed) {
            out.push_str(&format!(
                "// MIR-ERROR: file-level-features emit failed: {}\n",
                e
            ));
        }
    }

    // Phase 3e+ stub. Only multi-account `mod <name> { use
    // super::*; ... }` wrapping remains.
    out.push_str(
        "// MIR-TODO(phase-3e): multi-account `mod <name> { ... }` wrapping not ported \
         yet — single-account specs are fully covered.\n",
    );

    out.push_str("// ---- GENERATED BY QEDGEN — DO NOT EDIT BELOW THIS LINE ----\n");
    out
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3a structural prefix
// ----------------------------------------------------------------------

/// File header: banner with the `tests/kani.rs` fingerprint hash +
/// the legacy docstring. Mirrors `kani::generate` lines ~135–152.
fn emit_header(out: &mut String, parsed: &ParsedSpec) {
    let fp = crate::fingerprint::compute_fingerprint(parsed);
    let hash = fp
        .file_hashes
        .get("tests/kani.rs")
        .cloned()
        .unwrap_or_default();

    out.push_str(&crate::banner::banner(None, &hash));
    out.push_str("//\n");
    out.push_str("// Self-contained Kani proof harnesses for the spec.\n");
    out.push_str("//\n");
    out.push_str("// These proofs verify the spec's transition design using Kani bounded model\n");
    out.push_str("// checking. They operate on a pure model of the state machine (derived from\n");
    out.push_str("// the qedspec), independent of framework (Quasar/Anchor) types.\n");
    out.push_str("//\n");
    out.push_str("//   Lean proves:  transition functions preserve invariants (∀ states)\n");
    out.push_str(
        "//   Kani checks:  same properties via bounded model checking + overflow detection\n",
    );
    out.push_str("//   Together:     high assurance that the spec design is correct\n");
    out.push_str("//\n");
    out.push_str("// To run:  cargo kani --harness <name>   (requires cargo-kani)\n");
    out.push_str("// ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ---- ----\n");
    out.push_str("#![cfg(kani)]\n\n");
}

/// Math helpers (`mul_div_floor_u128` / `mul_div_ceil_u128`). Inlined
/// only when the spec's guards reference them (same predicate as
/// `kani.rs` line ~172). Mirrors the inline-when-needed shape so the
/// standalone harness compiles without depending on `src/math.rs`.
fn emit_math_helpers(out: &mut String, parsed: &ParsedSpec) {
    // Byte-equivalence note: legacy `kani.rs::generate` uses
    // backslash-newline continuations whose leading-whitespace consumption
    // strips the body indentation. The output is technically "wrong"
    // (no per-line indent inside the fns) but it's the canonical shape
    // every committed kani.rs fixture was generated against. Mirror
    // verbatim so Phase 3 stays byte-equivalent until the legacy emit
    // is intentionally re-indented.
    if crate::codegen::guards_use_math_helpers(parsed) {
        out.push_str(
            "#[allow(dead_code)]\n\
#[inline]\n\
fn mul_div_floor_u128(a: u128, b: u128, d: u128) -> u128 {\n\
    if d == 0 { return 0; }\n\
    a.saturating_mul(b) / d\n\
}\n\n\
#[allow(dead_code)]\n\
#[inline]\n\
fn mul_div_ceil_u128(a: u128, b: u128, d: u128) -> u128 {\n\
    if d == 0 { return 0; }\n\
    let prod = a.saturating_mul(b);\n\
    if prod % d == 0 { prod / d } else { (prod / d).saturating_add(1) }\n\
}\n\n",
        );
    }
}

/// State model header banner. Always emitted, even when the spec
/// declares no state (the empty banner is harmless and matches legacy
/// `kani::generate` line ~191).
fn emit_state_model_header(out: &mut String) {
    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// State model (derived from qedspec — no framework dependencies)\n");
    out.push_str(
        "// ============================================================================\n\n",
    );
}

/// File-scoped constants — `pub const NAME: u64 = VALUE;` per
/// `Mir.constants` entry. Per-ADT modules reference these via
/// `use super::*`, so they live at file scope rather than being
/// duplicated. Legacy delegates to `rust_codegen_util::emit_constants`;
/// MIR carries the same `(name, value)` pair shape so we can call into
/// the same helper for byte-equivalence.
fn emit_constants(out: &mut String, mir: &Mir) {
    if mir.constants.is_empty() {
        return;
    }
    crate::rust_codegen_util::emit_constants(out, &mir.constants);
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3b per-account structural body
// ----------------------------------------------------------------------

/// Per-account section structural body — single-account path. Mirrors
/// `kani::emit_kani_account_section` lines ~369–490 (records /
/// unit-enum sums / Status / State / property predicates / invariant
/// predicates / transition fns / ref_impls). Harness emissions stay
/// at the Phase 3c+ marker.
///
/// Multi-account dispatch (`mod <lowercase>`) is Phase 3e — until
/// then, multi-account specs emit only the primary account's view
/// here, prefixed by a `MIR-TODO(phase-3e)` marker in `render()`.
fn emit_account_section_structural(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    // Resolve state-fields + lifecycle view. Mirrors `kani::generate`
    // lines ~257–267:
    //   * single `type Account` block → use its fields + lifecycle
    //   * otherwise → fall back to `resolve_state_fields` +
    //     `spec.lifecycle_states`
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            // Multi-account: emit the primary account's view as the
            // best-effort approximation. Phase 3e replaces this with
            // proper per-account `mod` wrapping.
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };

    let mutable = util::mutable_fields(state_fields);
    let has_lifecycle = lifecycle.len() >= 2;

    // 1. User-defined record structs.
    util::emit_record_structs(out, parsed, "Clone, Copy, kani::Arbitrary", |t| {
        map_type(t, parsed)
    })?;

    // 2. Unit enum sums (sum-type variants without payload).
    util::emit_unit_enum_sums(out, parsed, "Clone, Copy, PartialEq, Eq, kani::Arbitrary")?;

    // 3. Status enum (per-account lifecycle).
    util::emit_lifecycle_status_enum_from(
        out,
        lifecycle,
        "Clone, Copy, PartialEq, Eq, kani::Arbitrary",
    );

    // 4. State struct.
    util::emit_state_struct_with_lifecycle(
        out,
        &mutable,
        "Clone, Copy",
        |t| map_type(t, parsed),
        has_lifecycle,
    )?;

    // 5. Property predicates.
    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();
    let properties: Vec<&crate::check::ParsedProperty> = parsed.properties.iter().collect();
    if !properties.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Property predicates (from qedspec `property` declarations)\n");
        out.push_str(
            "// ============================================================================\n\n",
        );
        // `emit_property_predicates_with` takes &[ParsedProperty] (not
        // &[&_]); reconstruct an owned Vec view of the filtered slice
        // (matches the legacy line 415–416 shape).
        let owned: Vec<crate::check::ParsedProperty> =
            properties.iter().map(|p| (*p).clone()).collect();
        util::emit_property_predicates_with(out, &owned, false, |t| map_type(t, parsed));
    }

    // 6. Invariant predicates (filter to those linked from a handler
    //    in this section — mirrors legacy line 427–448).
    let linked_invs: Vec<&crate::check::ParsedInvariant> = parsed
        .invariants
        .iter()
        .filter(|i| {
            handlers
                .iter()
                .any(|h| h.invariants.contains(&i.name) || h.establishes.contains(&i.name))
        })
        .collect();
    if !linked_invs.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Invariant predicates (from qedspec `invariant` declarations linked via\n");
        out.push_str(
            "// handler-side `invariant Name` clauses). v2.17.x wires ParsedInvariant.rust_expr\n",
        );
        out.push_str("// through to per-(handler, invariant) BMC preservation harnesses below.\n");
        out.push_str(
            "// ============================================================================\n\n",
        );
        util::emit_invariant_predicates(out, &linked_invs);
    }

    // 7. Transition functions (one per handler).
    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Transition functions (from qedspec operations — effects + guards)\n");
    out.push_str("//\n");
    out.push_str("// Each returns true if the guard passes and the transition fires,\n");
    out.push_str("// false if the guard rejects the operation.\n");
    out.push_str(
        "// ============================================================================\n\n",
    );
    for op in &handlers {
        util::emit_transition_fn(out, op, parsed, false, |t| map_type(t, parsed))?;
    }

    // 8. Reference implementations (v2.25 — pure-expression fns
    //    callable from ensures-preservation harnesses). Mirrors legacy
    //    line 470–491.
    if !parsed.ref_impls.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Reference implementations (from qedspec ref_impl declarations).\n");
        out.push_str(
            "// ============================================================================\n\n",
        );
        for r in &parsed.ref_impls {
            let params = r
                .params
                .iter()
                .map(|(n, t)| {
                    map_type(t, parsed)
                        .map(|rt| format!("{}: {}", n, rt))
                        .unwrap_or_else(|_| format!("{}: {}", n, t))
                })
                .collect::<Vec<_>>()
                .join(", ");
            let ret = map_type(&r.return_type, parsed).unwrap_or_else(|_| r.return_type.clone());
            out.push_str(&format!(
                "fn {}({}) -> {} {{\n    {}\n}}\n\n",
                r.name, params, ret, r.rust_body
            ));
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c1 guard-enforcement harnesses
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_rejects_invalid()` for
/// every handler with a guard or `requires` clause. Mirrors
/// `kani::emit_kani_account_section` lines ~493–568 (single-account
/// path; multi-account `mod <name>` wrapping is Phase 3e).
///
/// One harness per handler:
///   * Initialize state symbolically (`emit_state_init_symbolic`)
///   * `kani::assume(s.status == Status::<pre>)` if the handler is
///     lifecycle-gated
///   * Declare every param + abstract-binder as `kani::any()`
///   * `kani::assume(!(full_guard))` — at least one guard component
///     is violated
///   * `assert!(!<handler>(&mut s, args...))` — handler must reject
fn emit_guard_enforcement_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    // Resolve view — same logic as `emit_account_section_structural`.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    let guard_ops: Vec<&crate::check::ParsedHandler> =
        parsed.handlers.iter().filter(|op| op.has_guard()).collect();

    if guard_ops.is_empty() {
        return Ok(());
    }

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Guard enforcement — transitions reject invalid inputs\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    for op in &guard_ops {
        let Some(full_guard) = util::collect_full_guard(op, false) else {
            // Handler had `has_guard()` set but no expressible
            // negation — skip to avoid `kani::assume(!(true))`
            // vacuous harnesses (matches legacy kani.rs:515–519).
            continue;
        };

        out.push_str("#[kani::proof]\n");
        out.push_str("#[kani::unwind(2)]\n");
        out.push_str("#[kani::solver(cadical)]\n");
        out.push_str(&format!("fn verify_{}_rejects_invalid() {{\n", op.name));

        util::emit_state_init_symbolic(out, &mutable, lifecycle);
        util::emit_pre_status_assume(out, op, lifecycle);

        // Symbolic params.
        for (pname, ptype) in &op.takes_params {
            out.push_str(&format!(
                "    let {}: {} = kani::any();\n",
                pname,
                map_type(ptype, parsed)?
            ));
        }

        // v2.29 Slice A (#8) — abstract binders. Legacy kani.rs:537–546
        // calls `emit_abstract_binders` TWICE in a row with identical
        // args (looks like a copy-paste accident; surfaces only for
        // specs that declare abstract binders, where it would emit
        // duplicate `let X: T = kani::any();` lines). Per
        // [[feedback-cleanup-v3]] preserve the bug here for byte-
        // equivalence; cleanup deferred to v3.0 alongside the legacy.
        util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;
        util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

        out.push_str(&format!("    kani::assume(!({full_guard}));\n"));

        let args: String = op
            .takes_params
            .iter()
            .chain(op.abstract_binders.iter())
            .map(|(n, _)| format!(", {}", n))
            .collect();
        out.push_str(&format!("    assert!(!{}(&mut s{}),\n", op.name, args));
        out.push_str(&format!(
            "        \"{} must reject when guard is violated\");\n",
            op.name
        ));
        out.push_str("}\n\n");
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c2 abort-condition harnesses
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_aborts_if_<error>()` for
/// every `requires X else Error` clause across every handler.
/// Mirrors `kani::emit_kani_account_section` lines ~501–565.
///
/// One harness per (handler, abort clause):
///   * Symbolic state + pre-status assume + symbolic params +
///     (double-emit) abstract binders (bug-for-bug parity)
///   * `kani::assume(<abort.rust_expr>)` — the condition that
///     should trigger abortion
///   * `assert!(!<handler>(...))` — handler must reject
fn emit_abort_condition_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    // Resolve view — same logic as `emit_account_section_structural`.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    let abort_ops: Vec<&crate::check::ParsedHandler> = parsed
        .handlers
        .iter()
        .filter(|op| !op.aborts_if.is_empty())
        .collect();

    if abort_ops.is_empty() {
        return Ok(());
    }

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Abort conditions — operations must reject under specified conditions\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    for op in &abort_ops {
        for abort in &op.aborts_if {
            out.push_str("#[kani::proof]\n");
            out.push_str("#[kani::unwind(2)]\n");
            out.push_str("#[kani::solver(cadical)]\n");
            out.push_str(&format!(
                "fn verify_{}_aborts_if_{}() {{\n",
                op.name, abort.error_name
            ));

            util::emit_state_init_symbolic(out, &mutable, lifecycle);
            util::emit_pre_status_assume(out, op, lifecycle);

            for (pname, ptype) in &op.takes_params {
                out.push_str(&format!(
                    "    let {}: {} = kani::any();\n",
                    pname,
                    map_type(ptype, parsed)?
                ));
            }
            // Bug-for-bug parity: legacy double-calls
            // `emit_abstract_binders`. See guard-enforcement comment.
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

            out.push_str(&format!("    kani::assume({});\n", abort.rust_expr));

            let args: String = op
                .takes_params
                .iter()
                .chain(op.abstract_binders.iter())
                .map(|(n, _)| format!(", {}", n))
                .collect();
            out.push_str(&format!("    assert!(!{}(&mut s{}),\n", op.name, args));
            out.push_str(&format!(
                "        \"{} must abort with {}\");\n",
                op.name, abort.error_name
            ));
            out.push_str("}\n\n");
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c3 property-preservation harnesses
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_preserves_<property>()`
/// for every `(property, handler)` pair where `handler` is named in
/// the property's `preserved_by` list. Mirrors
/// `kani::emit_kani_account_section` lines ~567–802.
///
/// Per-pair harness shape:
///   * Pre-state: zeroed for init handlers (`pre_status ==
///     Uninitialized`), symbolic for non-init; `let mut post = pre;`
///   * Non-init: lifecycle pre-status assume, optional per-slot
///     binder bind, pre-property assumes (unary only — Binary
///     properties skip), MAX_MEMBERS-derived bound assume
///   * Symbolic params + abstract binders
///   * `emit_add_strict_bounds` for add-effect overflow gating
///   * `if <handler>(&mut post, args) { assert!(<prop>...); }`
///     dispatched on prop class (Binary → `prop(&pre, &post)`,
///     per-slot Unary → `prop_at(&post, binder)`, plain Unary →
///     `prop(&post)`)
fn emit_property_preservation_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    if parsed.properties.is_empty() {
        return Ok(());
    }

    // Resolve view — same logic as `emit_account_section_structural`.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Property preservation — invariants hold through all transitions\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();
    let properties: Vec<&crate::check::ParsedProperty> = parsed.properties.iter().collect();

    for prop in &properties {
        if prop.expression.is_none() {
            continue;
        }

        for op_name in &prop.preserved_by {
            // Filter to handlers in this section (single-account
            // mode → all of them; multi-account → only those tied
            // to the primary account, which Phase 3e closes).
            let Some(op) = handlers.iter().copied().find(|o| &o.name == op_name) else {
                continue;
            };

            out.push_str("#[kani::proof]\n");
            out.push_str("#[kani::unwind(2)]\n");
            out.push_str("#[kani::solver(cadical)]\n");
            out.push_str(&format!(
                "fn verify_{}_preserves_{}() {{\n",
                op_name, prop.name
            ));

            let is_init = op.pre_status.as_deref() == Some("Uninitialized");

            // v2.20 §S1.1: per-slot binder handling — skip the
            // local binding when the handler param shadows it
            // (same binder pre & post unifies the value).
            let handler_takes_binder = match &prop.per_slot {
                Some(slot) => op
                    .takes_params
                    .iter()
                    .any(|(n, t)| n == &slot.binder_name && t == &slot.binder_type),
                _ => false,
            };
            let needs_local_binder = prop.per_slot.is_some() && !handler_takes_binder;

            if is_init {
                // Init handler — pre-state is zeroed.
                out.push_str("    let pre = ");
                out.push_str("State {\n");
                for (fname, ftype) in &mutable {
                    if let Some(default) =
                        crate::proptest_gen::default_value_for_field(ftype, parsed)
                    {
                        out.push_str(&format!("        {}: {},\n", fname, default));
                    }
                }
                if let Some(initial) = lifecycle.first() {
                    if lifecycle.len() >= 2 {
                        out.push_str(&format!("        status: Status::{},\n", initial));
                    }
                }
                out.push_str("    };\n");
                out.push_str("    let mut post = pre;\n");
            } else {
                // Non-init — pre is symbolic.
                out.push_str("    let pre = State {\n");
                for (fname, _) in &mutable {
                    out.push_str(&format!("        {}: kani::any(),\n", fname));
                }
                if lifecycle.len() >= 2 {
                    out.push_str("        status: kani::any(),\n");
                }
                out.push_str("    };\n");
                if lifecycle.len() >= 2 {
                    if let Some(ref pre_status) = op.pre_status {
                        out.push_str(&format!(
                            "    kani::assume(pre.status == Status::{});\n",
                            pre_status
                        ));
                    }
                }

                if needs_local_binder {
                    if let Some(slot) = &prop.per_slot {
                        let rust_ty = map_type(&slot.binder_type, parsed)?;
                        out.push_str(&format!(
                            "    let {}: {} = kani::any();\n",
                            slot.binder_name, rust_ty
                        ));
                    }
                }

                // v2.23 Slice 4: assume unary pre-properties hold;
                // skip Binary (those have a `(pre, post)` shape
                // that asserts trivially against `(pre, pre)`).
                for pre_prop in &properties {
                    if pre_prop.expression.is_none() {
                        continue;
                    }
                    if pre_prop.class == crate::check::PropertyClass::Binary {
                        continue;
                    }
                    match &pre_prop.per_slot {
                        Some(slot) if pre_prop.name == prop.name => {
                            out.push_str(&format!(
                                "    kani::assume({}_at(&pre, {}));\n",
                                pre_prop.name, slot.binder_name
                            ));
                        }
                        _ => {
                            out.push_str(&format!("    kani::assume({}(&pre));\n", pre_prop.name));
                        }
                    }
                }

                // MAX_MEMBERS-derived bound assume — derived from
                // create_vault guard; same shape as legacy
                // kani.rs:715–728.
                if !parsed.constants.is_empty() {
                    for (cname, _cval) in &parsed.constants {
                        let upper = cname.to_uppercase();
                        if upper.contains("MAX") || upper.contains("MEMBER") {
                            if mutable.iter().any(|(f, _)| f == "member_count") {
                                out.push_str(&format!(
                                    "    kani::assume(pre.member_count <= {});\n",
                                    upper
                                ));
                            }
                            break;
                        }
                    }
                }

                out.push_str("    let mut post = pre;\n");
            }

            // Symbolic params.
            for (pname, ptype) in &op.takes_params {
                out.push_str(&format!(
                    "    let {}: {} = kani::any();\n",
                    pname,
                    map_type(ptype, parsed)?
                ));
            }
            // v2.29 Slice A (#8) — abstract binders. Single call
            // here (NOT the double-emit bug of guard/abort
            // sections) — legacy kani.rs:742–745 calls it once.
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

            // `emit_add_strict_bounds` against pre-state — same
            // owned-Vec workaround as legacy kani.rs:750–752.
            let owned_props: Vec<crate::check::ParsedProperty> =
                properties.iter().map(|p| (*p).clone()).collect();
            util::emit_add_strict_bounds(
                out,
                op,
                &owned_props,
                "    kani::assume(pre.{field} < pre.{bound}); // strict bound: {field} increments\n",
            );

            // Transition call + dispatch on prop class.
            let args: String = op
                .takes_params
                .iter()
                .chain(op.abstract_binders.iter())
                .map(|(n, _)| format!(", {}", n))
                .collect();
            out.push_str(&format!("    if {}(&mut post{}) {{\n", op_name, args));
            let is_binary_prop = prop.class == crate::check::PropertyClass::Binary;
            if is_binary_prop {
                out.push_str(&format!("        assert!({}(&pre, &post),\n", prop.name));
                out.push_str(&format!(
                    "            \"{} must hold after {} (binary: pre/post)\");\n",
                    prop.name, op_name
                ));
            } else {
                match &prop.per_slot {
                    Some(slot) => {
                        out.push_str(&format!(
                            "        assert!({}_at(&post, {}),\n",
                            prop.name, slot.binder_name
                        ));
                        out.push_str(&format!(
                            "            \"{} must hold after {} (forall {} : {})\");\n",
                            prop.name, op_name, slot.binder_name, slot.binder_type
                        ));
                    }
                    None => {
                        out.push_str(&format!("        assert!({}(&post),\n", prop.name));
                        out.push_str(&format!(
                            "            \"{} must hold after {}\");\n",
                            prop.name, op_name
                        ));
                    }
                }
            }
            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c7 ensures-preservation harnesses (v2.25 Phase B)
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_ensures_<idx>()` for every
/// `(handler, ensures clause)` pair. Mirrors
/// `kani::emit_kani_account_section` lines ~804–953 (v2.25 Phase B).
///
/// Per-pair harness shape:
///   * Symbolic state init + pre-status assume
///   * Symbolic params + abstract binders
///   * `kani::assume(<full_guard>)` so the harness only explores
///     pre-states the transition wouldn't reject (otherwise the
///     ensures clause would pass vacuously).
///   * `let pre = s.clone();` snapshot AFTER the assumes so it
///     reflects the constrained pre-state.
///   * `if <handler>(&mut s, args) { … }`:
///     - `let post = &s;` binds the `post.x` paths in the rendered
///       ensures expression.
///     - **CPI ensures-as-fact** (v2.26 Track G): for every
///       `call Iface.foo(args)` whose callee declares ensures,
///       substitute the callee params with the caller's call-site
///       expressions and emit `kani::assume(<substituted>);` so the
///       callee's contract becomes a fact in the caller's harness.
///       Tier-0 callees (no ensures) emit nothing — the
///       `cpi_no_callee_ensures` lint surfaces this.
///     - `assert!(<ensures.rust_expr_binary>, "ensures clause N on
///       <handler> violated ...")`.
///
/// **Position load-bearing**: this section sits between property-
/// preservation and invariant-preservation in legacy. Putting it
/// elsewhere breaks byte-equivalence on percolator (which exercises
/// `ensures` clauses with old(...) bindings).
fn emit_ensures_preservation_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();

    let handlers_with_ensures: Vec<&crate::check::ParsedHandler> = handlers
        .iter()
        .copied()
        .filter(|h| !h.ensures.is_empty())
        .collect();

    if handlers_with_ensures.is_empty() {
        return Ok(());
    }

    // Resolve view.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Ensures preservation — `ensures <expr>` clauses verified against\n");
    out.push_str("// (pre, post) of the spec-translated transition. Counterexamples here\n");
    out.push_str("// indicate the spec's effect block doesn't satisfy its own ensures —\n");
    out.push_str("// usually because the math lives in the user's Rust impl, behind a\n");
    out.push_str("// `modifies`-driven todo!() fill site. See SKILL.md §ref_impl.\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    for op in handlers_with_ensures {
        for (idx, ensures) in op.ensures.iter().enumerate() {
            out.push_str("#[kani::proof]\n");
            out.push_str("#[kani::unwind(2)]\n");
            out.push_str("#[kani::solver(cadical)]\n");
            out.push_str(&format!("fn verify_{}_ensures_{}() {{\n", op.name, idx));

            util::emit_state_init_symbolic(out, &mutable, lifecycle);
            util::emit_pre_status_assume(out, op, lifecycle);

            for (pname, ptype) in &op.takes_params {
                out.push_str(&format!(
                    "    let {}: {} = kani::any();\n",
                    pname,
                    map_type(ptype, parsed)?
                ));
            }
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

            // Assume requires hold pre-state (avoid vacuous pass).
            if let Some(full_guard) = util::collect_full_guard(op, false) {
                out.push_str(&format!("    kani::assume({});\n", full_guard));
            }

            // Snapshot AFTER assumes — pre reflects the
            // constrained pre-state Kani explores.
            out.push_str("    let pre = s.clone();\n");

            let args: String = op
                .takes_params
                .iter()
                .chain(op.abstract_binders.iter())
                .map(|(n, _)| format!(", {}", n))
                .collect();
            out.push_str(&format!("    if {}(&mut s{}) {{\n", op.name, args));
            out.push_str("        let post = &s;\n");

            // v2.26 Track G — CPI ensures-as-fact propagation.
            for call in &op.calls {
                let Some(iface) = parsed
                    .interfaces
                    .iter()
                    .find(|i| i.name == call.target_interface)
                else {
                    continue;
                };
                let Some(callee_handler) = iface
                    .handlers
                    .iter()
                    .find(|h| h.name == call.target_handler)
                else {
                    continue;
                };
                if callee_handler.ensures.is_empty() {
                    continue;
                }
                out.push_str(&format!(
                    "        // CPI ensures-as-fact ({}.{}):\n",
                    call.target_interface, call.target_handler,
                ));
                for callee_ens in &callee_handler.ensures {
                    let substituted = crate::cpi_substitute::substitute_callee_ensures_rust_binary(
                        &callee_ens.rust_expr_binary,
                        call,
                        &callee_handler.params,
                        callee_handler.result_binder.as_deref(),
                    );
                    out.push_str(&format!("        kani::assume({});\n", substituted));
                }
            }

            out.push_str(&format!("        assert!({},\n", ensures.rust_expr_binary));
            out.push_str(&format!(
                "            \"ensures clause {} on {} violated by spec-translated transition\");\n",
                idx, op.name
            ));
            out.push_str("    }\n");
            out.push_str("}\n\n");

            // Reference `pre` even when the if-branch isn't taken so
            // the spec-model `let pre = s.clone();` doesn't trigger
            // an unused-variable warning. Mirrors the legacy
            // behavior (kani.rs marks `pre` with `_` only when the
            // handler has zero calls AND zero references in the
            // ensures string — too brittle; matching legacy verbatim).
            let _ = pre_unused_workaround_needed(ensures);
        }
    }

    Ok(())
}

/// Stub placeholder for the `pre`-binding lint workaround. Legacy
/// doesn't actually emit any extra code here; the call exists only
/// to mark our intent to match its byte-exact shape. Returns false.
fn pre_unused_workaround_needed(_ensures: &crate::check::ParsedEnsures) -> bool {
    false
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c4 invariant-preservation harnesses
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_(preserves|establishes)_<invariant>()`
/// for every handler × invariant-clause pair. Mirrors
/// `kani::emit_kani_account_section` lines ~956–1063.
///
/// Per-pair harness shape:
///   * Iterate `op.invariants` (preserves, is_establish=false) ∪
///     `op.establishes` (is_establish=true)
///   * Skip invariants whose `rust_expr` is missing / unsupported
///   * Pre-state: zeroed for init handlers, symbolic for non-init
///   * Non-init preserves: `kani::assume(<inv>(&s))` to scope BMC to
///     states where the invariant already holds
///   * Non-init establishes: skip the pre-assume (handler is supposed
///     to *make* the invariant true regardless of pre-state)
///   * Symbolic params + abstract binders (single call — matches
///     property-preservation, not the double-emit bug of guard/abort)
///   * `if <handler>(&mut s, ...) { assert!(<inv>(&s)); }`
fn emit_invariant_preservation_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();

    // `linked_invs` — invariants referenced by at least one handler
    // in this section (matches the section-structural filter for
    // single-account; the multi-account `mod`-wrapping case is
    // Phase 3e).
    let linked_invs: Vec<&crate::check::ParsedInvariant> = parsed
        .invariants
        .iter()
        .filter(|i| {
            handlers
                .iter()
                .any(|h| h.invariants.contains(&i.name) || h.establishes.contains(&i.name))
        })
        .collect();

    if linked_invs.is_empty() {
        return Ok(());
    }

    // Resolve view — same logic as `emit_account_section_structural`.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Invariant preservation — `invariant Name` on a handler asserts the named\n");
    out.push_str("// top-level invariant holds before AND after the handler runs. Each pair\n");
    out.push_str("// becomes its own BMC proof.\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    for op in &handlers {
        // Build the `(invariant_name, is_establish)` pair list.
        let pairs: Vec<(&String, bool)> = op
            .invariants
            .iter()
            .map(|n| (n, false))
            .chain(op.establishes.iter().map(|n| (n, true)))
            .collect();

        for (inv_name, is_establish) in pairs {
            let Some(inv) = linked_invs.iter().find(|i| &i.name == inv_name) else {
                continue;
            };
            // Skip invariants whose body is missing or unsupported
            // (e.g. mentions `QEDGEN_UNSUPPORTED_QUANTIFIER`).
            if inv
                .rust_expr
                .as_deref()
                .map(crate::check::rust_expr_is_unsupported)
                .unwrap_or(true)
            {
                continue;
            }

            let is_init = op.pre_status.as_deref() == Some("Uninitialized");

            out.push_str("#[kani::proof]\n");
            out.push_str("#[kani::unwind(2)]\n");
            out.push_str("#[kani::solver(cadical)]\n");
            let verb = if is_establish {
                "establishes"
            } else {
                "preserves"
            };
            out.push_str(&format!(
                "fn verify_{}_{}_{}() {{\n",
                op.name, verb, inv.name
            ));

            if is_init {
                util::emit_state_init_zeroed(out, &mutable, lifecycle, parsed);
            } else {
                util::emit_state_init_symbolic(out, &mutable, lifecycle);
                util::emit_pre_status_assume(out, op, lifecycle);
                if !is_establish {
                    out.push_str(&format!("    kani::assume({}(&s));\n", inv.name));
                }
            }

            for (pname, ptype) in &op.takes_params {
                out.push_str(&format!(
                    "    let {}: {} = kani::any();\n",
                    pname,
                    map_type(ptype, parsed)?
                ));
            }
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

            let args: String = op
                .takes_params
                .iter()
                .chain(op.abstract_binders.iter())
                .map(|(n, _)| format!(", {}", n))
                .collect();
            out.push_str(&format!("    if {}(&mut s{}) {{\n", op.name, args));
            out.push_str(&format!("        assert!({}(&s),\n", inv.name));
            out.push_str(&format!(
                "            \"invariant {} must hold after {}\");\n",
                inv.name, op.name
            ));
            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c5 effect-conformance harnesses
// ----------------------------------------------------------------------

/// Emit per-(handler, field) effect-conformance harnesses. Mirrors
/// `kani::emit_kani_account_section` lines ~1065–1277. The B11 v2.6
/// split — one proof per `(handler, field)` pair instead of one per
/// handler — keeps a single stuck mul/div field from blocking sibling
/// field verification. Solver choice per harness via
/// `rust_codegen_util::pick_kani_solver_for_effect`:
///   * cadical — scalar / linear (default)
///   * minisat — narrow-type (u8/u16/u32) mul/div
///   * bin="z3" — wide-type (u64/u128/i128) mul/div
///
/// Per-(handler, field) body:
///   * Skip if `field`'s base isn't in this section's State
///     (multi-account safety: effects targeting another account's
///     field).
///   * Pre-state: zeroed for init handlers, symbolic for non-init.
///   * Symbolic params + abstract binders.
///   * Non-init: MAX_MEMBERS-style assume + `emit_add_strict_bounds`
///     on pre-state.
///   * `let pre_<F> = s.<F>;` snapshot for every mutable field
///     (skip the target field when op_kind == "set" — `set` doesn't
///     need a pre on its own target).
///   * `if <handler>(&mut s, args) { assert ... }`:
///     - set → `s.F == <resolved>`
///     - add → `s.F == pre_F.wrapping_add(<resolved>)`
///     - sub → `s.F == pre_F.wrapping_sub(<resolved>)`
///   * Sibling fields assert `s.G == pre_G` unless another effect in
///     the same handler mutates them.
fn emit_effect_conformance_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::{map_type, sanitize_ident};
    use crate::rust_codegen_util as util;

    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();

    let effect_ops: Vec<&crate::check::ParsedHandler> = handlers
        .iter()
        .copied()
        .filter(|op| op.has_effect())
        .collect();

    if effect_ops.is_empty() {
        return Ok(());
    }

    // Resolve view.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);
    let properties: Vec<&crate::check::ParsedProperty> = parsed.properties.iter().collect();

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Effect conformance — verify transition effects match spec\n");
    out.push_str("//\n");
    out.push_str("// Each proof applies a transition to symbolic state and checks that every\n");
    out.push_str("// field changed/unchanged matches the spec's effect: declarations.\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    let field_type_lookup: std::collections::HashMap<&str, &str> = mutable
        .iter()
        .map(|(n, t)| (n.as_str(), t.as_str()))
        .collect();

    for op in &effect_ops {
        let is_init = op.pre_status.as_deref() == Some("Uninitialized");

        for (field, op_kind, value) in &op.effects {
            let base = util::effect_target_base(field);
            if !field_type_lookup.contains_key(base) {
                continue;
            }

            let field_type = field_type_lookup.get(field.as_str()).copied().unwrap_or("");
            let solver = util::pick_kani_solver_for_effect(field_type, value, op);

            out.push_str("#[kani::proof]\n");
            out.push_str("#[kani::unwind(2)]\n");
            out.push_str(&format!("#[kani::solver({})]\n", solver));
            out.push_str(&format!(
                "fn verify_{}_effect_{}() {{\n",
                op.name,
                sanitize_ident(field)
            ));

            if is_init {
                util::emit_state_init_zeroed(out, &mutable, lifecycle, parsed);
            } else {
                util::emit_state_init_symbolic(out, &mutable, lifecycle);
                util::emit_pre_status_assume(out, op, lifecycle);
            }

            for (pname, ptype) in &op.takes_params {
                out.push_str(&format!(
                    "    let {}: {} = kani::any();\n",
                    pname,
                    map_type(ptype, parsed)?
                ));
            }
            util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

            // Bounds assumptions for arithmetic safety (non-init only).
            if !is_init {
                if !parsed.constants.is_empty() {
                    for (cname, _) in &parsed.constants {
                        let upper = cname.to_uppercase();
                        if upper.contains("MAX") || upper.contains("MEMBER") {
                            if mutable.iter().any(|(f, _)| f == "member_count") {
                                out.push_str(&format!(
                                    "    kani::assume(s.member_count <= {});\n",
                                    upper
                                ));
                            }
                            break;
                        }
                    }
                }
                let owned_props: Vec<crate::check::ParsedProperty> =
                    properties.iter().map(|p| (*p).clone()).collect();
                util::emit_add_strict_bounds(
                    out,
                    op,
                    &owned_props,
                    "    kani::assume(s.{field} < s.{bound}); // strict bound: {field} increments\n",
                );
            }

            // Pre-state snapshot — every mutable field except the
            // set-target.
            let needs_pre_for: Vec<&&(String, String)> = mutable
                .iter()
                .filter(|(fname, _)| !(fname.as_str() == field.as_str() && op_kind == "set"))
                .collect();
            for (fname, _) in &needs_pre_for {
                out.push_str(&format!("    let pre_{} = s.{};\n", fname, fname));
            }

            // Call transition + assertion dispatch.
            let args: String = op
                .takes_params
                .iter()
                .chain(op.abstract_binders.iter())
                .map(|(n, _)| format!(", {}", n))
                .collect();
            out.push_str(&format!("    if {}(&mut s{}) {{\n", op.name, args));

            let resolved = util::resolve_value(value, op, parsed, Some("pre_"));
            match op_kind.as_str() {
                "set" => {
                    out.push_str(&format!(
                        "        assert!(s.{} == {}, \"{} must equal {}\");\n",
                        field, resolved, field, resolved
                    ));
                }
                "add" => {
                    out.push_str(&format!(
                        "        assert!(s.{} == pre_{}.wrapping_add({}), \"{} must increment by {}\");\n",
                        field, field, resolved, field, resolved
                    ));
                }
                "sub" => {
                    out.push_str(&format!(
                        "        assert!(s.{} == pre_{}.wrapping_sub({}), \"{} must decrement by {}\");\n",
                        field, field, resolved, field, resolved
                    ));
                }
                _ => {}
            }

            // Assert sibling fields unchanged (unless mutated by another
            // effect in the same handler).
            for (fname, _) in &mutable {
                if fname.as_str() != field.as_str() {
                    let sibling_mutated = op
                        .effects
                        .iter()
                        .any(|(f, _, _)| f.as_str() == fname.as_str());
                    if !sibling_mutated {
                        out.push_str(&format!(
                            "        assert!(s.{} == pre_{}, \"{} must not change\");\n",
                            fname, fname, fname
                        ));
                    }
                }
            }

            out.push_str("    }\n");
            out.push_str("}\n\n");
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3c6 overflow-detection harnesses
// ----------------------------------------------------------------------

/// Emit `#[kani::proof] fn verify_<handler>_no_overflow()` for every
/// handler with an `add` effect. Kani's built-in overflow detection
/// fires on `+=` inside the transition body — no explicit assert
/// required; the proof exists to drive BMC across the parameter
/// space. Mirrors `kani::emit_kani_account_section` lines ~1279–1330.
fn emit_overflow_detection_harnesses(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    let handlers: Vec<&crate::check::ParsedHandler> = parsed.handlers.iter().collect();

    let overflow_ops: Vec<&crate::check::ParsedHandler> = handlers
        .iter()
        .copied()
        .filter(|op| op.effects.iter().any(|(_, kind, _)| kind == "add"))
        .collect();

    if overflow_ops.is_empty() {
        return Ok(());
    }

    // Resolve view.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else if parsed.account_types.is_empty() {
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        } else {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);

    out.push_str(
        "// ============================================================================\n",
    );
    out.push_str("// Overflow detection — Kani catches arithmetic overflow on add effects\n");
    out.push_str(
        "// ============================================================================\n\n",
    );

    for op in &overflow_ops {
        out.push_str("#[kani::proof]\n");
        out.push_str("#[kani::unwind(2)]\n");
        out.push_str("#[kani::solver(cadical)]\n");
        out.push_str(&format!("fn verify_{}_no_overflow() {{\n", op.name));

        util::emit_state_init_symbolic(out, &mutable, lifecycle);
        util::emit_pre_status_assume(out, op, lifecycle);

        for (pname, ptype) in &op.takes_params {
            out.push_str(&format!(
                "    let {}: {} = kani::any();\n",
                pname,
                map_type(ptype, parsed)?
            ));
        }
        util::emit_abstract_binders(out, op, "    ", "kani::any()", |t| map_type(t, parsed))?;

        let args: String = op
            .takes_params
            .iter()
            .chain(op.abstract_binders.iter())
            .map(|(n, _)| format!(", {}", n))
            .collect();
        out.push_str(&format!(
            "    {}(&mut s{});  // Kani detects overflow on += internally\n",
            op.name, args
        ));
        out.push_str("}\n\n");
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Section emitters — Phase 3d file-level features (single-mode only)
// ----------------------------------------------------------------------

/// Emit covers / liveness / environment harnesses at file scope.
/// Mirrors `kani::emit_file_level_features` (lines ~1339–1558).
/// These reference handlers by name and the per-spec `State`
/// directly, so they only fire in single-account mode where there's
/// a unique top-level `fn deposit(...)`. Multi-account specs skip
/// (per-ADT lifting is v2.22 scope per the legacy comment; Phase 3e
/// closes the multi-account wrapping).
///
/// Three sub-sections in order:
///   1. **Cover properties (reachability)** — per `(cover, trace)`
///      pair, build a nested `if` chain over the trace handlers and
///      cap with `kani::cover!(<last_op>(...), "trace is reachable")`.
///   2. **Liveness properties (bounded reachability)** — per
///      liveness, assume the from-state, run a `for _ in 0..bound`
///      loop dispatching to via_ops by non-deterministic `op: u8`,
///      then `kani::cover!(s.status == Status::<to_state>)`. Skipped
///      when the spec has no lifecycle (no target predicate to
///      cover) — emit a structured skip comment instead.
///   3. **Environment property harnesses** — per `(env, property)`
///      cross, init symbolic state, assume the property holds pre,
///      mutate `env.mutates` fields to `kani::any()`, assume the
///      constraints, then `assert!(<prop>(&s))`.
fn emit_file_level_features(out: &mut String, parsed: &ParsedSpec) -> Result<()> {
    use crate::codegen::map_type;
    use crate::rust_codegen_util as util;

    // Resolve view — same logic as `emit_account_section_structural`.
    let (state_fields, lifecycle): (&[(String, String)], &[String]) =
        if parsed.account_types.len() == 1 {
            (
                &parsed.account_types[0].fields,
                parsed.account_types[0].lifecycle.as_slice(),
            )
        } else {
            // Zero account types — flat state form.
            (
                util::resolve_state_fields(parsed),
                parsed.lifecycle_states.as_slice(),
            )
        };
    let mutable = util::mutable_fields(state_fields);
    let has_lifecycle = lifecycle.len() >= 2;

    // ── Cover properties ──────────────────────────────────────────
    if !parsed.covers.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Cover properties — reachability via kani::cover!\n");
        out.push_str(
            "// ============================================================================\n\n",
        );

        for cover in &parsed.covers {
            for (i, trace) in cover.traces.iter().enumerate() {
                let suffix = if cover.traces.len() > 1 {
                    format!("_{}", i)
                } else {
                    String::new()
                };
                out.push_str("#[kani::proof]\n");
                let unwind = trace.len() + 1;
                out.push_str(&format!("#[kani::unwind({})]\n", unwind));
                out.push_str("#[kani::solver(cadical)]\n");
                out.push_str(&format!("fn cover_{}{}() {{\n", cover.name, suffix));

                util::emit_state_init_symbolic(out, &mutable, lifecycle);

                // Nested-if chain over trace handlers; the last
                // step caps with `kani::cover!`.
                let mut indent = "    ".to_string();
                for (j, op_name) in trace.iter().enumerate() {
                    let op = parsed.handlers.iter().find(|o| o.name == *op_name);
                    if let Some(op) = op {
                        for (pname, ptype) in &op.takes_params {
                            out.push_str(&format!(
                                "{}let {}_{}: {} = kani::any();\n",
                                indent,
                                pname,
                                j,
                                map_type(ptype, parsed)?
                            ));
                        }
                    }
                    let args: String = op
                        .map(|o| {
                            o.takes_params
                                .iter()
                                .map(|(n, _)| format!(", {}_{}", n, j))
                                .collect()
                        })
                        .unwrap_or_default();

                    if j < trace.len() - 1 {
                        out.push_str(&format!("{}if {}(&mut s{}) {{\n", indent, op_name, args));
                        indent.push_str("    ");
                    } else {
                        out.push_str(&format!(
                            "{}kani::cover!({}(&mut s{}), \"{} trace is reachable\");\n",
                            indent, op_name, args, cover.name
                        ));
                    }
                }
                // Close braces (one less than trace length).
                for _ in 0..trace.len().saturating_sub(1) {
                    indent = indent[..indent.len() - 4].to_string();
                    out.push_str(&format!("{}}}\n", indent));
                }
                out.push_str("}\n\n");
            }
        }
    }

    // ── Liveness properties ──────────────────────────────────────
    if !parsed.liveness_props.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Liveness properties — bounded reachability via non-deterministic ops\n");
        out.push_str(
            "// ============================================================================\n\n",
        );

        for liveness in &parsed.liveness_props {
            let bound = liveness.within_steps.unwrap_or(10) as usize;

            // No lifecycle → no target predicate. Skip with a
            // structured comment (matches legacy line ~1434–1440).
            if !has_lifecycle {
                out.push_str(&format!(
                    "// liveness {}: skipped — spec has no lifecycle, no target predicate to cover\n\n",
                    liveness.name
                ));
                continue;
            }

            out.push_str("#[kani::proof]\n");
            out.push_str(&format!("#[kani::unwind({})]\n", bound + 1));
            out.push_str("#[kani::solver(cadical)]\n");
            out.push_str(&format!("fn verify_liveness_{}() {{\n", liveness.name));

            util::emit_state_init_symbolic(out, &mutable, lifecycle);

            // Assume the from-state so via-ops can fire.
            out.push_str(&format!(
                "    kani::assume(s.status == Status::{});\n",
                liveness.from_state
            ));

            let via_ops = &liveness.via_ops;
            out.push_str(&format!("    for _ in 0..{} {{\n", bound));
            out.push_str("        let op: u8 = kani::any();\n");
            out.push_str("        match op {\n");
            for (i, op_name) in via_ops.iter().enumerate() {
                let op = parsed.handlers.iter().find(|o| o.name == *op_name);
                let param_decls: String = match op {
                    Some(o) => o
                        .takes_params
                        .iter()
                        .map(|(n, t)| {
                            map_type(t, parsed)
                                .map(|rt| format!("            let {}: {} = kani::any();\n", n, rt))
                        })
                        .collect::<anyhow::Result<String>>()?,
                    None => String::new(),
                };
                let args: String = op
                    .map(|o| {
                        o.takes_params
                            .iter()
                            .chain(o.abstract_binders.iter())
                            .map(|(n, _)| format!(", {}", n))
                            .collect()
                    })
                    .unwrap_or_default();

                out.push_str(&format!("            {} => {{\n", i));
                out.push_str(&param_decls);
                out.push_str(&format!("                {}(&mut s{});\n", op_name, args));
                out.push_str("            }\n");
            }
            out.push_str("            _ => {}\n");
            out.push_str("        }\n");
            out.push_str("    }\n");

            out.push_str(&format!(
                "    kani::cover!(s.status == Status::{}, \"{} reaches {} within {} steps\");\n",
                liveness.leads_to_state, liveness.name, liveness.leads_to_state, bound
            ));
            out.push_str("}\n\n");
        }
    }

    // ── Environment harnesses ────────────────────────────────────
    if !parsed.environments.is_empty() {
        out.push_str(
            "// ============================================================================\n",
        );
        out.push_str("// Environment — properties hold under external state changes\n");
        out.push_str(
            "// ============================================================================\n\n",
        );

        for env in &parsed.environments {
            for prop in &parsed.properties {
                if prop.expression.is_none() {
                    continue;
                }
                let rust_constraints: &[String] = &env.constraints_rust;

                out.push_str("#[kani::proof]\n");
                out.push_str("#[kani::unwind(2)]\n");
                out.push_str("#[kani::solver(cadical)]\n");
                out.push_str(&format!(
                    "fn verify_{}_under_{}() {{\n",
                    prop.name, env.name
                ));

                util::emit_state_init_symbolic(out, &mutable, lifecycle);
                out.push_str(&format!("    kani::assume({}(&s));\n", prop.name));

                for (field, ftype) in &env.mutates {
                    out.push_str(&format!("    s.{} = kani::any();\n", field));
                    let _ = ftype;
                }
                for constraint in rust_constraints {
                    out.push_str(&format!("    kani::assume({});\n", constraint));
                }

                out.push_str(&format!("    assert!({}(&s),\n", prop.name));
                out.push_str(&format!(
                    "        \"{} must hold after {}\");\n",
                    prop.name, env.name
                ));
                out.push_str("}\n\n");
            }
        }
    }

    Ok(())
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::check;
    use std::path::Path;

    fn lower_fixture(rel_path: &str) -> (Mir, ParsedSpec) {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"))
            .parent()
            .and_then(Path::parent)
            .expect("crates/qedgen/ under repo root");
        let spec_path = root.join(rel_path);
        let parsed = check::parse_spec_file(&spec_path).expect("fixture parses");
        let mir = crate::mir::lower(&parsed);
        (mir, parsed)
    }

    #[test]
    fn render_emits_file_header_and_cfg_kani() {
        // The structural prefix is deterministic — every pilot
        // fixture produces the same banner + `#![cfg(kani)]` line.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.starts_with("// ---- GENERATED BY QEDGEN"),
            "expected banner-style first line; got:\n{}",
            &out[..out.len().min(200)]
        );
        assert!(
            out.contains("#![cfg(kani)]"),
            "expected #![cfg(kani)] attribute"
        );
        assert!(
            out.contains("Self-contained Kani proof harnesses for the spec."),
            "expected legacy file-header docstring"
        );
    }

    #[test]
    fn render_emits_state_model_header_banner() {
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// State model (derived from qedspec"),
            "expected state-model section header"
        );
    }

    #[test]
    fn render_emits_constants_when_spec_declares_them() {
        // percolator declares MAX_ACCOUNTS, POS_SCALE — Mir.constants
        // carries them as (name, value) and `emit_constants` lowers
        // them to `pub const NAME: u64 = VALUE;`.
        let (mir, parsed) = lower_fixture("examples/rust/percolator/percolator.qedspec");
        let out = render(&mir, &parsed);
        // `rust_codegen_util::emit_constants` writes `const NAME:
        // <ty> = VALUE;` (file-scoped, no `pub` — the per-ADT modules
        // pull them in via `use super::*`).
        assert!(
            out.contains("const MAX_ACCOUNTS"),
            "expected MAX_ACCOUNTS constant emit"
        );
        assert!(
            out.contains("const POS_SCALE"),
            "expected POS_SCALE constant emit"
        );
    }

    #[test]
    fn render_emits_phase_3e_trailing_todo_marker() {
        // Post Phase 3d the only remaining gap is multi-account
        // `mod <name>` wrapping. Every rendered file carries the
        // trailing phase-3e marker so users know what's missing.
        // (Multi-account specs also get the leading phase-3e marker
        // — see `render_emits_phase_3e_marker_for_multi_account`.)
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("MIR-TODO(phase-3e):"),
            "expected phase-3e TODO marker"
        );
    }

    #[test]
    fn render_emits_cover_harnesses_single_mode() {
        // Phase 3d — covers fire in single-account mode. Escrow has
        // `cover initialize_then_close [initialize, exchange]` so
        // the section emits.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Cover properties —"),
            "expected cover-properties section header"
        );
        assert!(
            out.contains("fn cover_"),
            "expected at least one cover_<name> harness"
        );
    }

    #[test]
    fn render_skips_file_level_features_for_multi_account() {
        // Phase 3d — multi-account specs skip file-level features
        // (matches legacy gating). Lending is multi-account, so
        // even if it declared covers/liveness/env, the section
        // headers must NOT emit.
        let (mir, parsed) = lower_fixture("examples/rust/lending/lending.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            !out.contains("// Cover properties — reachability via kani::cover!"),
            "multi-account specs must skip file-level cover section"
        );
        assert!(
            !out.contains(
                "// Liveness properties — bounded reachability via non-deterministic ops"
            ),
            "multi-account specs must skip file-level liveness section"
        );
        assert!(
            !out.contains("// Environment — properties hold under external state changes"),
            "multi-account specs must skip file-level environment section"
        );
    }

    #[test]
    fn render_emits_effect_conformance_harnesses() {
        // Phase 3c5 — `emit_effect_conformance_harnesses` fires per
        // `(handler, field)` pair for every handler with effects.
        // Escrow's `initialize` has `:= deposit_amount` / `:=
        // receive_amount` / `:= Open` effects so it emits.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Effect conformance —"),
            "expected effect-conformance section header"
        );
        assert!(
            out.contains("fn verify_initialize_effect_"),
            "expected initialize_effect_<field> harness"
        );
    }

    #[test]
    fn render_emits_overflow_detection_harnesses_for_add_effects() {
        // Phase 3c6 — `emit_overflow_detection_harnesses` fires per
        // handler with at least one `add` effect.
        // bundled-stdlib-demo's `deposit` has `total_assets +=
        // amount`, so the overflow harness emits.
        let (mir, parsed) = lower_fixture("examples/rust/bundled-stdlib-demo/pool.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Overflow detection —"),
            "expected overflow-detection section header"
        );
        assert!(
            out.contains("fn verify_deposit_no_overflow()"),
            "expected verify_deposit_no_overflow harness"
        );
    }

    #[test]
    fn render_emits_ensures_preservation_harnesses() {
        // Phase 3c7 — `emit_ensures_preservation_harnesses` fires
        // per (handler, ensures clause). Percolator has handlers
        // with ensures clauses (e.g. deposit with old(...) bindings).
        let (mir, parsed) = lower_fixture("examples/rust/percolator/percolator.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Ensures preservation —"),
            "expected ensures-preservation section header"
        );
        assert!(
            out.contains("_ensures_"),
            "expected at least one _ensures_<idx> harness"
        );
    }

    #[test]
    fn render_emits_no_invariant_preservation_section_when_no_clauses() {
        // Phase 3c4 — `emit_invariant_preservation_harnesses` only
        // fires when at least one handler carries `invariant Name`
        // or `establishes Name` clauses. Current pilots don't use
        // these (`invariant` declarations exist but aren't claimed
        // by handlers), so the section header doesn't emit.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            !out.contains("// Invariant preservation —"),
            "expected no invariant-preservation section for pilots without handler invariant clauses"
        );
    }

    #[test]
    fn render_emits_property_preservation_harnesses() {
        // Multisig declares `property votes_bounded` with
        // `preserved_by`. The Phase 3c3 emitter fires one
        // `verify_<handler>_preserves_votes_bounded()` per matched
        // handler. Section header is constant; the per-pair body is
        // covered by the byte-equivalence sweep against legacy.
        let (mir, parsed) = lower_fixture("examples/rust/multisig/multisig.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Property preservation —"),
            "expected property-preservation section header"
        );
        assert!(
            out.contains("_preserves_"),
            "expected at least one preserves_<prop> harness"
        );
    }

    #[test]
    fn render_emits_no_abort_section_when_no_aborts_if() {
        // Phase 3c2 — `emit_abort_condition_harnesses` only fires when
        // `op.aborts_if` is non-empty, which is the direct
        // `aborts_if Pred Error` DSL form. Current pilots use
        // `requires X else Err` which lowers to a different field
        // (`requires_or_abort`), so the section header doesn't emit.
        // This asserts the no-op behavior matches legacy.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            !out.contains("// Abort conditions —"),
            "expected no abort-conditions section for pilots without `aborts_if`"
        );
    }

    #[test]
    fn render_emits_guard_enforcement_harnesses() {
        // Phase 3c1 — emit_guard_enforcement_harnesses fires one
        // `verify_<handler>_rejects_invalid()` per guard-bearing
        // handler. Escrow's `initialize` has `requires deposit_amount
        // > 0 && receive_amount > 0`, so the harness emits.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("// Guard enforcement"),
            "expected guard-enforcement section header"
        );
        assert!(
            out.contains("fn verify_initialize_rejects_invalid()"),
            "expected initialize rejects_invalid harness"
        );
        assert!(
            out.contains("kani::assume(!("),
            "expected `kani::assume(!(guard))` negation"
        );
        assert!(
            out.contains("\"initialize must reject when guard is violated\""),
            "expected assert message"
        );
    }

    #[test]
    fn render_emits_state_struct_for_single_account() {
        // Phase 3b — `emit_account_section_structural` delegates to
        // `rust_codegen_util::emit_state_struct_with_lifecycle`, which
        // emits `struct State { ... }` with mutable fields + optional
        // `status: Status`. Escrow has lifecycle states so it gets the
        // status field.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(out.contains("struct State {"), "expected State struct");
        assert!(out.contains("status: Status"), "expected status field");
        // Transition fns mirror the spec's handler set.
        assert!(
            out.contains("fn initialize(s: &mut State"),
            "expected initialize transition fn"
        );
        assert!(
            out.contains("fn exchange(s: &mut State"),
            "expected exchange transition fn"
        );
        assert!(
            out.contains("fn cancel(s: &mut State"),
            "expected cancel transition fn"
        );
    }

    #[test]
    fn render_emits_phase_3e_marker_for_multi_account() {
        // Lending declares Pool + Loan account types — multi-account
        // wrapping (`mod <name> { ... }`) is Phase 3e. Until then,
        // the structural body still emits for the primary account
        // (best-effort) prefixed by the phase-3e marker.
        let (mir, parsed) = lower_fixture("examples/rust/lending/lending.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("MIR-TODO(phase-3e)"),
            "expected phase-3e multi-account marker"
        );
    }
}
