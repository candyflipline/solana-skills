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
//! ## Phase 4a-4b scope
//!
//! Phase 4a (prior commit): scaffold + pure-delegation dispatch.
//! `generate(&Mir, &ParsedSpec, &Path, Target)` calls each promoted-
//! pub(crate) `codegen::generate_<X>` sub-generator in legacy's order.
//!
//! Phase 4b–4c in progress: sub-generators ported one at a time,
//! smallest + most deterministic first.
//!   * 4b/1 — `emit_cargo_toml` (Mir.name + needs_spl predicate
//!     from MIR account kinds / Stmt::TokenTransfer / Stmt::Cpi).
//!   * 4b/2 — `emit_math` (no spec dependency; pure text emit
//!     against the fingerprint hash).
//!   * 4b/3 — `emit_events` (mir.events for structure; falls back
//!     to parsed.events for field-type strings).
//!   * 4c/1 — `emit_errors` (mir.errors.variants + mir.name;
//!     R26/R28 augmentation predicates stay on parsed).
//!   * 4c/2 — `emit_ref_impls` (mir.ref_impls — shape-identical
//!     to parsed.ref_impls; same field-type-string fallback as
//!     events).
//!
//! Remaining sub-generators delegate to their legacy
//! `crate::codegen::generate_<X>` for now.
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
    // Phase 4b/3 — MIR-direct port.
    emit_events(mir, parsed, &fp, output_dir, target)?;
    // Phase 4c/1 — MIR-direct port.
    emit_errors(mir, parsed, &fp, output_dir, target)?;
    crate::codegen::generate_instructions(parsed, &fp, spec_path, output_dir, target)?;
    crate::codegen::generate_guards(parsed, &fp, output_dir, target)?;
    // Phase 4b/2 — MIR-direct port. `generate_math` body is
    // fully deterministic (no spec read at all), so the gate is
    // still the parsed-side `guards_use_math_helpers` predicate
    // until that predicate gets its own MIR port.
    if crate::codegen::guards_use_math_helpers(parsed) {
        emit_math(&fp, output_dir)?;
    }
    // Phase 4c/2 — MIR-direct port.
    emit_ref_impls(mir, parsed, &fp, output_dir, target)?;
    crate::codegen::generate_imported_mirror(parsed, &fp, output_dir, target)?;
    // Phase 4b/1 — MIR-direct port.
    emit_cargo_toml(mir, &fp, output_dir, target)?;

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
// Sub-generators — Phase 4b ports
// ----------------------------------------------------------------------

/// Emit `Cargo.toml` for the generated Anchor/Quasar program crate.
/// MIR-direct port of `codegen::generate_cargo_toml` +
/// `render_qedgen_cargo_toml`. The on-disk merge logic
/// (`merge_cargo_toml`) stays in `codegen.rs` as `pub(crate)` — it's
/// pure text manipulation with no spec dependency.
///
/// Reads from MIR:
///   * `Mir.name` → `[package] name = "<lowercase-with-dashes>"`
///   * `needs_spl` predicate (mirrors legacy `render_qedgen_cargo_toml`
///     line ~4996–5000):
///     - any handler account with `AccountKind::Token` / `Mint`
///     - any `Stmt::TokenTransfer` in any handler body
///     - any `Stmt::Cpi` whose `target` references the `Token`
///       interface
///
/// The Cargo.toml content is byte-identical to legacy output — same
/// fingerprint hash, same `anchor-lang = "0.32.1"` / `anchor-spl`
/// pins, same `qedgen-macros` git tag, same `[workspace]` footer.
fn emit_cargo_toml(
    mir: &Mir,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    let fresh = render_cargo_toml(mir, fp, target);
    let path = output_dir.join("Cargo.toml");
    let final_toml = match std::fs::read_to_string(&path) {
        Ok(existing) if !existing.trim().is_empty() => {
            crate::codegen::merge_cargo_toml(&existing, &fresh)
        }
        _ => fresh,
    };
    std::fs::write(path, final_toml)?;
    Ok(())
}

fn render_cargo_toml(
    mir: &Mir,
    fp: &crate::fingerprint::SpecFingerprint,
    target: Target,
) -> String {
    let program_name = mir.name.to_lowercase().replace('_', "-");
    let needs_spl = mir_needs_spl(mir);
    let hash = fp
        .file_hashes
        .get("Cargo.toml")
        .cloned()
        .unwrap_or_default();
    let qedgen_version = env!("CARGO_PKG_VERSION");

    let mut out = String::new();
    out.push_str(&format!(
        "# ---- GENERATED BY QEDGEN ---- spec-hash:{}\n\n",
        hash
    ));
    out.push_str("[package]\n");
    out.push_str(&format!("name = \"{}\"\n", program_name));
    out.push_str("version = \"0.1.0\"\n");
    out.push_str("edition = \"2021\"\n\n");
    out.push_str("[lib]\n");
    out.push_str("crate-type = [\"cdylib\", \"lib\"]\n\n");
    out.push_str("[features]\n");
    out.push_str("client = []\n");
    out.push_str("debug = []\n\n");
    out.push_str("[dependencies]\n");
    match target {
        Target::Anchor => {
            out.push_str("anchor-lang = \"0.32.1\"\n");
            if needs_spl {
                out.push_str("anchor-spl = \"0.32.1\"\n");
            }
        }
        Target::Quasar => {
            out.push_str("quasar-lang = { version = \"0.0.0\" }\n");
            if needs_spl {
                out.push_str("quasar-spl = { version = \"0.0.0\" }\n");
            }
        }
        Target::Pinocchio => unreachable!("Pinocchio is rejected at the init dispatcher"),
    }
    out.push_str(&format!(
        "qedgen-macros = {{ git = \"https://github.com/qedgen/solana-skills\", tag = \"v{}\" }}\n",
        qedgen_version
    ));

    // Self-contained workspace footer — mirrors legacy line ~5046–5052.
    out.push_str("\n[workspace]\n");

    out
}

/// Emit `src/errors.rs` — `#[error_code] pub enum <Name>Error { ... }`
/// for every declared error variant. MIR-direct port of
/// `codegen::generate_errors`.
///
/// Reads from MIR:
///   * `Mir.name` → enum prefix `<PascalCase(name)>Error`.
///   * `Mir.errors.variants` → base list of variants.
///
/// The `needs_lifecycle` / `needs_invalid_pda` augmentation
/// predicates read from `parsed` for now — they walk
/// `ParsedHandler.accounts.pda_seeds`, `ParsedAccountType.variants[*].fields`,
/// and other compound shapes that don't have direct MIR equivalents
/// yet. Future slice: lift the predicates into MIR proper.
fn emit_errors(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    if mir.errors.variants.is_empty() {
        return Ok(());
    }
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    let prelude_import = match target {
        Target::Anchor => "use anchor_lang::prelude::*;\n",
        Target::Quasar => "use quasar_lang::prelude::*;\n",
        Target::Pinocchio => unreachable!("Pinocchio is rejected at the init dispatcher"),
    };

    let error_name = format!("{}Error", crate::codegen::to_pascal_case(&mir.name));

    let mut out = String::new();
    out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, "src/errors.rs"));
    out.push_str(prelude_import);
    out.push('\n');

    // R26 augmentation: handlers with non-init lifecycle pre-status
    // trigger the auto-added `InvalidLifecycle` variant. Walks
    // `parsed.handlers` for now (the predicate reads pre_status as
    // raw string — MIR carries it as a `VariantTag` via
    // `HandlerMir.transition`; replacing the read with the MIR
    // form is a separately-trackable cleanup).
    let needs_lifecycle = parsed.handlers.iter().any(|h| {
        let pre = h.pre_status.as_deref().unwrap_or("");
        let is_init = matches!(pre, "Uninitialized" | "Empty");
        !pre.is_empty() && !is_init
    });

    // R28 augmentation: runtime PDA verification triggers
    // `InvalidPda`. Same complex walk as legacy — the predicate
    // reads `parsed.handlers[*].accounts.pda_seeds` against
    // `parsed.account_types.variants[*].fields`, which doesn't have
    // a single-call MIR equivalent. Use the pub(crate) helper.
    let needs_invalid_pda = (matches!(target, Target::Quasar)
        || (matches!(target, Target::Anchor)
            && crate::codegen::is_multi_variant_adt_state_pub(parsed)))
        && parsed.handlers.iter().any(|h| {
            let bound: std::collections::HashSet<&str> =
                h.accounts.iter().map(|a| a.name.as_str()).collect();
            let is_init_handler = matches!(
                h.pre_status.as_deref(),
                Some("Uninitialized") | Some("Empty")
            );
            h.accounts.iter().any(|acct| {
                let Some(seeds) = &acct.pda_seeds else {
                    return false;
                };
                if acct.is_signer {
                    return false;
                }
                let on_account_matches = match h.on_account.as_deref() {
                    Some(adt) => {
                        let lower = adt.to_lowercase();
                        acct.name == lower || acct.name.starts_with(&lower)
                    }
                    None => true,
                };
                if is_init_handler && on_account_matches {
                    return false;
                }
                seeds.iter().any(|seed| {
                    let is_literal = seed.starts_with('"') && seed.ends_with('"');
                    if is_literal || bound.contains(seed.as_str()) {
                        return false;
                    }
                    if matches!(target, Target::Anchor) {
                        parsed.account_types.iter().any(|a| {
                            a.variants
                                .iter()
                                .any(|v| v.fields.iter().any(|(n, _)| n == seed))
                        })
                    } else {
                        true
                    }
                })
            })
        });

    let mut codes: Vec<String> = mir.errors.variants.clone();
    if needs_lifecycle && !codes.iter().any(|c| c == "InvalidLifecycle") {
        codes.push("InvalidLifecycle".to_string());
    }
    if needs_invalid_pda && !codes.iter().any(|c| c == "InvalidPda") {
        codes.push("InvalidPda".to_string());
    }

    out.push_str("#[error_code]\n");
    out.push_str(&format!("pub enum {} {{\n", error_name));
    for (i, code) in codes.iter().enumerate() {
        out.push_str(&format!("    {} = {},\n", code, i));
    }
    out.push_str("}\n");
    out.push_str("// ---- END GENERATED ----\n");

    std::fs::write(src_dir.join("errors.rs"), &out)?;
    Ok(())
}

/// Emit `src/ref_impls.rs` — one `pub fn` per declared `ref_impl`.
/// MIR-direct port of `codegen::generate_ref_impls`.
///
/// Iteration source is `mir.ref_impls` (shape-identical to
/// `parsed.ref_impls` since MIR mirrors the fields verbatim). Param +
/// return type strings still flow through `codegen::map_type_for_target`
/// against `parsed` — same `Ty → DSL string` gap as `emit_events`,
/// closes when the shared helper lands.
fn emit_ref_impls(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    if mir.ref_impls.is_empty() {
        return Ok(());
    }
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;
    let mut out = String::new();
    out.push_str(&crate::codegen::marker(
        "DO NOT EDIT",
        fp,
        "src/ref_impls.rs",
    ));
    out.push_str(
        "//! Reference implementations (from qedspec `ref_impl` declarations).\n\
         //! Pure expressions — no state mutation, no side effects.\n\
         //! Generated alongside guards.rs so `requires` / `ensures` clauses\n\
         //! and user handler bodies can call them by name.\n\n",
    );
    out.push_str("#![allow(dead_code, clippy::too_many_arguments)]\n\n");
    for r in &mir.ref_impls {
        let params = r
            .params
            .iter()
            .map(|(n, t)| {
                let ty = crate::codegen::map_type_for_target(t, parsed, target)
                    .unwrap_or_else(|_| t.clone());
                format!("{}: {}", n, ty)
            })
            .collect::<Vec<_>>()
            .join(", ");
        let ret = crate::codegen::map_type_for_target(&r.return_type, parsed, target)
            .unwrap_or_else(|_| r.return_type.clone());
        if let Some(doc) = &r.doc {
            for line in doc.lines() {
                out.push_str(&format!("/// {}\n", line.trim_start_matches("///").trim()));
            }
        }
        out.push_str(&format!(
            "#[inline]\npub fn {}({}) -> {} {{\n    {}\n}}\n\n",
            r.name, params, ret, r.rust_body
        ));
    }
    out.push_str("// ---- END GENERATED ----\n");
    std::fs::write(src_dir.join("ref_impls.rs"), &out)?;
    Ok(())
}

/// Emit `src/events.rs` — one `#[event] pub struct <EventName> {
/// ... }` per declared event. MIR-direct port of
/// `codegen::generate_events`.
///
/// Iteration source is `mir.events` (typed event structure); the
/// field-type strings come from a parallel lookup against
/// `parsed.events` because `map_type_for_target` consumes the raw
/// DSL string (not the MIR `Ty` enum). Once `Ty → DSL-string`
/// conversion lands as a shared helper, this can read fields
/// entirely from MIR.
fn emit_events(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    if mir.events.is_empty() {
        return Ok(());
    }
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    // Per-target framework surface. Hardcoded here for events only
    // (the full `FrameworkSurface` struct stays module-private in
    // `codegen.rs`; Phase 4b ports use the minimal slice they need).
    let (prelude_import, explicit_discriminator): (&str, bool) = match target {
        Target::Anchor => ("use anchor_lang::prelude::*;\n", false),
        Target::Quasar => ("use quasar_lang::prelude::*;\n", true),
        Target::Pinocchio => unreachable!("Pinocchio is rejected at the init dispatcher"),
    };

    let mut out = String::new();
    out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, "src/events.rs"));
    out.push_str(prelude_import);
    out.push('\n');

    for (i, event) in mir.events.iter().enumerate() {
        // Look up the corresponding ParsedEvent for the raw DSL
        // field-type strings (`map_type_for_target` consumes
        // String, not MIR `Ty`).
        let parsed_event = parsed
            .events
            .iter()
            .find(|e| e.name == event.name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MIR event '{}' has no matching ParsedEvent (parser/lowering mismatch)",
                    event.name
                )
            })?;

        if explicit_discriminator {
            out.push_str(&format!("#[event(discriminator = {})]\n", i + 1));
        } else {
            out.push_str("#[event]\n");
        }
        out.push_str(&format!("pub struct {} {{\n", event.name));
        for (fname, ftype) in &parsed_event.fields {
            out.push_str(&format!(
                "    pub {}: {},\n",
                fname,
                crate::codegen::map_type_for_target(ftype, parsed, target)?
            ));
        }
        out.push_str("}\n\n");
    }

    out.push_str("// ---- END GENERATED ----\n");

    std::fs::write(src_dir.join("events.rs"), &out)?;
    Ok(())
}

/// Emit `src/math.rs` — fixed-point math helpers used by spec-
/// derived guards / properties. MIR-direct port of
/// `codegen::generate_math`; body is fully deterministic (no spec
/// dependency), so the port reproduces the legacy text verbatim.
/// The only data input is the `tests/math.rs` fingerprint hash via
/// the marker banner.
fn emit_math(fp: &crate::fingerprint::SpecFingerprint, output_dir: &Path) -> Result<()> {
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;
    let mut out = String::new();
    out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, "src/math.rs"));
    out.push_str("//! Fixed-point math helpers used by spec-derived guards and properties.\n\n");
    out.push_str("#![allow(dead_code)]\n\n");
    out.push_str(
        "/// Floor of `(a * b) / d`. Returns `0` if `d == 0` (caller must guard).\n\
/// Uses saturating multiplication as a safe approximation; specs that need\n\
/// exact u256-width fixed-point math should pin a checked widening crate\n\
/// once the spec language exposes one.\n\
#[inline]\n\
pub fn mul_div_floor_u128(a: u128, b: u128, d: u128) -> u128 {\n\
    if d == 0 {\n\
        return 0;\n\
    }\n\
    a.saturating_mul(b) / d\n\
}\n\n",
    );
    out.push_str(
        "/// Ceiling of `(a * b) / d`. Same caveats as `mul_div_floor_u128`.\n\
#[inline]\n\
pub fn mul_div_ceil_u128(a: u128, b: u128, d: u128) -> u128 {\n\
    if d == 0 {\n\
        return 0;\n\
    }\n\
    let prod = a.saturating_mul(b);\n\
    if prod % d == 0 {\n\
        prod / d\n\
    } else {\n\
        (prod / d).saturating_add(1)\n\
    }\n\
}\n",
    );
    out.push_str("// ---- END GENERATED ----\n");
    std::fs::write(src_dir.join("math.rs"), &out)?;
    Ok(())
}

/// MIR predicate for `needs_spl` — true when the program crate's
/// Cargo.toml needs `anchor-spl` / `quasar-spl` pulled in. Mirrors
/// the legacy heuristic in `render_qedgen_cargo_toml` (line ~4996).
fn mir_needs_spl(mir: &Mir) -> bool {
    use crate::mir::{AccountKind, Stmt};

    for handler in &mir.handlers {
        // Any account declared as Token / Mint type.
        if handler
            .accounts
            .iter()
            .any(|a| matches!(a.kind, AccountKind::Token | AccountKind::Mint))
        {
            return true;
        }
        // Any TokenTransfer stmt (handles both the `transfers { … }`
        // sugar and `call Token.transfer(...)` — both lower to
        // `Stmt::TokenTransfer` per the MIR lowering contract).
        // Any explicit Cpi targeting the `Token` interface.
        for stmt in &handler.body.stmts {
            match stmt {
                Stmt::TokenTransfer { .. } => return true,
                Stmt::Cpi { target, .. } if target.0 == "Token" => return true,
                _ => {}
            }
        }
    }
    false
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
