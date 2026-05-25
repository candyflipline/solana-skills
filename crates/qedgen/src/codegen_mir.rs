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
    // Phase 4e — MIR-direct port.
    emit_lib(mir, parsed, &fp, output_dir, target)?;
    // Phase 4f — MIR-direct port.
    emit_state(mir, parsed, &fp, output_dir, target)?;
    // Phase 4b/3 — MIR-direct port.
    emit_events(mir, parsed, &fp, output_dir, target)?;
    // Phase 4c/1 — MIR-direct port.
    emit_errors(mir, parsed, &fp, output_dir, target)?;
    // Phase 4h — MIR-direct port.
    emit_instructions(mir, parsed, &fp, spec_path, output_dir, target)?;
    // Phase 4g — deferred. `generate_guards` is 636L of per-handler
    // `requires` / `effects` / `auth` / `status` emission deeply
    // coupled to `ParsedHandler` fields with no clean structural
    // seam. A meaningful MIR port requires lifting requires +
    // effects into typed `Stmt` nodes first — that's a separate
    // v3.0-class refactor. Until then, delegate to legacy.
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
    // Phase 4d — MIR-direct port.
    emit_imported_mirror(mir, parsed, &fp, output_dir, target)?;
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

/// Emit `src/lib.rs` — the `#[program] pub mod` entry with one
/// `pub fn` per handler dispatching to `ctx.accounts.handler(...)`.
/// Idempotent: if `src/lib.rs` already exists, the call is a no-op
/// (matches legacy line 714–722) so user-stamped imports / extra
/// modules survive regeneration. MIR-direct port of
/// `codegen::generate_lib`.
///
/// Reads from MIR:
///   * `Mir.name` → program mod name (lowercased) + crate root.
///   * `mir.handlers[*].name` / `.doc` → per-handler fn emission.
///   * `mir.events.is_empty()` / `mir.errors.variants.is_empty()` /
///     `mir.ref_impls.is_empty()` → mod re-export gating.
///   * `mir.imports.values().any(...)` → `pub mod imported;` gate.
///
/// Falls back to `parsed` for:
///   * `program_id` (top-level `Option<String>` not in MIR).
///   * `type_aliases` (for Fin alias resolution on Quasar param
///     types).
///   * `ParsedHandler.has_bumps()` / `.takes_params` / `.accounts`
///     (the per-handler dispatch + Anchor token/mint detection
///     walk ParsedHandler fields directly).
///   * `render_handler_accounts_struct` (600+ LoC Anchor helper
///     that owns the `#[derive(Accounts)]` struct emission;
///     promoted to `pub(crate)`, still consumes `ParsedSpec` /
///     `ParsedHandler` directly).
fn emit_lib(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    use crate::codegen::{to_pascal_case, FrameworkSurface};

    let surface = FrameworkSurface::for_target(target);
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    let lib_path = src_dir.join("lib.rs");
    if lib_path.exists() {
        eprintln!(
            "programs/{}/src/lib.rs already exists — skipping (user-owned). guards.rs regenerated.",
            output_dir
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("<program>")
        );
        return Ok(());
    }

    let program_name = mir.name.to_lowercase();
    let program_id = parsed
        .program_id
        .as_deref()
        .unwrap_or("11111111111111111111111111111111");

    let mut out = String::new();
    out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, "src/lib.rs"));
    out.push_str(surface.crate_attrs);
    out.push_str(surface.prelude_import);
    out.push('\n');
    out.push_str("mod instructions;\n");
    if matches!(target, Target::Quasar) {
        out.push_str("use instructions::*;\n");
    }

    if !mir.events.is_empty() {
        out.push_str("pub mod events;\n");
    }
    if !mir.errors.variants.is_empty() {
        out.push_str("pub mod errors;\n");
    }
    out.push_str("pub mod state;\n");
    out.push_str("pub mod guards;\n");
    if crate::codegen::guards_use_math_helpers(parsed) {
        out.push_str("pub mod math;\n");
    }
    if !mir.ref_impls.is_empty() {
        out.push_str("pub mod ref_impls;\n");
    }
    if mir
        .imports
        .values()
        .any(|imp| !imp.account_types.is_empty())
    {
        out.push_str("pub mod imported;\n");
    }
    out.push('\n');

    out.push_str(&format!("declare_id!(\"{}\");\n\n", program_id));

    out.push_str("#[program]\n");
    out.push_str(&format!(
        "{} {} {{\n",
        surface.program_mod_vis, program_name
    ));
    out.push_str("    use super::*;\n\n");

    // Per-handler fn emission. Iterates `mir.handlers` (typed
    // structure) but reads `parsed.handlers` by index for the
    // `has_bumps()` / `takes_params` / `type_aliases` Fin-resolution
    // details that consume ParsedHandler directly.
    for (i, handler) in mir.handlers.iter().enumerate() {
        let parsed_handler = parsed
            .handlers
            .iter()
            .find(|h| h.name == handler.name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MIR handler '{}' has no matching ParsedHandler (parser/lowering mismatch)",
                    handler.name
                )
            })?;
        let pascal = to_pascal_case(&handler.name);

        if let Some(ref doc) = handler.doc {
            out.push_str(&format!("    /// {}\n", doc));
        }
        if surface.explicit_handler_discriminator {
            out.push_str(&format!("    #[instruction(discriminator = {})]\n", i));
        }

        let mut params = format!("ctx: {}<{}>", surface.context_type, pascal);

        let needs_fin_cast = |ptype: &str| -> bool {
            if !matches!(target, Target::Quasar) {
                return false;
            }
            let mut resolved = ptype.trim().to_string();
            while let Some((_, rhs)) = parsed.type_aliases.iter().find(|(n, _)| n == &resolved) {
                resolved = rhs.trim().to_string();
            }
            resolved.starts_with("Fin")
        };

        for (pname, ptype) in &parsed_handler.takes_params {
            let rust_ty = if needs_fin_cast(ptype) {
                "u32".to_string()
            } else {
                crate::codegen::map_type_for_target(ptype, parsed, target)?
            };
            params.push_str(&format!(", {}: {}", pname, rust_ty));
        }

        out.push_str(&format!(
            "    pub fn {}({}) -> {} {{\n",
            handler.name, params, surface.handler_result_type
        ));

        let cast_arg = |pname: &str, ptype: &str| -> String {
            if needs_fin_cast(ptype) {
                format!("{} as usize", pname)
            } else {
                pname.to_string()
            }
        };

        if parsed_handler.has_bumps() {
            out.push_str(&format!(
                "        ctx.accounts.handler({}&ctx.bumps)\n",
                parsed_handler
                    .takes_params
                    .iter()
                    .map(|(n, t)| format!("{}, ", cast_arg(n, t)))
                    .collect::<String>()
            ));
        } else {
            out.push_str(&format!(
                "        ctx.accounts.handler({})\n",
                parsed_handler
                    .takes_params
                    .iter()
                    .map(|(n, t)| cast_arg(n, t))
                    .collect::<Vec<_>>()
                    .join(", ")
            ));
        }
        out.push_str("    }\n\n");
    }

    out.push_str("}\n");

    // Anchor: emit `#[derive(Accounts)]` structs at crate root via
    // legacy `render_handler_accounts_struct` (600+ LoC helper that
    // consumes ParsedSpec / ParsedHandler extensively — porting it
    // is a separately-trackable cleanup).
    if matches!(target, Target::Anchor) {
        let is_multi = parsed.account_types.len() > 1;
        let default_state_name = format!("{}Account", to_pascal_case(&mir.name));
        out.push('\n');
        out.push_str("// `#[derive(Accounts)]` structs live at the crate root so the\n");
        out.push_str("// Anchor `#[program]` macro can resolve them via `crate::*`.\n");
        out.push_str("// The handler impl blocks live next to the (always-regenerated)\n");
        out.push_str("// guard module in `instructions/<name>.rs`.\n");
        out.push_str("use crate::state::*;\n");
        let has_token = parsed.handlers.iter().any(|h| {
            h.accounts
                .iter()
                .any(|a| a.account_type.as_deref() == Some("token") || a.name == "token_program")
        });
        let has_mint = parsed.handlers.iter().any(|h| {
            h.accounts
                .iter()
                .any(|a| a.account_type.as_deref() == Some("mint"))
        });
        let imports = surface.token_imports(has_token, has_mint);
        if !imports.is_empty() {
            out.push_str(&imports);
        }
        for handler in &parsed.handlers {
            out.push('\n');
            out.push_str(&crate::codegen::render_handler_accounts_struct(
                handler,
                parsed,
                is_multi,
                &default_state_name,
                &surface,
                target,
            ));
        }
    }

    out.push_str("// ---- END GENERATED ----\n");

    std::fs::write(src_dir.join("lib.rs"), &out)?;
    Ok(())
}

/// Emit `src/instructions/mod.rs` + per-handler
/// `src/instructions/<name>.rs` scaffold files. MIR-direct port of
/// `codegen::generate_instructions` (77L entry — the per-handler
/// emit body lives in `render_handler_scaffold`, a 600+ LoC helper
/// promoted to `pub(crate)` and called unchanged).
///
/// Per-handler `<name>.rs` files are USER-OWNED — emitted only when
/// missing. The `mod.rs` re-exporter is always regenerated.
///
/// Reads from MIR for iteration (`mir.handlers[*].name`); falls back
/// to `parsed` for the per-handler scaffold body
/// (`ParsedHandler` carries the accounts / effects / auth /
/// transition / takes_params data the scaffold renders).
fn emit_instructions(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    spec_path: &Path,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    use crate::codegen::to_pascal_case;

    let instr_dir = output_dir.join("src").join("instructions");
    std::fs::create_dir_all(&instr_dir)?;

    let is_multi = parsed.account_types.len() > 1;
    let default_state_name = format!("{}Account", to_pascal_case(&mir.name));

    // mod.rs — always regenerated, pure scaffold.
    let mut mod_out = String::new();
    mod_out.push_str(&crate::codegen::marker(
        "DO NOT EDIT",
        fp,
        "src/instructions/mod.rs",
    ));
    for handler in &mir.handlers {
        mod_out.push_str(&format!("pub mod {};\n", handler.name));
    }
    // Quasar re-exports `Accounts` structs from each
    // `instructions/<name>.rs`; Anchor keeps them in lib.rs at
    // crate root.
    if matches!(target, Target::Quasar) {
        mod_out.push('\n');
        for handler in &mir.handlers {
            let pascal = to_pascal_case(&handler.name);
            mod_out.push_str(&format!("pub use {}::{};\n", handler.name, pascal));
        }
    }
    mod_out.push_str("// ---- END GENERATED ----\n");
    std::fs::write(instr_dir.join("mod.rs"), &mod_out)?;

    // Read spec source once for spec_hash attributes (handles both
    // single-file and multi-file specs).
    let spec_src = crate::check::read_spec_source(spec_path).unwrap_or_default();
    let spec_attr = crate::codegen::relative_spec_path(spec_path, output_dir);

    // Per-handler scaffold files (user-owned — skipped if existing).
    // Iteration source is `mir.handlers` for the name; the matching
    // `ParsedHandler` is what `render_handler_scaffold` actually
    // consumes (per-handler accounts / effects / requires walk).
    for handler_mir in &mir.handlers {
        let handler = parsed
            .handlers
            .iter()
            .find(|h| h.name == handler_mir.name)
            .ok_or_else(|| {
                anyhow::anyhow!(
                    "MIR handler '{}' has no matching ParsedHandler",
                    handler_mir.name
                )
            })?;

        let handler_path = instr_dir.join(format!("{}.rs", handler.name));
        if handler_path.exists() {
            eprintln!(
                "programs/{}/src/instructions/{}.rs already exists — skipping (user-owned). guards.rs regenerated.",
                output_dir
                    .file_name()
                    .and_then(|n| n.to_str())
                    .unwrap_or("<program>"),
                handler.name
            );
            continue;
        }

        let out = crate::codegen::render_handler_scaffold(
            handler,
            parsed,
            is_multi,
            &default_state_name,
            &spec_src,
            &spec_attr,
            target,
        )?;
        std::fs::write(&handler_path, &out)?;
    }

    Ok(())
}

/// Emit `src/state.rs` — `#[account]` data structures for the
/// program's persisted state. MIR-direct port of
/// `codegen::generate_state` (328L).
///
/// Dispatches three shapes:
///   1. **Multi-account** (`mir.account_states.len() > 1`): one
///      `<Name>Account` `#[account]` struct per account_type, with
///      optional `<Name>Status` enum.
///   2. **Multi-variant ADT (Anchor only, WrongState-gated)**:
///      wrapper-struct + inner-enum pair, with Slice B accessor
///      methods for fields shared across variants.
///   3. **Flat single-account**: `<Name>Account` struct from
///      `spec.state_fields` with optional bump / status fields +
///      lifecycle `Status` enum.
///
/// Reads from MIR for structure (`mir.name`, `mir.account_states`),
/// from `parsed` for compound shapes not yet lifted into MIR
/// (`records`, `state_fields`, `lifecycle_states`, `pdas`, per-
/// account `pda_ref`).
fn emit_state(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    use crate::codegen::{
        is_multi_variant_adt_state_pub, map_type_for_target, map_type_pod, to_pascal_case,
        FrameworkSurface,
    };

    let surface = FrameworkSurface::for_target(target);
    let src_dir = output_dir.join("src");
    std::fs::create_dir_all(&src_dir)?;

    let is_multi = mir.account_states.len() > 1;

    let mut out = String::new();
    out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, "src/state.rs"));
    out.push_str(surface.prelude_import);
    out.push('\n');

    // Records first — emitted as `#[repr(C)]` structs with target-
    // specific derives. Anchor needs Borsh + InitSpace for the
    // `#[account]` outer struct's space calculation; Quasar needs
    // Pod-companion type mapping for zero-copy alignment.
    for record in &parsed.records {
        out.push_str("#[repr(C)]\n");
        let derives = match target {
            Target::Anchor => "#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq)]\n",
            _ => "#[derive(Clone, Copy)]\n",
        };
        out.push_str(derives);
        out.push_str(&format!("pub struct {} {{\n", record.name));
        for (fname, ftype) in &record.fields {
            let rust_ty = match target {
                Target::Quasar => map_type_pod(ftype, parsed)?,
                _ => map_type_for_target(ftype, parsed, target)?,
            };
            out.push_str(&format!("    pub {}: {},\n", fname, rust_ty));
        }
        out.push_str("}\n\n");
    }

    if is_multi {
        // Multi-account: iterate `mir.account_states` for structure;
        // pda_ref is on ParsedAccountType so look up the matching
        // parsed entry by name.
        for (idx, acct_mir) in mir.account_states.iter().enumerate() {
            let acct = parsed
                .account_types
                .iter()
                .find(|a| a.name == acct_mir.name)
                .ok_or_else(|| {
                    anyhow::anyhow!(
                        "MIR account_state '{}' has no matching ParsedAccountType",
                        acct_mir.name
                    )
                })?;
            let struct_name = format!("{}Account", acct.name);

            let account_attr = if surface.explicit_account_discriminator {
                format!("#[account(discriminator = {})]\n", idx + 1)
            } else {
                "#[account]\n".to_string()
            };
            out.push_str(&account_attr);
            if matches!(target, Target::Anchor) {
                out.push_str("#[derive(InitSpace)]\n");
            }
            out.push_str(&format!("pub struct {} {{\n", struct_name));

            for (fname, ftype) in &acct.fields {
                out.push_str(&format!(
                    "    pub {}: {},\n",
                    fname,
                    map_type_for_target(ftype, parsed, target)?
                ));
            }

            if acct.pda_ref.is_some() && !acct.fields.iter().any(|(n, _)| n == "bump") {
                out.push_str("    pub bump: u8,\n");
            }

            if !acct.lifecycle.is_empty() && !acct.fields.iter().any(|(n, _)| n == "status") {
                out.push_str("    pub status: u8,\n");
            }

            out.push_str("}\n\n");

            if !acct.lifecycle.is_empty() {
                out.push_str(&format!("/// {} lifecycle states.\n", acct.name));
                out.push_str("#[derive(Clone, Copy, PartialEq, Eq)]\n");
                out.push_str("#[repr(u8)]\n");
                out.push_str(&format!("pub enum {}Status {{\n", acct.name));
                for (i, state) in acct.lifecycle.iter().enumerate() {
                    out.push_str(&format!("    {} = {},\n", state, i));
                }
                out.push_str("}\n\n");
            }
        }
    } else if is_multi_variant_adt_state_pub(parsed) && matches!(target, Target::Anchor) {
        // Multi-variant ADT (Anchor only, WrongState-gated):
        // wrapper struct + inner enum + Slice B accessors.
        let state_name = format!("{}Account", to_pascal_case(&mir.name));
        let inner_name = format!("{}Inner", state_name);
        let acct = &parsed.account_types[0];

        out.push_str("#[account]\n");
        out.push_str("#[derive(InitSpace)]\n");
        out.push_str(&format!("pub struct {} {{\n", state_name));
        out.push_str(&format!("    pub inner: {},\n", inner_name));
        if !parsed.pdas.is_empty() && !parsed.state_fields.iter().any(|(n, _)| n == "bump") {
            out.push_str("    pub bump: u8,\n");
        }
        out.push_str("}\n\n");

        out.push_str(&format!(
            "/// Variant-payload state for {0}. The Anchor wrapper above\n\
             /// carries the account discriminator; this enum carries the\n\
             /// state-machine variant + per-variant payload fields.\n",
            state_name
        ));
        out.push_str(
            "#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Debug, PartialEq)]\n",
        );
        out.push_str(&format!("pub enum {} {{\n", inner_name));
        for variant in &acct.variants {
            if variant.fields.is_empty() {
                out.push_str(&format!("    {},\n", variant.name));
            } else {
                out.push_str(&format!("    {} {{\n", variant.name));
                for (fname, ftype) in &variant.fields {
                    out.push_str(&format!(
                        "        {}: {},\n",
                        fname,
                        map_type_for_target(ftype, parsed, target)?
                    ));
                }
                out.push_str("    },\n");
            }
        }
        out.push_str("}\n\n");

        // Slice B accessors for fields shared across variants.
        let mut field_index: std::collections::BTreeMap<String, Vec<(String, String)>> =
            std::collections::BTreeMap::new();
        for variant in &acct.variants {
            for (fname, ftype) in &variant.fields {
                field_index
                    .entry(fname.clone())
                    .or_default()
                    .push((variant.name.clone(), ftype.clone()));
            }
        }
        if !field_index.is_empty() {
            out.push_str(&format!("impl {} {{\n", inner_name));
            for (fname, occurrences) in &field_index {
                let first_ty = &occurrences[0].1;
                if occurrences.iter().any(|(_, t)| t != first_ty) {
                    continue;
                }
                let rust_ty = map_type_for_target(first_ty, parsed, target)?;
                out.push_str(&format!(
                    "    /// v2.29 Slice B accessor for `{0}`. Panics on variants\n\
                     /// that don't carry the field — guarded against by the\n\
                     /// per-handler lifecycle check that fires before any\n\
                     /// `requires` emission in `crate::guards`.\n",
                    fname
                ));
                out.push_str(&format!(
                    "    pub fn {}(&self) -> &{} {{\n        match self {{\n",
                    fname, rust_ty
                ));
                for (variant_name, _) in occurrences {
                    out.push_str(&format!(
                        "            Self::{} {{ {}, .. }} => {},\n",
                        variant_name, fname, fname
                    ));
                }
                let all_variants = acct.variants.len();
                if occurrences.len() < all_variants {
                    out.push_str(&format!(
                        "            _ => panic!(\"{}::{}() called on a variant without `{}`\"),\n",
                        inner_name, fname, fname
                    ));
                }
                out.push_str("        }\n    }\n");
            }
            out.push_str("}\n");
        }
    } else {
        // Flat single-account fallback.
        let state_name = format!("{}Account", to_pascal_case(&mir.name));

        let account_attr = if surface.explicit_account_discriminator {
            "#[account(discriminator = 1)]\n"
        } else {
            "#[account]\n"
        };
        out.push_str(&format!("{}pub struct {} {{\n", account_attr, state_name));

        for (fname, ftype) in &parsed.state_fields {
            out.push_str(&format!(
                "    pub {}: {},\n",
                fname,
                map_type_for_target(ftype, parsed, target)?
            ));
        }

        if !parsed.pdas.is_empty() && !parsed.state_fields.iter().any(|(n, _)| n == "bump") {
            out.push_str("    pub bump: u8,\n");
        }

        if !parsed.lifecycle_states.is_empty()
            && !parsed.state_fields.iter().any(|(n, _)| n == "status")
        {
            out.push_str("    pub status: u8,\n");
        }

        out.push_str("}\n");

        if !parsed.lifecycle_states.is_empty() {
            out.push_str("\n/// Program lifecycle states.\n");
            out.push_str("#[derive(Clone, Copy, PartialEq, Eq)]\n");
            out.push_str("#[repr(u8)]\n");
            out.push_str("pub enum Status {\n");
            for (i, state) in parsed.lifecycle_states.iter().enumerate() {
                out.push_str(&format!("    {} = {},\n", state, i));
            }
            out.push_str("}\n");
        }
    }

    out.push_str("// ---- END GENERATED ----\n");

    std::fs::write(src_dir.join("state.rs"), &out)?;
    Ok(())
}

/// Emit `src/imported/<ns>.rs` mirror files + `src/imported/mod.rs`
/// re-export aggregator. MIR-direct port of
/// `codegen::generate_imported_mirror`.
///
/// Iteration source is `mir.imports: BTreeMap<Symbol,
/// ImportedSpecMir>` (Phase 1c-7 unified imports). Each
/// `ImportedSpecMir.account_types` / `.records` is
/// `Vec<ParsedAccountType>` / `Vec<ParsedRecordType>` directly —
/// MIR mirrors the parsed shapes verbatim, so iteration is a 1:1
/// translation. `dep_key` extraction goes through the
/// `ImportOrigin` enum (`Builtin(k)` / `File(k)` both wrap a key;
/// `Inline` has no source artifact and never produces a mirror).
///
/// Tier-0 stubs (SPL Token, System Program, Metaplex bundled) have
/// `account_types.is_empty()` and skip both the per-namespace file
/// and the mod.rs re-export (matches legacy line ~1296–1302 +
/// 1505–1509 gating).
fn emit_imported_mirror(
    mir: &Mir,
    parsed: &ParsedSpec,
    fp: &crate::fingerprint::SpecFingerprint,
    output_dir: &Path,
    target: Target,
) -> Result<()> {
    // Early exit when no imported namespace carries account_types.
    if !mir
        .imports
        .values()
        .any(|imp| !imp.account_types.is_empty())
    {
        return Ok(());
    }

    let (prelude_import, explicit_account_discriminator): (&str, bool) = match target {
        Target::Anchor => ("use anchor_lang::prelude::*;\n", false),
        Target::Quasar => ("use quasar_lang::prelude::*;\n", true),
        Target::Pinocchio => unreachable!("Pinocchio is rejected at the init dispatcher"),
    };

    let src_dir = output_dir.join("src");
    let imported_dir = src_dir.join("imported");
    std::fs::create_dir_all(&imported_dir)?;

    // Per-namespace file emission. BTreeMap order is sorted by
    // alias for deterministic output.
    for (local_name, imp) in &mir.imports {
        if imp.account_types.is_empty() {
            continue;
        }
        let dep_key = match &imp.origin {
            crate::mir::ImportOrigin::Builtin(k) | crate::mir::ImportOrigin::File(k) => k.clone(),
            crate::mir::ImportOrigin::Inline => {
                // Inline interface — no source artifact + already
                // gated out by `account_types.is_empty()` above
                // (inline blocks never declare account_types). Skip
                // defensively.
                continue;
            }
        };

        let mut out = String::new();
        let file_rel = format!("src/imported/{}.rs", local_name);
        out.push_str(&crate::codegen::marker("DO NOT EDIT", fp, &file_rel));
        out.push_str(&format!(
            "//! v2.29 Slice H mirror of `{0}`'s account types\n\
             //! (sourced from dep `{1}`).\n\
             //!\n\
             //! Hand-editing is unsafe: every `qedgen codegen` regenerates\n\
             //! this file from the imported `.qedspec`'s `type` declarations.\n\
             //! To change a field, change the imported spec and re-resolve.\n\n",
            local_name, dep_key,
        ));
        out.push_str(prelude_import);
        out.push('\n');

        // Records — declared first so account_types can reference them.
        for record in &imp.records {
            out.push_str("#[repr(C)]\n");
            let derives = match target {
                Target::Anchor => "#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Copy, Debug, PartialEq)]\n",
                _ => "#[derive(Clone, Copy)]\n",
            };
            out.push_str(derives);
            out.push_str(&format!("pub struct {} {{\n", record.name));
            for (fname, ftype) in &record.fields {
                let rust_ty = crate::codegen::map_type_for_target(ftype, parsed, target)?;
                out.push_str(&format!("    pub {}: {},\n", fname, rust_ty));
            }
            out.push_str("}\n\n");
        }

        // Account types — single-variant flat struct or multi-
        // variant wrapper+inner enum, mirroring `generate_state`'s
        // dispatch shape.
        for (idx, acct) in imp.account_types.iter().enumerate() {
            let is_multi_variant = acct.variants.len() > 1;
            let account_attr = if explicit_account_discriminator {
                format!("#[account(discriminator = {})]\n", idx + 1)
            } else {
                "#[account]\n".to_string()
            };

            if !is_multi_variant {
                out.push_str(&format!("{}pub struct {} {{\n", account_attr, acct.name));
                for (fname, ftype) in &acct.fields {
                    let rust_ty = crate::codegen::map_type_for_target(ftype, parsed, target)?;
                    out.push_str(&format!("    pub {}: {},\n", fname, rust_ty));
                }
                if !acct.lifecycle.is_empty() && !acct.fields.iter().any(|(n, _)| n == "status") {
                    out.push_str("    pub status: u8,\n");
                }
                out.push_str("}\n\n");

                if !acct.lifecycle.is_empty() {
                    out.push_str(&format!(
                        "/// {} lifecycle states (mirrored from `{}`).\n",
                        acct.name, dep_key
                    ));
                    out.push_str("#[derive(Clone, Copy, PartialEq, Eq)]\n");
                    out.push_str("#[repr(u8)]\n");
                    out.push_str(&format!("pub enum {}Status {{\n", acct.name));
                    for (i, state) in acct.lifecycle.iter().enumerate() {
                        out.push_str(&format!("    {} = {},\n", state, i));
                    }
                    out.push_str("}\n\n");
                }
                continue;
            }

            // Multi-variant ADT: wrapper struct + inner enum.
            let inner_name = format!("{}Inner", acct.name);
            out.push_str(&format!("{}pub struct {} {{\n", account_attr, acct.name));
            out.push_str(&format!("    pub inner: {},\n", inner_name));
            out.push_str("}\n\n");

            out.push_str(&format!(
                "/// Variant-payload state for `{0}` (mirrored from `{1}`).\n",
                acct.name, dep_key
            ));
            out.push_str(
                "#[derive(AnchorSerialize, AnchorDeserialize, InitSpace, Clone, Debug, PartialEq)]\n",
            );
            out.push_str(&format!("pub enum {} {{\n", inner_name));
            for variant in &acct.variants {
                if variant.fields.is_empty() {
                    out.push_str(&format!("    {},\n", variant.name));
                } else {
                    out.push_str(&format!("    {} {{\n", variant.name));
                    for (fname, ftype) in &variant.fields {
                        out.push_str(&format!(
                            "        {}: {},\n",
                            fname,
                            crate::codegen::map_type_for_target(ftype, parsed, target)?
                        ));
                    }
                    out.push_str("    },\n");
                }
            }
            out.push_str("}\n\n");

            // Slice B accessor pattern — fields that appear with
            // consistent type across variants get an accessor.
            let mut field_index: std::collections::BTreeMap<String, Vec<(String, String)>> =
                std::collections::BTreeMap::new();
            for variant in &acct.variants {
                for (fname, ftype) in &variant.fields {
                    field_index
                        .entry(fname.clone())
                        .or_default()
                        .push((variant.name.clone(), ftype.clone()));
                }
            }
            if !field_index.is_empty() {
                out.push_str(&format!("impl {} {{\n", inner_name));
                for (fname, occurrences) in &field_index {
                    let first_ty = &occurrences[0].1;
                    if occurrences.iter().any(|(_, t)| t != first_ty) {
                        continue;
                    }
                    let rust_ty = crate::codegen::map_type_for_target(first_ty, parsed, target)?;
                    out.push_str(&format!(
                        "    /// v2.29 Slice H accessor for `{0}`. Panics on variants\n\
                         /// that don't carry the field — the per-handler lifecycle\n\
                         /// check at the top of each `crate::guards::*` fn prevents\n\
                         /// the panic arm from being reached at runtime.\n",
                        fname
                    ));
                    out.push_str(&format!(
                        "    pub fn {}(&self) -> &{} {{\n        match self {{\n",
                        fname, rust_ty
                    ));
                    for (variant_name, _) in occurrences {
                        out.push_str(&format!(
                            "            Self::{} {{ {}, .. }} => {},\n",
                            variant_name, fname, fname
                        ));
                    }
                    if occurrences.len() < acct.variants.len() {
                        out.push_str(&format!(
                            "            _ => panic!(\"{}::{}() called on a variant without `{}`\"),\n",
                            inner_name, fname, fname
                        ));
                    }
                    out.push_str("        }\n    }\n");
                }
                out.push_str("}\n\n");
            }
        }

        out.push_str("// ---- END GENERATED ----\n");
        std::fs::write(imported_dir.join(format!("{}.rs", local_name)), &out)?;
    }

    // mod.rs re-export aggregator. Mirrors legacy line ~1497–1511.
    let mut mod_out = String::new();
    mod_out.push_str(&crate::codegen::marker(
        "DO NOT EDIT",
        fp,
        "src/imported/mod.rs",
    ));
    mod_out.push_str("//! v2.29 Slice H — re-exports for imported namespace mirrors.\n\n");
    mod_out.push_str("#![allow(non_snake_case)]\n\n");
    for (local_name, imp) in &mir.imports {
        if imp.account_types.is_empty() {
            continue;
        }
        mod_out.push_str(&format!("pub mod {};\n", local_name));
    }
    mod_out.push_str("\n// ---- END GENERATED ----\n");
    std::fs::write(imported_dir.join("mod.rs"), mod_out)?;

    Ok(())
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
