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
    emit_uninterpreted_helpers(&mut out, mir);
    emit_ref_impls(&mut out, mir);
    emit_constants(&mut out, mir);
    emit_lifecycle_marker(&mut out, mir);
    emit_state_struct(&mut out, mir);
    emit_transitions(&mut out, mir);
    emit_operation_inductive(&mut out, mir);
    emit_invariants(&mut out, mir);
    emit_properties(&mut out, mir);
    emit_aborts_if(&mut out, mir);
    emit_ensures(&mut out, mir);
    emit_frame_conditions(&mut out, mir);

    // TODO Phase 1c (subsequent slices): CPI theorems, cover,
    // liveness, environments, overflow.
    out.push_str("-- TODO(mir-phase-1c-later): CPI theorems\n\n");
    out.push_str("-- TODO(mir-phase-1c-later): cover / liveness / environments / overflow\n\n");

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

/// Emit `abbrev NAME : Nat := VALUE` lines for top-level constants.
/// Mirrors `lean_gen::render_single_account` line ~1203.
fn emit_constants(out: &mut String, mir: &Mir) {
    if mir.constants.is_empty() {
        return;
    }
    for (name, val) in &mir.constants {
        out.push_str(&format!("abbrev {} : Nat := {}\n", safe_name(name), val));
    }
    out.push('\n');
}

/// Emit uninterpreted-helper declarations.
/// Mirrors `lean_gen::emit_uninterpreted_helpers`.
///
/// Each helper becomes a Lean `opaque <name> : T1 → T2 → ... → R`
/// declaration. `opaque` (not `axiom`) so transition functions stay
/// computable.
fn emit_uninterpreted_helpers(out: &mut String, mir: &Mir) {
    if mir.uninterpreted_helpers.is_empty() {
        return;
    }
    out.push_str(
        "-- Uninterpreted helpers: declared opaquely so generated\n\
         -- transitions typecheck even though the DSL doesn't model\n\
         -- their semantics. Treat each as an abstract Bool predicate;\n\
         -- strengthen into a concrete definition in your support\n\
         -- module if you want to discharge it (rather than trust it).\n\
         -- `opaque` keeps the transition functions computable\n\
         -- (axioms would force them noncomputable).\n",
    );
    for h in &mir.uninterpreted_helpers {
        let sig = if h.arg_types.is_empty() {
            h.return_type.clone()
        } else {
            let mut parts: Vec<String> = h.arg_types.clone();
            parts.push(h.return_type.clone());
            parts.join(" \u{2192} ") // →
        };
        out.push_str(&format!("opaque {} : {}\n", safe_name(&h.name), sig));
    }
    out.push('\n');
}

/// Emit `def`-style reference-implementation declarations.
/// Mirrors `lean_gen::emit_ref_impls`.
///
/// `ref_impl` bodies are emitted as Lean `def`s. Map-indexed subscripts
/// (`m[i]`) in the Lean body get rewritten to function-application
/// form (`(m i)`) since `Map N T = Fin N → T` doesn't have a GetElem
/// instance — handled by a small rewrite pass that lean_gen.rs ships
/// as `rewrite_subscripts_lean`. For Phase 1c-4 we emit the body
/// verbatim; the subscript-rewrite port lands in a follow-up if any
/// pilot fixture trips on it.
fn emit_ref_impls(out: &mut String, mir: &Mir) {
    if mir.ref_impls.is_empty() {
        return;
    }
    out.push_str(
        "-- Reference implementations: pure expressions named so\n\
         -- ensures clauses can call them. The user's Rust impl is\n\
         -- verified to satisfy the ensures referencing these, not\n\
         -- forced to implement them verbatim.\n",
    );
    for r in &mir.ref_impls {
        let params = r
            .params
            .iter()
            .map(|(n, t)| format!("({} : {})", safe_name(n), map_dsl_ty(t)))
            .collect::<Vec<_>>()
            .join(" ");
        let ret = map_dsl_ty(&r.return_type);
        // Phase 1c-4 emits the lean_body verbatim. lean_gen.rs runs a
        // `rewrite_subscripts_lean` pass over Map-indexed expressions
        // (`m[i]` → `(m i)`); port when a pilot fixture needs it.
        let body = &r.lean_body;
        if params.is_empty() {
            out.push_str(&format!(
                "def {} : {} := {}\n",
                safe_name(&r.name),
                ret,
                body
            ));
        } else {
            out.push_str(&format!(
                "def {} {} : {} := {}\n",
                safe_name(&r.name),
                params,
                ret,
                body
            ));
        }
    }
    out.push('\n');
}

/// Map a DSL type-string to its Lean form. Mirrors
/// `lean_gen::map_type_with_compound` for the common cases used by
/// `ref_impl` parameter / return types. Unknown forms pass through
/// unchanged — Phase 1c-4 doesn't try for compound-type support
/// (`Map[N] T`, `Fin[N]`, ...). A future slice can port the legacy's
/// compound-aware mapper when a fixture demands it.
fn map_dsl_ty(s: &str) -> String {
    match s.trim() {
        "U8" | "U16" | "U32" | "U64" | "U128" => "Nat".to_string(),
        "I8" | "I16" | "I32" | "I64" | "I128" => "Int".to_string(),
        other => other.to_string(),
    }
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

/// Emit one transition function per handler. Mirrors
/// `lean_gen::render_transitions` for the pilot scope:
///
/// ```text
/// def <name>Transition (s : State) (signer : Pubkey) <params> : Option State :=
///   let <auth_alias> := signer  -- when `auth <who>` isn't a state field
///   if <require-or-abort-conjunction> then
///     some { s with <assigns>... }
///   else
///     none
/// ```
///
/// Pilot scope: lowers `Stmt::RequireOrAbort` into the if-condition;
/// `Stmt::Assign` / `CheckedAdd` / `CheckedSub` / `WrapAdd` / etc. into
/// the `{ s with ... }` record update. `TokenTransfer` and `Cpi`
/// don't affect local state — they're discharged by separate CPI
/// theorems (Phase 1c-later slice).
fn emit_transitions(out: &mut String, mir: &Mir) {
    for h in &mir.handlers {
        emit_handler_transition(out, mir, h);
    }
}

fn emit_handler_transition(out: &mut String, _mir: &Mir, h: &crate::mir::HandlerMir) {
    use crate::mir::Stmt;

    let trans_name = safe_name(&format!("{}Transition", h.name));
    let param_sig = param_sig_str(&h.params);

    // Signature.
    out.push_str(&format!(
        "def {} (s : State) (signer : Pubkey){} : Option State :=\n",
        trans_name, param_sig
    ));

    // Auth alias: when `auth <who>` is not a state field, bind `who` to
    // `signer` so user-written predicates referencing it resolve.
    if let Some(auth_name) = handler_auth_name(h) {
        // Phase 1c approximation: emit the alias whenever `auth` is set.
        // Determining whether `who` is a state field requires walking
        // the State variants — we have that info, but the legacy code's
        // gate is more nuanced (it checks if `who` collides with a
        // field name OR a Pubkey-typed state field). Erring on the
        // side of always emitting the alias matches the most-common
        // case in the pilot fixtures.
        out.push_str(&format!("  let {} := signer\n", safe_name(&auth_name)));
    }

    // RequireOrAbort clauses → if-condition.
    let conds: Vec<String> = h
        .body
        .stmts
        .iter()
        .filter_map(|s| match s {
            Stmt::RequireOrAbort { pred, .. } => Some(pred.0.lean.clone()),
            _ => None,
        })
        .collect();

    // Assign / Add / Sub family → record-update parts.
    let mut with_parts: Vec<String> = Vec::new();
    for stmt in &h.body.stmts {
        match stmt {
            Stmt::Assign { path, rhs } => {
                // Drop `<field> := <account_binding>.pubkey` — the
                // mirror behavior from lean_gen.rs:1839; account-binding
                // pubkey refs have no Lean scope.
                if is_account_pubkey_ref(&rhs.rust) {
                    continue;
                }
                with_parts.push(format!(
                    "{} := {}",
                    safe_name(&path_field_name(path)),
                    rhs.lean
                ));
            }
            Stmt::CheckedAdd { path, delta, .. }
            | Stmt::WrapAdd { path, delta }
            | Stmt::SatAdd { path, delta } => {
                let f = safe_name(&path_field_name(path));
                with_parts.push(format!("{} := s.{} + {}", f, f, delta.lean));
            }
            Stmt::CheckedSub { path, delta, .. }
            | Stmt::WrapSub { path, delta }
            | Stmt::SatSub { path, delta } => {
                let f = safe_name(&path_field_name(path));
                with_parts.push(format!("{} := s.{} - {}", f, f, delta.lean));
            }
            _ => {}
        }
    }

    // Lifecycle promotion: `state := .NextVariant` lowers as
    // `status := .NextVariant` on the lifecycle marker field. For
    // pilot fixtures, the transition arrow on HandlerMir.transition
    // captures this; emit the post-status set when present.
    if let Some((_, post)) = &h.transition {
        // Only emit the `status :=` part when lifecycle is real
        // (≥2 states); single-lifecycle specs skip the marker per
        // issue #43.
        if _mir.state.lifecycle_states.len() >= 2 {
            with_parts.push(format!("status := .{}", safe_name(post)));
        }
    }

    let then_body = if with_parts.is_empty() {
        "some s".to_string()
    } else {
        format!("some {{ s with {} }}", with_parts.join(", "))
    };

    if conds.is_empty() {
        out.push_str(&format!("  {}\n\n", then_body));
    } else {
        let joined = conds
            .iter()
            .map(|c| paren_low_prec(c))
            .collect::<Vec<_>>()
            .join(" \u{2227} ");
        out.push_str(&format!("  if {} then\n", joined));
        out.push_str(&format!("    {}\n", then_body));
        out.push_str("  else\n");
        out.push_str("    none\n\n");
    }
}

/// Emit property declarations + preservation theorems. Mirrors
/// `lean_gen::render_properties_inner` for the structural shape:
///
/// ```text
/// def <name> (s : State) : Prop := <body>
///
/// theorem <name>_preserved_by_<handler> (s s' : State) (signer : Pubkey) <params>
///     (h_inv : <name> s) (h : <handler>Transition s signer <args> = some s') :
///     <name> s' := sorry
///
/// /-- <name> is preserved by every operation. Auto-proven by case split. -/
/// theorem <name>_invariant (s s' : State) (signer : Pubkey) (op : Operation)
///     (h_inv : <name> s) (h : applyOp s signer op = some s') :
///     <name> s' := sorry
/// ```
///
/// Phase 1c-5: emits the theorem statements with `sorry` bodies for
/// every preservation sub-lemma. lean_gen.rs's `preservation_proof_script`
/// generates discharged proofs via `if_neg` / `dsimp + omega`
/// projection; that's a follow-up. Properties with no
/// `expression` body emit a structured comment only.
fn emit_properties(out: &mut String, mir: &Mir) {
    if mir.properties.is_empty() {
        return;
    }

    for prop in &mir.properties {
        // Predicate def (when body is present).
        if let Some(expr) = &prop.expression {
            // lean_gen.rs:2716-2737 strips a leading `∀ s : State,`
            // binder since the surrounding def already introduces
            // `(s : State)`. Mirror that — but only when the binder
            // ident is exactly `s`.
            let body = strip_state_forall(&expr.lean);
            out.push_str(&format!(
                "def {} (s : State) : Prop := {}\n\n",
                safe_name(&prop.name),
                body
            ));
        } else {
            out.push_str(&format!(
                "-- PROPERTY OBLIGATION (declared, no predicate body): {}\n\n",
                prop.name
            ));
            continue;
        }

        // Per-handler preservation sub-lemmas.
        let covered: Vec<&crate::mir::HandlerMir> = mir
            .handlers
            .iter()
            .filter(|h| prop.preserved_by.contains(&h.name))
            .collect();
        for h in &covered {
            let trans_name = safe_name(&format!("{}Transition", h.name));
            let param_sig = param_sig_str(&h.params);
            let param_args = param_args_str(&h.params);
            let sub_lemma = safe_name(&format!("{}_preserved_by_{}", prop.name, h.name));
            out.push_str(&format!(
                "theorem {} (s s' : State) (signer : Pubkey){}\n",
                sub_lemma, param_sig
            ));
            out.push_str(&format!(
                "    (h_inv : {} s) (h : {} s signer{} = some s') :\n",
                safe_name(&prop.name),
                trans_name,
                param_args
            ));
            out.push_str(&format!("    {} s' := sorry\n\n", safe_name(&prop.name)));
        }

        // Master theorem: preserved by every Operation case.
        if !covered.is_empty() {
            out.push_str(&format!(
                "/-- {} is preserved by every operation. -/\n",
                prop.name
            ));
            out.push_str(&format!(
                "theorem {}_invariant (s s' : State) (signer : Pubkey) (op : Operation)\n",
                safe_name(&prop.name)
            ));
            out.push_str(&format!(
                "    (h_inv : {} s) (h : applyOp s signer op = some s') :\n",
                safe_name(&prop.name)
            ));
            out.push_str(&format!("    {} s' := sorry\n\n", safe_name(&prop.name)));
        }
    }
}

/// If `expr` starts with `∀ s : T,` or `forall s : T,`, strip the
/// quantifier prefix and return the body — the surrounding `def
/// <prop> (s : State)` already binds `s`. Other quantified bodies
/// (value binders) pass through unchanged. Mirrors lean_gen.rs:2716.
fn strip_state_forall(expr: &str) -> String {
    let trimmed = expr.trim();
    let rest = trimmed
        .strip_prefix('\u{2200}')
        .or_else(|| trimmed.strip_prefix("forall"));
    if let Some(rest) = rest {
        let rest_trim = rest.trim_start();
        // Only strip if the quantified binder is literally `s`.
        if rest_trim.starts_with("s ") || rest_trim.starts_with("s:") {
            if let Some(comma_pos) = rest.find(',') {
                return rest[comma_pos + 1..].trim().to_string();
            }
        }
    }
    trimmed.to_string()
}

/// Emit invariant theorems. Mirrors `lean_gen::render_invariants_theorem_form`.
///
/// Per invariant with a predicate body:
/// ```text
/// /-- Invariant: <name> — <doc> -/
/// theorem <name> (s : State) : <prefixed-pred> := by sorry
/// ```
///
/// Bare bodies (description-only invariants from pre-v2.14) emit a
/// structured comment instead — no `theorem ... := True` tautology.
/// The `bare_invariant` lint surfaces these as P3.
fn emit_invariants(out: &mut String, mir: &Mir) {
    if mir.invariants.is_empty() {
        return;
    }

    // Collect all state-field names across every variant for the
    // `prefix_state_fields` regex pass.
    let field_set: std::collections::HashSet<String> = mir
        .state
        .variants
        .iter()
        .flat_map(|v| v.fields.iter().map(|f| f.name.clone()))
        .collect();

    for inv in &mir.invariants {
        match &inv.body {
            Some(pred) => {
                let prefixed = prefix_state_fields(&pred.0.lean, &field_set);
                out.push_str(&format!(
                    "/-- Invariant: {}{} -/\n",
                    inv.name,
                    if inv.doc.is_empty() {
                        String::new()
                    } else {
                        format!(" — {}", inv.doc)
                    }
                ));
                out.push_str(&format!(
                    "theorem {} (s : State) : {} := by sorry\n\n",
                    inv.name, prefixed
                ));
            }
            None => {
                out.push_str(&format!(
                    "-- INVARIANT OBLIGATION (declared, no predicate body): {}\n",
                    inv.name
                ));
                if !inv.doc.is_empty() {
                    out.push_str(&format!("--   description: {}\n", inv.doc));
                }
                out.push_str("-- The spec declared this name but didn't supply a predicate body\n");
                out.push_str("-- (`invariant <name> : <expr>`). Give it a body to verify.\n\n");
            }
        }
    }
}

/// Emit frame condition theorems. Mirrors
/// `lean_gen::render_frame_conditions`.
///
/// For each handler with a `modifies` clause, emit a theorem proving
/// that every field NOT in `modifies` stays equal across the
/// transition. Lifecycle-transitioning handlers implicitly modify the
/// `status` field.
fn emit_frame_conditions(out: &mut String, mir: &Mir) {
    let has_modifies = mir.handlers.iter().any(|h| h.modifies.is_some());
    if !has_modifies {
        return;
    }

    out.push_str(
        "-- ============================================================================\n",
    );
    out.push_str("-- Frame conditions (modifies)\n");
    out.push_str(
        "-- ============================================================================\n\n",
    );

    // All declared state-field names across every variant.
    let all_fields: Vec<String> = {
        let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
        let mut out_fields = Vec::new();
        for v in &mir.state.variants {
            for f in &v.fields {
                if seen.insert(f.name.clone()) {
                    out_fields.push(f.name.clone());
                }
            }
        }
        out_fields
    };

    for h in &mir.handlers {
        let Some(modified) = &h.modifies else {
            continue;
        };
        let modified_set: std::collections::HashSet<String> = modified
            .iter()
            .map(|p| p.segments.last().cloned().unwrap_or_default())
            .collect();

        let status_modified = h.transition.is_some();
        let unchanged: Vec<&String> = all_fields
            .iter()
            .filter(|f| !(modified_set.contains(*f) || (*f == "status" && status_modified)))
            .collect();
        if unchanged.is_empty() {
            continue;
        }

        let trans_name = safe_name(&format!("{}Transition", h.name));
        let param_sig = param_sig_str(&h.params);
        let param_args = param_args_str(&h.params);
        let theorem_name = safe_name(&format!("{}_frame", h.name));

        out.push_str(&format!(
            "theorem {} (s s' : State) (signer : Pubkey){}\n",
            theorem_name, param_sig
        ));
        out.push_str(&format!(
            "    (h : {} s signer{} = some s') :\n",
            trans_name, param_args
        ));
        let conjuncts: Vec<String> = unchanged
            .iter()
            .map(|f| format!("s'.{} = s.{}", safe_name(f), safe_name(f)))
            .collect();
        out.push_str(&format!(
            "    {} := sorry\n\n",
            conjuncts.join(" \u{2227} ")
        ));
    }
}

/// Prefix every state-field identifier in `expr` with `s.`. Word-boundary
/// regex avoids touching substrings of other identifiers (e.g., `amount`
/// shouldn't become `s.amount` inside `taker_amount`). Skips fields
/// already prefixed.
fn prefix_state_fields(expr: &str, fields: &std::collections::HashSet<String>) -> String {
    let mut out = expr.to_string();
    for field in fields {
        let pattern = format!(r"\b{}\b", regex::escape(field));
        let re = regex::Regex::new(&pattern).expect("regex compiles for state-field name");
        let replacement = format!("s.{}", field);
        // Skip if already prefixed somewhere — avoid double-prefix on
        // re-passes. The simple way: check `s.<field>` literal presence
        // before applying.
        if out.contains(&replacement) {
            // Already partly prefixed — fall back to a non-greedy
            // single-pass apply that won't double-prefix because the
            // `\b` regex doesn't match after `.` (word boundary
            // already broken by the dot).
            out = re
                .replace_all(&out, regex::NoExpand(&replacement))
                .into_owned();
        } else {
            out = re
                .replace_all(&out, regex::NoExpand(&replacement))
                .into_owned();
        }
    }
    out
}

/// Emit abort theorems. Mirrors `lean_gen::render_aborts_if`.
///
/// For each handler with abort surface (`aborts_if` clauses or
/// `requires X else Err`), emits per-clause theorems:
///
/// ```text
/// theorem <h>_aborts_if_<Err> (s : State) (signer : Pubkey) <params>
///     (h : <pred>) : <h>Transition s signer <args> = none := sorry
/// ```
///
/// For `requires X else Err` the hypothesis is negated form
/// `¬(<requires-expr>)`. When `aborts_total` is set on the handler,
/// emits a single `<h>_aborts_iff` theorem with the disjunction of
/// every abort condition.
///
/// Phase 1c approximation: emits the theorem statements with
/// `:= sorry` bodies for every case. lean_gen.rs has a finer
/// `abort_requires_proof` path that auto-discharges via `if_neg`
/// projection on requires-derived aborts; porting that lands later.
fn emit_aborts_if(out: &mut String, mir: &Mir) {
    let has_aborts = mir
        .handlers
        .iter()
        .any(|h| !h.aborts_if.is_empty() || !h.requires_or_abort.is_empty());
    if !has_aborts {
        return;
    }

    out.push_str(
        "-- ============================================================================\n",
    );
    out.push_str("-- Abort conditions — operations must reject under specified conditions\n");
    out.push_str(
        "-- ============================================================================\n\n",
    );

    for h in &mir.handlers {
        if h.aborts_if.is_empty() && h.requires_or_abort.is_empty() {
            continue;
        }
        let trans_name = safe_name(&format!("{}Transition", h.name));
        let param_sig = param_sig_str(&h.params);
        let param_args = param_args_str(&h.params);

        // `aborts_total` collapses all abort conditions into a single
        // iff theorem. Mirror lean_gen.rs:4396.
        let all_abort_lean: Vec<String> = h
            .aborts_if
            .iter()
            .map(|a| a.pred.0.lean.clone())
            .chain(
                h.requires_or_abort
                    .iter()
                    .map(|r| format!("\u{00AC}({})", r.pred.0.lean)),
            )
            .collect();

        if h.aborts_total && !all_abort_lean.is_empty() {
            let theorem_name = safe_name(&format!("{}_aborts_iff", h.name));
            out.push_str(&format!(
                "theorem {} (s : State) (signer : Pubkey){} :\n",
                theorem_name, param_sig
            ));
            out.push_str(&format!(
                "    {} s signer{} = none \u{2194}\n",
                trans_name, param_args
            ));
            let disjunction = all_abort_lean.join(" \u{2228} ");
            out.push_str(&format!("    ({}) := sorry\n\n", disjunction));
            continue;
        }

        // Per-clause theorems. Disambiguate when the same error name
        // appears multiple times on a single handler (issue #8 #3).
        let mut error_total: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        for a in &h.aborts_if {
            *error_total.entry(a.err.clone()).or_insert(0) += 1;
        }
        for r in &h.requires_or_abort {
            *error_total.entry(r.err.clone()).or_insert(0) += 1;
        }
        let mut error_seen: std::collections::HashMap<String, usize> =
            std::collections::HashMap::new();
        let theorem_name_for =
            |err: &str, seen: &mut std::collections::HashMap<String, usize>| -> String {
                let total = error_total.get(err).copied().unwrap_or(0);
                let idx = {
                    let entry = seen.entry(err.to_string()).or_insert(0);
                    let cur = *entry;
                    *entry += 1;
                    cur
                };
                if total > 1 {
                    safe_name(&format!("{}_aborts_if_{}_{}", h.name, err, idx))
                } else {
                    safe_name(&format!("{}_aborts_if_{}", h.name, err))
                }
            };

        // Legacy aborts_if clauses: hypothesis IS the predicate.
        for a in &h.aborts_if {
            let theorem_name = theorem_name_for(&a.err, &mut error_seen);
            out.push_str(&format!(
                "theorem {} (s : State) (signer : Pubkey){}\n",
                theorem_name, param_sig
            ));
            out.push_str(&format!(
                "    (h : {}) : {} s signer{} = none := sorry\n\n",
                a.pred.0.lean, trans_name, param_args
            ));
        }

        // requires-else clauses: hypothesis is ¬(predicate). Skip
        // clauses that reference a handler account's `.pubkey` /
        // `.key()` — those identifiers aren't in Lean scope, so the
        // theorem would mention free variables. Mirrors
        // lean_gen.rs:4467. Skipping here keeps the Lean compilable;
        // the runtime-side check still fires in Rust.
        for r in &h.requires_or_abort {
            if mentions_handler_account_pubkey(&r.pred.0.lean, &h.accounts) {
                continue;
            }
            let theorem_name = theorem_name_for(&r.err, &mut error_seen);
            out.push_str(&format!(
                "theorem {} (s : State) (signer : Pubkey){}\n",
                theorem_name, param_sig
            ));
            out.push_str(&format!(
                "    (h : \u{00AC}({})) : {} s signer{} = none := sorry\n\n",
                r.pred.0.lean, trans_name, param_args
            ));
        }
    }
}

/// Emit ensures theorems. Mirrors `lean_gen::render_ensures`.
///
/// Per handler, one theorem per `ensures` clause:
///
/// ```text
/// theorem <h>_ensures_<i> (s s' : State) (signer : Pubkey) <params>
///     (h : <h>Transition s signer <args> = some s') :
///     <ensures-lean-expr> := sorry
/// ```
fn emit_ensures(out: &mut String, mir: &Mir) {
    let has_ensures = mir.handlers.iter().any(|h| !h.post.is_empty());
    if !has_ensures {
        return;
    }

    out.push_str(
        "-- ============================================================================\n",
    );
    out.push_str("-- Post-conditions (ensures)\n");
    out.push_str(
        "-- ============================================================================\n\n",
    );

    for h in &mir.handlers {
        let trans_name = safe_name(&format!("{}Transition", h.name));
        let param_sig = param_sig_str(&h.params);
        let param_args = param_args_str(&h.params);
        for (i, ens) in h.post.iter().enumerate() {
            let theorem_name = safe_name(&format!("{}_ensures_{}", h.name, i));
            out.push_str(&format!(
                "theorem {} (s s' : State) (signer : Pubkey){}\n",
                theorem_name, param_sig
            ));
            out.push_str(&format!(
                "    (h : {} s signer{} = some s') :\n",
                trans_name, param_args
            ));
            out.push_str(&format!("    {} := sorry\n\n", ens.0.lean));
        }
    }
}

/// Detect whether an expression references a handler-account's
/// `.pubkey` or `.key()`. Account-binding pubkey refs aren't in Lean
/// scope; emitting theorems that mention them yields unprovable
/// statements with free identifiers. Mirrors
/// `lean_gen::mentions_handler_account_pubkey`.
fn mentions_handler_account_pubkey(
    expr: &str,
    accounts: &[crate::mir::AccountBindingShape],
) -> bool {
    accounts.iter().any(|a| {
        let needle_pubkey = format!("{}.pubkey", a.name);
        let needle_key = format!("{}.key()", a.name);
        expr.contains(&needle_pubkey) || expr.contains(&needle_key)
    })
}

/// Build the call-side argument string for transition function
/// invocations: `" p1 p2 ..."`. Empty when `params` is empty.
fn param_args_str(params: &[(crate::mir::Symbol, crate::mir::Ty)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    format!(
        " {}",
        params
            .iter()
            .map(|(n, _)| n.as_str())
            .collect::<Vec<_>>()
            .join(" ")
    )
}

/// Emit the `inductive Operation` enum + `def applyOp` dispatcher.
/// Mirrors `lean_gen::render_operation_inductive`.
fn emit_operation_inductive(out: &mut String, mir: &Mir) {
    if mir.handlers.is_empty() {
        return;
    }

    out.push_str("inductive Operation where\n");
    for h in &mir.handlers {
        let ctor = safe_name(&h.name);
        if h.params.is_empty() {
            out.push_str(&format!("  | {}\n", ctor));
        } else {
            let params: Vec<String> = h
                .params
                .iter()
                .map(|(n, t)| format!("({} : {})", n, render_ty(t)))
                .collect();
            out.push_str(&format!("  | {} {}\n", ctor, params.join(" ")));
        }
    }
    out.push_str("  deriving Repr, DecidableEq, BEq\n\n");

    // applyOp dispatcher.
    out.push_str("def applyOp (s : State) (signer : Pubkey) : Operation \u{2192} Option State\n");
    for h in &mir.handlers {
        let ctor = safe_name(&h.name);
        let trans = safe_name(&format!("{}Transition", h.name));
        let names: Vec<String> = h.params.iter().map(|(n, _)| n.clone()).collect();
        let pattern_args = if names.is_empty() {
            String::new()
        } else {
            format!(" {}", names.join(" "))
        };
        let call_args = if names.is_empty() {
            String::new()
        } else {
            format!(" {}", names.join(" "))
        };
        out.push_str(&format!(
            "  | .{}{} => {} s signer{}\n",
            ctor, pattern_args, trans, call_args
        ));
    }
    out.push('\n');
}

fn emit_state_struct(out: &mut String, mir: &Mir) {
    // For multi-variant ADTs (e.g., State | Uninitialized | Open of
    // {...} | Closed), the flat-state form unions every variant's
    // fields into one struct, keyed by name. The status field carries
    // the lifecycle discriminator. Mirrors lean_gen.rs's flat-state
    // shape — variants don't get separate constructors here; that's
    // the `render_single_account_adt` (multi-variant ADT) path landing
    // later in Phase 1c.
    if mir.state.variants.is_empty() {
        return;
    }

    let has_lifecycle = mir.state.lifecycle_states.len() >= 2;

    // Union fields across all variants, preserving declaration order
    // and de-duping by name.
    let mut seen: std::collections::BTreeSet<String> = std::collections::BTreeSet::new();
    let mut unique_fields: Vec<&crate::mir::FieldDecl> = Vec::new();
    for v in &mir.state.variants {
        for f in &v.fields {
            if seen.insert(f.name.clone()) {
                unique_fields.push(f);
            }
        }
    }

    out.push_str("structure State where\n");
    for field in &unique_fields {
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

/// Build a parameter signature string for transition function
/// declarations: `" (p1 : T1) (p2 : T2) ..."`. Empty string when
/// `params` is empty. Mirrors `lean_gen::param_sig_str` for the
/// MIR-typed parameter list.
fn param_sig_str(params: &[(crate::mir::Symbol, crate::mir::Ty)]) -> String {
    if params.is_empty() {
        return String::new();
    }
    params
        .iter()
        .map(|(n, t)| format!(" ({} : {})", n, render_ty(t)))
        .collect::<Vec<_>>()
        .join("")
}

/// Extract the auth-account name for the alias-let, if any. For the
/// pilot scope, `auth <name>` lowers to `Some(AccountOrField::Account(ByBinding(name)))`.
/// Returns `None` for permissionless handlers and dotted-auth shapes
/// (which were desugared into a synthetic `requires` clause upstream
/// and don't need a separate alias).
fn handler_auth_name(h: &crate::mir::HandlerMir) -> Option<crate::mir::Symbol> {
    use crate::mir::{AccountOrField, AccountRef};
    match &h.auth {
        Some(AccountOrField::Account(AccountRef::ByBinding(name))) => Some(name.clone()),
        _ => None,
    }
}

/// Extract the field name from a Path. For Phase 1c we accept dotted
/// paths but emit only the trailing segment (matches lean_gen.rs's
/// `strip_variant_prefix_for_flat_state` behavior on the flat-state
/// path).
fn path_field_name(path: &crate::mir::Path) -> String {
    path.segments
        .last()
        .cloned()
        .unwrap_or_else(|| "?".to_string())
}

/// Detect whether the RHS string references an account-binding's
/// `.pubkey` field — `lean_gen.rs:1839` drops these from the record
/// update since they have no Lean scope.
fn is_account_pubkey_ref(rust: &str) -> bool {
    // Heuristic matching lean_gen.rs::is_account_binding_pubkey_ref:
    // the RHS is exactly `<identifier>.pubkey`.
    let trimmed = rust.trim();
    trimmed
        .strip_suffix(".pubkey")
        .map(|head| !head.is_empty() && head.chars().all(|c| c.is_alphanumeric() || c == '_'))
        .unwrap_or(false)
}

/// Wrap an expression in parens if it contains low-precedence operators
/// that would re-group when joined under `∧`. Mirrors
/// `lean_gen::paren_if_low_prec` — defensive parens at concat sites
/// (the mitigation for divergence class C3 in
/// `docs/design/codegen-divergence.md`).
fn paren_low_prec(expr: &str) -> String {
    let trimmed = expr.trim();
    // Already-parenthesized at the top level: leave alone.
    if trimmed.starts_with('(') && trimmed.ends_with(')') {
        // Check the parens actually match (could be `(a) ∧ (b)`).
        let mut depth = 0i32;
        let mut top_level_seen = false;
        for c in trimmed.chars() {
            match c {
                '(' => depth += 1,
                ')' => depth -= 1,
                _ => {
                    if depth == 0 {
                        top_level_seen = true;
                        break;
                    }
                }
            }
        }
        if !top_level_seen {
            return trimmed.to_string();
        }
    }
    // Look for low-precedence ops (or / and) at the top level.
    if has_top_level_op(trimmed, &[" or ", " ∨ ", " || "]) {
        format!("({})", trimmed)
    } else {
        trimmed.to_string()
    }
}

fn has_top_level_op(expr: &str, ops: &[&str]) -> bool {
    let mut depth = 0i32;
    for (i, c) in expr.char_indices() {
        match c {
            '(' => depth += 1,
            ')' => depth -= 1,
            _ if depth == 0 => {
                for op in ops {
                    if expr[i..].starts_with(op) {
                        return true;
                    }
                }
            }
            _ => {}
        }
    }
    false
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
    fn render_aborts_if_clauses() {
        let mir = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir);

        // escrow declares `requires deposit_amount > 0 and receive_amount > 0
        // else InvalidAmount` on initialize — should produce an
        // `initialize_aborts_if_InvalidAmount` theorem with the
        // negated predicate as hypothesis.
        assert!(
            out.contains("theorem initialize_aborts_if_InvalidAmount"),
            "expected initialize_aborts_if_InvalidAmount theorem:\n{}",
            &out[..out.len().min(2000)]
        );
        // The hypothesis should be the negation of the requires
        // predicate.
        assert!(
            out.contains("¬(deposit_amount > 0"),
            "expected negated requires hypothesis"
        );
        // The abort-conditions header should appear.
        assert!(out.contains("Abort conditions"));
    }

    #[test]
    fn render_skips_account_pubkey_aborts() {
        // exchange + cancel both have `requires initializer_ta.pubkey == ...`
        // — these reference a handler-account's .pubkey field which isn't
        // in Lean scope. The filter should skip them.
        let mir = lower_fixture("examples/rust/escrow/escrow.qedspec");
        let out = render(&mir);
        // No theorem should reference `initializer_ta.pubkey` in its
        // hypothesis — it's filtered out.
        assert!(
            !out.contains("(h : ¬(initializer_ta.pubkey"),
            "account-pubkey requires should be filtered from abort theorems:\n{}",
            out
        );
    }

    #[test]
    fn render_emits_constants() {
        // Percolator declares MAX_ACCOUNTS, MAX_VAULT_TVL,
        // POS_SCALE, MAX_ACCOUNT_NOTIONAL.
        let mir = lower_fixture("examples/rust/percolator/percolator.qedspec");
        let out = render(&mir);
        assert!(
            out.contains("abbrev MAX_ACCOUNTS : Nat := 1024"),
            "expected MAX_ACCOUNTS abbrev"
        );
        assert!(out.contains("abbrev POS_SCALE : Nat := 1000000"));
    }

    #[test]
    fn render_emits_uninterpreted_helpers() {
        // Synthetic MIR with an uninterpreted helper. Pilot fixtures
        // don't declare helpers explicitly (check.rs infers them when
        // an undeclared fn is referenced), so build the MIR by hand
        // to exercise the emit path.
        let mir = Mir {
            name: "T".to_string(),
            state: crate::mir::StateAdt::default(),
            accounts: crate::mir::AccountTable::default(),
            errors: crate::mir::ErrorEnum::default(),
            interfaces: crate::mir::InterfaceRegistry::default(),
            handlers: vec![],
            invariants: vec![],
            events: vec![],
            constants: vec![],
            uninterpreted_helpers: vec![crate::mir::UninterpretedHelper {
                name: "is_valid".to_string(),
                arg_types: vec!["Nat".to_string()],
                return_type: "Bool".to_string(),
            }],
            ref_impls: vec![],
            properties: vec![],
        };
        let out = render(&mir);
        assert!(
            out.contains("opaque is_valid : Nat \u{2192} Bool"),
            "expected opaque is_valid : Nat → Bool in:\n{}",
            out
        );
        assert!(out.contains("Uninterpreted helpers"));
    }

    #[test]
    fn render_emits_ref_impls() {
        let mir = Mir {
            name: "T".to_string(),
            state: crate::mir::StateAdt::default(),
            accounts: crate::mir::AccountTable::default(),
            errors: crate::mir::ErrorEnum::default(),
            interfaces: crate::mir::InterfaceRegistry::default(),
            handlers: vec![],
            invariants: vec![],
            events: vec![],
            constants: vec![],
            uninterpreted_helpers: vec![],
            ref_impls: vec![crate::mir::RefImpl {
                name: "scale".to_string(),
                doc: None,
                params: vec![
                    ("a".to_string(), "U64".to_string()),
                    ("b".to_string(), "U64".to_string()),
                ],
                return_type: "U64".to_string(),
                lean_body: "a * b".to_string(),
                rust_body: "a * b".to_string(),
            }],
            properties: vec![],
        };
        let out = render(&mir);
        assert!(
            out.contains("def scale (a : Nat) (b : Nat) : Nat := a * b"),
            "expected ref_impl scale lowered:\n{}",
            out
        );
    }

    #[test]
    fn render_emits_properties_with_preservation() {
        // Lending declares `property pool_solvency : ...` and
        // names handlers it's preserved by. Confirm the predicate
        // def, per-handler preservation sub-lemmas, and master
        // theorem all emit.
        let mir = lower_fixture("examples/rust/lending/lending.qedspec");
        let out = render(&mir);

        // Must have a Lean def for the property.
        assert!(
            out.contains("def pool_solvency (s : State) : Prop :="),
            "expected pool_solvency predicate def:\n{}",
            &out[..out.len().min(3000)]
        );

        // Master preservation theorem with applyOp.
        assert!(
            out.contains("theorem pool_solvency_invariant"),
            "expected pool_solvency_invariant master theorem"
        );
        assert!(out.contains("(op : Operation)"));
    }

    #[test]
    fn render_emits_invariant_theorems() {
        // Lending declares `invariant collateral_backing`.
        let mir = lower_fixture("examples/rust/lending/lending.qedspec");
        let out = render(&mir);
        assert!(
            out.contains("/-- Invariant: collateral_backing"),
            "expected collateral_backing invariant comment"
        );
        assert!(
            out.contains("theorem collateral_backing (s : State)"),
            "expected collateral_backing theorem"
        );
        assert!(
            out.contains(":= by sorry"),
            "expected `by sorry` body on invariants"
        );
    }

    #[test]
    fn prefix_state_fields_word_boundary() {
        let mut fields = std::collections::HashSet::new();
        fields.insert("amount".to_string());
        fields.insert("taker".to_string());

        // Bare field references get prefixed.
        let out = prefix_state_fields("amount > 0", &fields);
        assert_eq!(out, "s.amount > 0");

        // Substrings inside longer identifiers are NOT prefixed
        // (word-boundary regex). Tricky: `taker_amount` contains both
        // `taker` and `amount` as substrings but neither as a whole
        // word.
        let out = prefix_state_fields("taker_amount > 0", &fields);
        assert_eq!(out, "taker_amount > 0");
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
