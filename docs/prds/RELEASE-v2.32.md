# QEDGen v2.32 — Legacy-codegen migration & deletion

**Status:** shipped (on `feat/v2.32-records-to-mir`). **Theme:** finish the MIR
migration for the spec classes still pinned to legacy, *then* delete the Lean +
Kani legacy modules.

## Why this was a migration, not a clean deletion

The v2.30→v2.32 sequencing assumed v2.32 could just delete `lean_gen.rs`
(~8.7K LoC) + `kani.rs` (~2.4K LoC) once the `QEDGEN_LEGACY_*` soak was clean.
It couldn't — the soak left those paths **load-bearing**. The `main.rs` dispatch
forced legacy regardless of the env var whenever `parsed.is_assembly_target()`
(Lean + Kani) or `!parsed.records.is_empty()` (Lean), because the MIR emitters
didn't yet handle sBPF or record-bearing specs. Deleting then would have
regressed all five sBPF examples + percolator. Hence v2.32 was a migration:
close the gaps, prove parity, delete last.

## What shipped (in dependency order)

1. **Records → `lean_gen_mir`.** Emit `structure T` + `instance Inhabited T`
   for each `type T { … }`, plus the bare-field-assign wrapping
   (`{ acct with … }`) and indexed-record-field effects. Percolator regenerates
   byte-identical to its committed `Spec.lean` (gated by
   `mir_snapshot::snapshot_percolator`).

2. **sBPF → `lean_gen_mir` (Lean only).** `mir::lower` lifts the assembly target
   into `Mir.is_assembly`; `lean_gen_mir::generate` dispatches to a ported
   `render_sbpf` that reads `ParsedSpec` directly. **Reframe:** sBPF gets **no**
   Kani or proptest harnesses — assembly is verified by Lean proofs + client-side
   tests, not a Rust state-model harness (the old generic fall-through emitted
   meaningless Anchor-shaped Kani for sBPF — a latent bug now fixed). So
   `codegen --kani` / `--proptest` / `--all` skip assembly targets with a note.
   `kani.rs` never had sBPF logic, so nothing was ported there.

3. **Sidecar extraction.** Lifted `write_spec_with_sidecars` + its closure
   (interface-pin import injection, sibling `<Iface>.lean` axiom modules,
   lakefile `roots`/`require` wiring) out of `lean_gen.rs` into the new
   renderer-agnostic `lean_sidecars.rs`, so it outlived `lean_gen.rs`'s deletion.

4. **Deleted `lean_gen.rs` + `kani.rs` (~11K LoC)** and dropped the
   `QEDGEN_LEGACY_LEAN` / `QEDGEN_LEGACY_KANI` env-var reads. `lean_gen_mir` is
   the sole Lean path; `kani_mir` the sole Kani path. Repointed the remaining
   `lean_gen`/`kani` references (`check.rs::proof_pkg_name` → `lean_sidecars`;
   `regen_drift.rs` + `main.rs init` → the MIR generators).

5. **Docs sweep.** Updated CLAUDE.md's MIR section + module list, the
   `lean_gen_mir` / `kani_mir` / `lean_sidecars` `//!` docstrings, README, and
   `references/cli.md` to drop the deleted modules + escape-hatch language.

## Regression gates

- `tests/{mir,kani,codegen}_snapshot.rs` green — every pilot fixture
  byte-identical to checked-in references.
- The full sBPF renderer (instruction blocks / guard theorems / `ea_*` lemmas /
  completeness `structure Spec`) is gated by a golden test
  (`lean_gen_mir::tests::sbpf_render_matches_golden`, fixture
  `dropset_sbpf.qedspec`); the sibling axiom-module writer by
  `lean_sidecars::tests::axiom_module_matches_golden`. Both goldens were proven
  byte-identical to the legacy renderers before deletion.
- `scripts/check-lake-build.sh --strict` (CI, cold Mathlib) — proves the
  migrated sBPF + record proofs still build. **This is the one remaining
  release gate; byte-parity makes it a formality.**

## Out of scope (stays for v3.0)

- codegen + proptest legacy deletion — `codegen_mir::generate_guards` and the
  proptest body aren't MIR-direct yet (the typed-`Stmt` lift). The
  `QEDGEN_LEGACY_CODEGEN` / `QEDGEN_LEGACY_PROPTEST` hatches remain until then.
