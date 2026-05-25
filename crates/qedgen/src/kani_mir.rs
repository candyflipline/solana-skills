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
//! ## Phase 3a scope (this commit)
//!
//! Scaffold + the deterministic structural sections (file header,
//! math helpers, state-model header, constants). All harness-emit
//! sections — records, enums, State struct, transitions, guard /
//! effect / overflow / abort / property / invariant harnesses, file-
//! level features (covers, liveness, environment) — emit a structured
//! `MIR-TODO(phase-3<N>)` comment marking where the next slice picks
//! up. This commit is intentionally a no-op for users who don't set
//! `QEDGEN_USE_MIR_KANI`; opt-in users get the partial scaffold so
//! the structural sections can be eyeballed against `kani.rs`.
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

/// Pure render. Phase 3a emits the deterministic structural prefix
/// (banner / math helpers / state-model header / constants) and a
/// `MIR-TODO` marker where future slices pick up.
pub fn render(mir: &Mir, parsed: &ParsedSpec) -> String {
    let mut out = String::new();
    emit_header(&mut out, parsed);
    emit_math_helpers(&mut out, parsed);
    emit_state_model_header(&mut out);
    emit_constants(&mut out, mir);

    // Phase 3b+ stub. Every remaining section emits here in
    // subsequent slices, per the kani.rs section walk in this
    // module's header doc.
    out.push_str(
        "// MIR-TODO(phase-3b): records / enums / Status / State / transitions / \
         guard / effect / overflow / abort / property / invariant / file-level \
         features (covers / liveness / environment) not ported yet — fall back to \
         legacy `kani.rs` for production output.\n",
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
    if crate::codegen::guards_use_math_helpers(parsed) {
        out.push_str(
            "#[allow(dead_code)]\n\
#[inline]\n\
fn mul_div_floor_u128(a: u128, b: u128, d: u128) -> u128 {\n    \
    if d == 0 { return 0; }\n    \
    a.saturating_mul(b) / d\n\
}\n\n\
#[allow(dead_code)]\n\
#[inline]\n\
fn mul_div_ceil_u128(a: u128, b: u128, d: u128) -> u128 {\n    \
    if d == 0 { return 0; }\n    \
    let prod = a.saturating_mul(b);\n    \
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
    fn render_emits_phase_3b_todo_marker() {
        // Until subsequent slices port the remaining sections, every
        // rendered file ends with the structured TODO marker so
        // users know what's missing.
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir, &parsed);
        assert!(
            out.contains("MIR-TODO(phase-3b)"),
            "expected phase-3b TODO marker"
        );
    }
}
