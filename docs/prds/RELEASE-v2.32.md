# QEDGen v2.32 — Legacy-codegen migration & deletion (plan)

**Status:** planning. **Theme:** finish the MIR migration for the spec classes
still pinned to legacy, *then* delete the Lean + Kani legacy modules.

## Why this is a migration, not a deletion

The v2.30→v2.32 sequencing assumed v2.32 could just delete `lean_gen.rs`
(~8.7K LoC) + `kani.rs` (~2.4K LoC) once the `QEDGEN_LEGACY_*` soak was clean.
It can't — the soak left those paths **load-bearing**. The dispatch in
`main.rs` forces legacy regardless of the env var when `mir_out_of_scope`:

```rust
// Lean  (main.rs)
let mir_out_of_scope = parsed.is_assembly_target() || !parsed.records.is_empty();
// Kani  (main.rs)
let mir_out_of_scope = parsed.is_assembly_target();
```

So legacy is the **only correct path** for:

| Spec class | Codegen still on legacy | Blocking gap in the MIR path |
|---|---|---|
| sBPF / `pragma sbpf` (counter, dropset, slippage, transfer, tree) | Lean **and** Kani | `lean_gen_mir`/`kani_mir` `is_sbpf(_mir)` is a Phase-0 stub — pragmas aren't lifted into MIR, so MIR emits Anchor-shaped output |
| Record-bearing (`type T { … }` — percolator) | Lean | `lean_gen_mir` never ported record `structure T` + `instance Inhabited T` emission + bare-field-assign wrapping (`{ acct with … }`) |

Deleting now would regress all five sBPF examples + percolator. Hence v2.32 is
a migration: close the gaps, prove parity, delete last.

## Workstreams (in dependency order)

1. **Lift sBPF into MIR.** Carry `pragma sbpf` / assembly-target through
   `mir::lower` so `is_sbpf(mir)` is real, not a stub. Port the sBPF Lean
   header/program-module shape (`lean_gen`'s sBPF renderer) and the sBPF Kani
   shape onto the MIR emitters. Gate: all 5 `examples/sbpf/*` regenerate
   byte-identical to their committed `Spec.lean` (and Kani harnesses).

2. **Port records into `lean_gen_mir`.** Emit `structure T` + `instance
   Inhabited T` for each `type T { … }`, plus the bare-field-assign wrapping.
   Gate: percolator regenerates byte-identical to `percolator.Spec.lean`.

3. **Drop the `mir_out_of_scope` fallback + env-var hatches.** Once 1+2 land,
   make MIR the unconditional path; remove `QEDGEN_LEGACY_LEAN` /
   `QEDGEN_LEGACY_KANI` reads.

4. **Delete `lean_gen.rs` + `kani.rs`** and any now-orphaned helpers. Update
   the snapshot suites so they assert MIR output against the checked-in
   references (they already do — the legacy-route comparison just goes away).

5. **Docs sweep.** Update CLAUDE.md's MIR section, the module `//!` docstrings,
   and references/ to drop the legacy escape-hatch language.

## Regression gates (the "no new bugs" bar)

- `tests/{lean,kani}_snapshot.rs` (or codegen_snapshot for Lean) stay green —
  every pilot fixture byte-identical to checked-in references.
- `scripts/check-lake-build.sh --strict` passes on every `examples/*/formal_verification`
  (rust + **sBPF**) — proves the migrated sBPF + record proofs still build.
- `examples/sbpf/*` + percolator regenerate with **zero diff** before the
  legacy modules are removed (parity proof), then again after (no-op proof).

## Out of scope (stays for v3.0)

- codegen + proptest legacy deletion — `codegen_mir::generate_guards` and the
  proptest body aren't MIR-direct yet (the typed-`Stmt` lift). Separate project.
