// Phase 4 (v2.30) MIR-based Anchor/Quasar codegen. Lives in parallel
// to `codegen.rs` until snapshot equivalence is validated on every
// pilot fixture; then `codegen.rs` is retired.
//
// Dead-code warnings expected during incremental wiring.
#![allow(dead_code)]

//! qedgen Anchor/Quasar codegen — MIR consumer.
//!
//! Phase 4 of the v2.30 refactor. The existing `codegen.rs` (7,572
//! LoC, 67 fns) is the user-facing program emitter — it ships the
//! `lib.rs`, `state.rs`, `errors.rs`, `events.rs`,
//! `instructions/<handler>.rs`, `guards.rs`, `math.rs`, `Cargo.toml`,
//! etc. that every `qedgen codegen --target {anchor,quasar}` writes
//! into `programs/`. Highest blast radius of any qedgen codegen
//! because the output is the actual Solana program users compile
//! and deploy.
//!
//! ## Dispatch
//!
//! `QEDGEN_USE_MIR_CODEGEN=1` switches the `qedgen codegen --target
//! …` call site to this module; without the flag, legacy
//! `codegen::generate` runs. Default stays on legacy until
//! snapshot equivalence ratifies every pilot fixture (mirrors Lean
//! / Kani Phase 1 / Phase 3 sequencing).
//!
//! ## Phase 4a scope (this commit)
//!
//! Scaffold + pure-delegation dispatch. `generate(&Mir, &ParsedSpec,
//! &Path, Target)` calls each promoted-pub(crate) `codegen::generate_<X>`
//! sub-generator in legacy's order. Byte-equivalence is trivial
//! because the bodies ARE the legacy bodies — the MIR side is just a
//! per-sub-generator orchestration shell at this slice. Subsequent
//! Phase 4 slices port individual sub-generators to consume `Mir`
//! fragments directly.
//!
//! Why pure-delegation first instead of porting any sub-generator
//! immediately: codegen.rs's blast radius (`programs/lib.rs` and
//! friends are the user's deployed Solana program) means the
//! refactor must land in slices that each individually pass the
//! snapshot gate. Phase 4a stands up the structural shell so the
//! dispatch is in place; Phase 4b+ migrates sub-generators one at a
//! time, each commit independently verifiable.
//!
//! ## Sub-generator porting order (planned)
//!
//! Smallest + most deterministic first; per-handler instructions
//! last (highest cross-helper coupling).
//!
//! | Phase | Sub-generator           | Legacy LoC | Notes                          |
//! |-------|-------------------------|-----------|--------------------------------|
//! | 4b    | `generate_cargo_toml`   |  16       | Trivial — toml template        |
//! | 4b    | `generate_math`         |  49       | Fixed inline math fns          |
//! | 4b    | `generate_events`       |  44       | `#[event]` structs from spec.events |
//! | 4c    | `generate_errors`       | 113       | `#[error_code]` enum from spec.error_codes |
//! | 4c    | `generate_ref_impls`    |  50       | `ref_impl` fns                 |
//! | 4d    | `generate_imported_mirror` | 232    | `src/imported/<ns>.rs` mirrors |
//! | 4e    | `generate_lib`          | 238       | `#[program] pub mod` entry     |
//! | 4f    | `generate_state`        | 328       | `#[account]` data struct       |
//! | 4g    | `generate_guards`       | 636       | Largest — guard fns per handler |
//! | 4h    | `generate_instructions` |  77 entry | Per-handler files (helpers elsewhere) |
//! | 4i    | snapshot tests + flip   |   —       | mirrors Lean 1d / Kani 3f      |

use anyhow::Result;
use std::path::Path;

use crate::check::ParsedSpec;
use crate::mir::Mir;
use crate::Target;

/// Generate the Anchor/Quasar program code under `output_dir`,
/// consuming MIR. Mirrors `codegen::generate(spec_path, output_dir,
/// target)` shape but accepts a pre-lowered `Mir` + the originating
/// `ParsedSpec` + the spec path (needed by `generate_instructions`'s
/// drift-stamping logic).
///
/// Phase 4a body: pure delegation to legacy `codegen::generate_<X>`
/// sub-generators in the same order as `codegen::generate`. Future
/// slices replace each delegated call with a MIR-direct port.
pub fn generate(
    mir: &Mir,
    parsed: &ParsedSpec,
    spec_path: &Path,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    // Mir is unused at this slice — kept on the signature so the
    // dispatch site doesn't need to change as Phase 4b+ sub-generators
    // start consuming it.
    let _ = mir;

    if parsed.handlers.is_empty() {
        anyhow::bail!("No handlers found in the spec — is this a valid qedspec file?");
    }

    crate::rust_codegen_util::check_effect_targets(parsed)?;

    // Project initialization check (mirrors `codegen::generate` line
    // 5250).
    if crate::init::find_qed_dir(spec_path).is_none() {
        anyhow::bail!(
            "No .qed/ directory found next to {} — run `qedgen init` first.",
            spec_path.display()
        );
    }

    std::fs::create_dir_all(output_dir)?;

    let fp = crate::fingerprint::compute_fingerprint(parsed);

    // Sub-generator dispatch — same order as `codegen::generate`
    // (lines 5261–5280). Each delegated call gets its MIR-direct
    // port in a later Phase 4 slice.
    crate::codegen::generate_lib(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_state(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_events(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_errors(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_instructions(parsed, &fp, spec_path, output_dir, target)?;
    crate::codegen::generate_guards(parsed, &fp, output_dir, target)?;
    if crate::codegen::guards_use_math_helpers(parsed) {
        crate::codegen::generate_math(&fp, output_dir)?;
    }
    crate::codegen::generate_ref_impls(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_imported_mirror(parsed, &fp, output_dir, target)?;
    crate::codegen::generate_cargo_toml(parsed, &fp, output_dir, target)?;

    let file_count = 4
        + parsed.handlers.len()
        + usize::from(!parsed.events.is_empty())
        + usize::from(!parsed.error_codes.is_empty());

    eprintln!(
        "[MIR-pilot] Generated {} files in {}",
        file_count,
        output_dir.display()
    );

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
    fn phase_4a_scaffold_loads() {
        // Phase 4a doesn't render to disk — `generate` requires a
        // .qed/ dir + output_dir. The smoke test here is that the
        // module compiles and that `lower_fixture` round-trips a
        // real spec into MIR + parsed without panicking. The
        // dispatch-level integration test lives in the codegen
        // sweep (Phase 4i snapshot harness, future slice).
        let (mir, parsed) = lower_fixture("examples/rust/escrow/escrow.qedspec");
        assert!(!parsed.handlers.is_empty(), "escrow has handlers");
        assert!(!mir.state.variants.is_empty(), "escrow has state variants");
    }
}
