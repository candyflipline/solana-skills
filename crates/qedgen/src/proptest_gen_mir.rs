// Phase 5 (v2.30) MIR-based proptest codegen. Lives in parallel to
// `proptest_gen.rs` until snapshot equivalence is validated on every
// pilot fixture; then `proptest_gen.rs` is retired.
//
// Dead-code warnings expected during incremental wiring.
#![allow(dead_code)]

//! qedgen proptest codegen — MIR consumer.
//!
//! Phase 5 of the v2.30 refactor. The existing `proptest_gen.rs`
//! (2,110 LoC) emits `programs/tests/proptest.rs` — Tier-1
//! property-based testing harnesses derived from the spec's state
//! machine (~100ms counterexamples; lighter-weight than Kani BMC).
//!
//! ## Dispatch
//!
//! `QEDGEN_LEGACY_PROPTEST=1` opts back into the legacy
//! `proptest_gen::generate` ParsedSpec-direct path. Default routes
//! through this module (MIR-default post Phase 5 flip).
//!
//! ## Phase 5 scope (this commit)
//!
//! Pure-delegation scaffold + snapshot harness + dispatch flip.
//! `generate(&Mir, &ParsedSpec, &Path, &Path)` delegates the full
//! emit to legacy `proptest_gen::generate` — proptest's per-handler
//! `arb_state` / preservation / invariant / guard / overflow /
//! sequence harnesses are tightly coupled to `ParsedHandler` fields
//! (per-slot binders, abstract binders, effect lowering, `old(...)`
//! resolution) with no clean structural seam to port without
//! lifting requires + effects into typed `Stmt` nodes first — same
//! deferral as Anchor Phase 4g `generate_guards`. The scaffold
//! preserves the dispatch surface so a future cleanup (v3.0) can
//! migrate internal sub-emitters incrementally without touching
//! callers.
//!
//! ## What pure-delegation buys
//!
//! - **Snapshot gate** — `tests/proptest_snapshot.rs` locks the
//!   MIR-default output against a checked-in reference per pilot;
//!   any unintended drift between legacy and MIR routes fails the
//!   gate immediately.
//! - **Escape hatch** — `QEDGEN_LEGACY_PROPTEST=1` keeps the
//!   legacy code path reachable for users who hit unexpected
//!   drift, mirroring the Lean / Kani / Anchor escape hatches.
//! - **Structural parity** — proptest now has the same opt-out
//!   contract as the other three primary codegens; the
//!   refactor's bug-class-elimination promise applies uniformly
//!   when sub-emitters get MIR-direct ports.

use anyhow::Result;
use std::path::Path;

use crate::check::ParsedSpec;
use crate::mir::Mir;

/// Generate the proptest harness file at `output_path`, consuming
/// MIR. Mirrors `proptest_gen::generate(spec_path, output_path)`
/// shape but accepts a pre-lowered `Mir` + `ParsedSpec` + the
/// originating spec path (needed by legacy's drift-stamp logic).
///
/// Phase 5 body: pure delegation to `proptest_gen::generate`.
/// `Mir` carried on the signature so the dispatch site doesn't
/// need to change as future slices migrate sub-emitters.
pub fn generate(
    mir: &Mir,
    parsed: &ParsedSpec,
    spec_path: &Path,
    output_path: &Path,
) -> Result<()> {
    // MIR is unused at this slice — kept on the signature so the
    // dispatch site doesn't need to change as future slices start
    // consuming it.
    let _ = mir;

    if parsed.handlers.is_empty() {
        anyhow::bail!("No operations found in the spec — is this a valid qedspec file?");
    }

    // Delegate the full emit to legacy. The legacy entry takes
    // a spec PATH (re-parses internally); we pass through the
    // originating path so its fingerprint + spec-hash logic
    // computes the same value as a direct legacy invocation.
    crate::proptest_gen::generate(spec_path, output_path)
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
    fn phase_5_scaffold_loads() {
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        assert!(!parsed.handlers.is_empty(), "escrow has handlers");
        assert!(!mir.handlers.is_empty(), "escrow MIR has handlers");
    }
}
