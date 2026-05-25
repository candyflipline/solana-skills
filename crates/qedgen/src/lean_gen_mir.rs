// Phase 1 (v2.30) MIR-based Lean codegen. Lives in parallel to
// `lean_gen.rs` until snapshot equivalence is validated on every pilot
// fixture; then `lean_gen.rs` is retired.
//
// Dead-code warnings during incremental wiring.
#![allow(dead_code)]

//! qedgen Lean codegen — MIR consumer.
//!
//! This is Phase 1 of the v2.30 refactor. The existing `lean_gen.rs`
//! (8,661 LoC) consumes `ParsedSpec` directly; this module replicates
//! the same emitted output but consumes `mir::Mir`. The flag
//! `QEDGEN_USE_MIR=1` switches the `qedgen codegen --lean` call site to
//! this module; without the flag, legacy `lean_gen` runs.
//!
//! ## Phase 1a survey — what we must replicate
//!
//! `lean_gen::generate(spec, output_path)` does:
//! 1. Compute `content = render(spec)`.
//! 2. Inject `import <Iface>` lines for pinned interface modules.
//! 3. Write `Spec.lean` at `output_path`.
//! 4. For each pinned-but-unverified interface, write a sibling
//!    `<Iface>.lean` axiom module under the same dir.
//! 5. Update `lakefile.lean`'s `roots` array.
//!
//! `lean_gen::render(spec) -> String` dispatches based on spec shape:
//! - sBPF (`pragma sbpf`) → `render_sbpf` — out of v2.30 scope.
//! - Indexed (records or `Map[N] T`) → `render_indexed_state` — Phase 1
//!   stretch goal.
//! - Multi-account (`account_types.len() > 1`) → `render_multi_account` —
//!   Phase 2.
//! - Single-account → `render_single_account` (pilot path).
//!
//! `render_single_account` emits the following sections in order:
//! 1. `import QEDGen.Solana.{Account, Cpi, State, Valid}`.
//! 2. `namespace <ProgramName>` + `open QEDGen.Solana`.
//! 3. Uninterpreted helpers + ref-impls (`emit_uninterpreted_helpers`,
//!    `emit_ref_impls`).
//! 4. Constants (`abbrev NAME : Nat := VALUE`).
//! 5. `inductive Status` if ≥2 lifecycle states.
//! 6. `structure State` with all state fields.
//! 7. Transition functions (`render_transitions`) — one `def
//!    <handler>_transition (s : State) ... : Option State` per handler.
//! 8. CPI theorems (`render_cpi_theorems`) — per-handler `theorem
//!    <handler>_cpi_correct` for Tier-1/2 callees.
//! 9. Invariant theorems (`render_invariants_theorem_form`).
//! 10. `inductive Operation` + `def applyOp` — the union of all
//!     handlers.
//! 11. Property theorems (`render_properties`).
//! 12. Abort theorems (`render_aborts_if`).
//! 13. Ensures theorems (`render_ensures`).
//! 14. Frame conditions (`render_frame_conditions`).
//! 15. Cover / liveness / environment / overflow theorems.
//! 16. `end <ProgramName>`.
//!
//! Multi-variant ADT specs (`type State | A | B of { ... }`) take a
//! different branch (`render_single_account_adt`) — those land later in
//! Phase 1.
//!
//! ## Pilot scope for this phase
//!
//! Sections 1–6 (file structure, namespace, state struct) +
//! transitions for the pilot `Stmt` set (RequireOrAbort, Assign,
//! CheckedAdd/Sub, WrapAdd/Sub, SatAdd/Sub, TokenTransfer →
//! CPI theorem) + the lifecycle gate from `HandlerMir.transition`.
//!
//! Sections 9–16 (invariants, properties, aborts, ensures, frame,
//! cover, liveness, environments, overflow) are stubbed for the first
//! sub-pass and filled in iteratively. The snapshot equivalence gate
//! (Phase 1d) drives which sections must land for which fixtures.

use crate::mir::Mir;
use anyhow::Result;
use std::path::Path;

/// Top-level entry — mirrors `lean_gen::generate`. Writes `Spec.lean`
/// and sibling axiom modules at `output_path`.
///
/// Phase 1 sub-pass: implements the file-write side-effect; the
/// `render` body is incomplete (see below). Sibling axiom modules and
/// lakefile updates are not yet wired — they come back when CPI
/// theorem emission lands.
pub fn generate(mir: &Mir, output_path: &Path) -> Result<()> {
    let content = render(mir);
    if let Some(parent) = output_path.parent() {
        std::fs::create_dir_all(parent)?;
    }
    std::fs::write(output_path, &content)?;
    eprintln!("  wrote {} (MIR codegen)", output_path.display());
    Ok(())
}

/// Pure render. Dispatches based on the MIR shape and emits the full
/// Spec.lean as a String.
///
/// Phase 1 stub: emits header + namespace + state struct only. Other
/// sections land iteratively as Phase 1c progresses.
pub fn render(mir: &Mir) -> String {
    // Dispatch by spec shape — sBPF, indexed, multi-account, single
    // — mirrors `lean_gen::render`'s top-level branch logic. Phase 1
    // pilot only implements `render_single_account`; the others
    // delegate to TODO stubs that emit a marker comment so call sites
    // are obvious.
    if is_sbpf(mir) {
        return render_sbpf_stub(mir);
    }
    if is_indexed(mir) {
        return render_indexed_stub(mir);
    }
    if is_multi_account(mir) {
        return render_multi_account_stub(mir);
    }
    render_single_account(mir)
}

// ----------------------------------------------------------------------
// Shape detection — mirrors lean_gen.rs predicates
// ----------------------------------------------------------------------

fn is_sbpf(_mir: &Mir) -> bool {
    // sBPF specs declare `pragma sbpf { ... }`; the MIR doesn't carry
    // pragma info yet (Phase 0 didn't lift it). v3.0 will. For now,
    // assume non-sBPF — sBPF specs aren't in the v2.30 pilot scope.
    false
}

fn is_indexed(mir: &Mir) -> bool {
    // Indexed spec: declares records (modeled as `Custom` types in MIR)
    // or uses `Map[N]` fields. Detect by scanning state-field types.
    mir.state.variants.iter().any(|v| {
        v.fields
            .iter()
            .any(|f| matches!(&f.ty, crate::mir::Ty::Map { .. }))
    })
}

fn is_multi_account(mir: &Mir) -> bool {
    // Multi-account specs declare > 1 `type Account` block. MIR
    // collapses to a single `StateAdt` today; multi-account support
    // requires Phase 2's MIR extension. For pilot, single-account only.
    // Detection placeholder — returns false until MIR carries
    // multi-account info.
    let _ = mir;
    false
}

// ----------------------------------------------------------------------
// Shape-specific renderers
// ----------------------------------------------------------------------

fn render_single_account(mir: &Mir) -> String {
    let mut out = String::new();
    emit_header(&mut out, mir);
    emit_namespace_open(&mut out, mir);
    emit_constants(&mut out, mir);
    emit_lifecycle_marker(&mut out, mir);
    emit_state_struct(&mut out, mir);

    // TODO Phase 1c: transitions, CPI theorems, invariants,
    // operation inductive + applyOp, properties, aborts, ensures,
    // frame, cover, liveness, environments, overflow obligations.
    // Sections below are placeholders so the generated file has a
    // clear shape during incremental wiring.
    out.push_str("-- TODO(mir-phase-1c): transitions\n\n");
    out.push_str("-- TODO(mir-phase-1c): CPI theorems\n\n");
    out.push_str("-- TODO(mir-phase-1c): invariants\n\n");
    out.push_str("-- TODO(mir-phase-1c): Operation + applyOp\n\n");
    out.push_str("-- TODO(mir-phase-1c): properties / aborts / ensures / frame\n\n");

    emit_namespace_close(&mut out, mir);
    out
}

fn render_sbpf_stub(_mir: &Mir) -> String {
    "-- MIR-TODO(phase-?): sBPF codegen not yet ported to MIR\n".to_string()
}

fn render_indexed_stub(_mir: &Mir) -> String {
    "-- MIR-TODO(phase-1-stretch): indexed-state codegen not yet ported\n".to_string()
}

fn render_multi_account_stub(_mir: &Mir) -> String {
    "-- MIR-TODO(phase-2): multi-account codegen not yet ported\n".to_string()
}

// ----------------------------------------------------------------------
// Section emitters (called from render_single_account)
// ----------------------------------------------------------------------

fn emit_header(out: &mut String, _mir: &Mir) {
    out.push_str("import QEDGen.Solana.Account\n");
    out.push_str("import QEDGen.Solana.Cpi\n");
    out.push_str("import QEDGen.Solana.State\n");
    out.push_str("import QEDGen.Solana.Valid\n\n");
}

fn emit_namespace_open(out: &mut String, mir: &Mir) {
    out.push_str(&format!("namespace {}\n\n", mir.name));
    out.push_str("open QEDGen.Solana\n\n");
}

fn emit_namespace_close(out: &mut String, mir: &Mir) {
    out.push_str(&format!("end {}\n", mir.name));
}

fn emit_constants(_out: &mut String, _mir: &Mir) {
    // TODO(mir-phase-1c): MIR doesn't yet carry `ParsedSpec.constants`.
    // Lift in a future Phase 0 patch + emit `abbrev NAME : Nat := VALUE`
    // lines here. For pilot fixtures that don't declare constants
    // (escrow, lending, multisig), this is a no-op.
}

fn emit_lifecycle_marker(out: &mut String, mir: &Mir) {
    // lean_gen.rs:1216 — emit `inductive Status` if the lifecycle has
    // ≥ 2 states. Issue #43: a single-state lifecycle is no
    // discriminator; emitting Status for it collides with user-declared
    // `status` fields.
    let states = &mir.state.lifecycle_states;
    if states.len() < 2 {
        return;
    }
    out.push_str("inductive Status where\n");
    for s in states {
        out.push_str(&format!("  | {} : Status\n", safe_name(s)));
    }
    out.push_str("  deriving DecidableEq, Repr\n\n");
}

fn emit_state_struct(out: &mut String, mir: &Mir) {
    // For single-account multi-variant specs, render the canonical
    // flat-state form (Phase 1 pilot scope). Multi-variant ADT
    // emission (the inductive-State form for ≥2 variants) is a
    // Phase 1c task.
    let primary = match mir.state.variants.first() {
        Some(v) => v,
        None => return,
    };

    let has_lifecycle = mir.state.lifecycle_states.len() >= 2;

    out.push_str("structure State where\n");
    for field in &primary.fields {
        out.push_str(&format!(
            "  {} : {}\n",
            safe_name(&field.name),
            render_ty(&field.ty)
        ));
    }
    if has_lifecycle {
        out.push_str("  status : Status\n");
    }
    out.push_str("  deriving DecidableEq, Repr\n\n");
}

// ----------------------------------------------------------------------
// Helpers
// ----------------------------------------------------------------------

/// Render a MIR `Ty` to its Lean form. Mirrors the encoding used by
/// `lean_gen::render_ty_for_field` for compatibility — every numeric
/// type widens to `Nat` since proofs run in Nat, and Pubkey is an
/// opaque Lean abbreviation.
fn render_ty(ty: &crate::mir::Ty) -> String {
    use crate::mir::Ty;
    match ty {
        Ty::U8 | Ty::U16 | Ty::U32 | Ty::U64 | Ty::U128 => "Nat".to_string(),
        Ty::I64 | Ty::I128 => "Int".to_string(),
        Ty::Bool => "Bool".to_string(),
        Ty::Pubkey => "Pubkey".to_string(),
        Ty::Custom(name) => name.clone(),
        Ty::Map { capacity: _, value } => {
            // Phase 1 stub: indexed-state has its own renderer; this
            // codepath shouldn't fire for single-account pilot specs.
            format!("Map /* {} */", render_ty(value))
        }
    }
}

/// Quote Lean reserved names. Today minimal — extend as fixtures
/// surface collisions. `lean_gen.rs::safe_name` uses `«name»` quoting
/// for the same purpose; keep the contract identical.
fn safe_name(name: &str) -> String {
    // Lean reserved words / keywords that collide with common field
    // names. Mirrors `lean_gen.rs::safe_name`. Extended on a
    // surface-as-needed basis.
    const LEAN_RESERVED: &[&str] = &[
        "open",
        "end",
        "let",
        "in",
        "do",
        "match",
        "with",
        "if",
        "then",
        "else",
        "type",
        "def",
        "theorem",
        "lemma",
        "structure",
        "instance",
        "class",
        "namespace",
        "section",
        "private",
        "protected",
        "public",
        "abbrev",
        "axiom",
        "inductive",
        "where",
        "deriving",
        "Nat",
        "Int",
        "Bool",
        "Type",
        "Prop",
    ];
    if LEAN_RESERVED.contains(&name) {
        format!("«{}»", name)
    } else {
        name.to_string()
    }
}

// ----------------------------------------------------------------------
// Tests
// ----------------------------------------------------------------------

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::Path as FsPath;

    fn lower_fixture(rel_path: &str) -> Mir {
        let manifest_dir = std::env::var("CARGO_MANIFEST_DIR").unwrap();
        let workspace_root = FsPath::new(&manifest_dir)
            .ancestors()
            .nth(2)
            .expect("workspace root above crates/qedgen");
        let spec_path = workspace_root.join(rel_path);
        let parsed = crate::check::parse_spec_file(&spec_path)
            .unwrap_or_else(|e| panic!("parse {}: {e}", spec_path.display()));
        crate::mir::lower(&parsed)
    }

    #[test]
    fn render_emits_header_namespace_state() {
        let mir = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir);

        // Header imports present.
        assert!(out.contains("import QEDGen.Solana.Account"));
        assert!(out.contains("import QEDGen.Solana.State"));

        // Namespace matches the spec name.
        assert!(out.contains("namespace Escrow"));
        assert!(out.contains("end Escrow"));

        // open QEDGen.Solana follows the namespace.
        assert!(out.contains("open QEDGen.Solana"));
    }

    #[test]
    fn render_lifecycle_marker_threshold() {
        // escrow has 3 lifecycle states (Uninitialized | Open | Closed)
        // so Status inductive is emitted.
        let mir = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir);
        // Lifecycle is the lifecycle_states vec on StateAdt; for the
        // multi-variant ADT path of escrow, lifecycle_states is
        // populated by `lower_state`. If the count is < 2, no Status
        // marker — verify the boundary.
        let lifecycle_count = mir.state.lifecycle_states.len();
        if lifecycle_count >= 2 {
            assert!(out.contains("inductive Status"));
        } else {
            assert!(!out.contains("inductive Status"));
        }
    }

    #[test]
    fn render_pilot_fixtures_no_panic() {
        for fixture in &[
            "examples/rust/escrow/escrow.qedspec",
            "examples/rust/lending/lending.qedspec",
            "examples/rust/multisig/multisig.qedspec",
            "examples/rust/bundled-stdlib-demo/pool.qedspec",
        ] {
            let mir = lower_fixture(fixture);
            let out = render(&mir);
            assert!(out.contains("namespace "), "{}", fixture);
            assert!(out.contains("end "), "{}", fixture);
        }
    }
}
