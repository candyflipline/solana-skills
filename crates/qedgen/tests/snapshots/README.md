# MIR codegen snapshots

Per `docs/design/qedgen-mir-sketch.md`. Each `<fixture>.Spec.lean`
file here is the MIR-rendered Lean output for the corresponding
pilot fixture under `examples/rust/`. The companion
`tests/mir_snapshot.rs` runs `qedgen codegen --lean` against each
fixture (MIR is the default Lean codegen path post v2.30 Phase 2)
and asserts byte-equality against the snapshot.

## Workflow

| Goal | Command |
|---|---|
| Verify no drift | `cargo test --test mir_snapshot` |
| Refresh after an intentional MIR codegen change | `UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot` then inspect + commit the diff |
| Add a new pilot fixture | Add a `snapshot_<name>` fn to `tests/mir_snapshot.rs` and run `UPDATE_SNAPSHOTS=1` once to seed |

## MIR ↔ legacy parity

Every pilot fixture is byte-identical across MIR and the legacy
`lean_gen.rs` (verified against fresh-legacy regen at the listed
phase):

| Fixture | Path | Phase reached parity |
|---|---|---|
| `bundled-stdlib-demo` | ADT | Phase 1c-8 |
| `cross-program-vault` | ADT | Phase 1c-8 |
| `escrow-split` | ADT + §15 cover-trace | Phase 1c-9 |
| `escrow` | flat single-account | Phase 1c-10 |
| `multisig` | indexed (`Map[N] T`) | Phase 1e |
| `lending` | multi-account (`PoolState` + `LoanState`) | Phase 2 |

The snapshots lock the MIR output (not the legacy output), so a
failing snapshot signals "MIR changed" rather than "MIR drifted
from legacy" — but they're the same shape today.
