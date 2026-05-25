# MIR codegen snapshots — Phase 1d

Phase 1d snapshot equivalence per `docs/design/qedgen-mir-sketch.md`.
Each `<fixture>.Spec.lean` file here is the MIR-rendered Lean output
for the corresponding pilot fixture under `examples/rust/`. The
companion `tests/mir_snapshot.rs` runs `qedgen codegen --lean` with
`QEDGEN_USE_MIR=1` against each fixture and asserts byte-equality
against the snapshot.

## Workflow

| Goal | Command |
|---|---|
| Verify no drift | `cargo test --test mir_snapshot` |
| Refresh after an intentional MIR codegen change | `UPDATE_SNAPSHOTS=1 cargo test --test mir_snapshot` then inspect + commit the diff |
| Add a new pilot fixture | Add a `snapshot_<name>` fn to `tests/mir_snapshot.rs` and run `UPDATE_SNAPSHOTS=1` once to seed |

## MIR vs legacy parity, as of commit landing this directory

The snapshots lock the MIR output, NOT the legacy output. Where the
two diverge today:

| Fixture | Path | MIR ⇆ legacy |
|---|---|---|
| `bundled-stdlib-demo` | ADT | **byte-identical** |
| `cross-program-vault` | ADT | **byte-identical** |
| `escrow-split` | ADT | identical modulo deferred §15 `cover_trace_proof` auto-discharge witnesses (~13 lines) |
| `escrow` | flat | substantial pre-existing divergence — `inductive Status` deriving order, transition body shape (no signer-equality / lifecycle gate), cover proof witnesses |
| `lending` | flat | same as escrow |
| `multisig` | flat | same as escrow |

ADT-path byte-equivalence is the v2.30 Phase 1c-8 deliverable; the
flat-path differences predate Phase 1d and are tracked as follow-on
work. See `docs/design/qedgen-mir-sketch.md` §"Deferred — return in
a dedicated Phase 1d session" for the open items, notably:

- `cover_trace_proof` / `liveness_proof_script` / `overflow_proof_script`
  / `preservation_proof_script` auto-discharge ports (§15 + §11) —
  closes the ~13 lines in `escrow-split` and reduces flat-path diff
  modestly.
- Flat-state transition body / `inductive Status` / abort proof shape
  alignment — closes the bulk of the flat-path diff.

Tracked in `MEMORY.md` as
[[project-v230-mir-byte-equivalence]] when it lands as a follow-up.
