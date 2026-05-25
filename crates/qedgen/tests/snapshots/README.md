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
| `escrow-split` | ADT | byte-identical (vs fresh-legacy regen) after §15 `cover_trace_proof` port |
| `escrow` | flat | **byte-identical** (vs fresh-legacy regen) after Phase 1c-10 flat-path alignment |
| `lending` | flat | divergent — multi-account codegen (`PoolState` + `LoanState` vs single `State`); Phase 2 |
| `multisig` | flat | divergent — indexed-state imports + `Map[N] T` compound-type lowering; separate work |

ADT-path byte-equivalence was the v2.30 Phase 1c-8 deliverable;
escrow flat-path byte-equivalence was the v2.30 Phase 1c-10
deliverable (Status/State deriving order, signer-equality + lifecycle
gate conjuncts in transition bodies, requires-based abort
auto-proof, liveness path-finding + auto-proof script). Lending and
multisig still diverge for unrelated structural reasons
(multi-account split; indexed-state lowering) tracked separately.

Remaining `qedgen-mir-sketch.md` deferred items:

- §11 `overflow_proof_script` / §15 `preservation_proof_script`
  ports — pattern-match on the flat transition body's `split` /
  `cases` structure that Phase 1c-10 unblocked. Next-session scope.
- Multi-account `render_multi_account` (Phase 2) — required to close
  the lending diff.
- Indexed-state `Map[N] T` + `IndexedState` import emission —
  required to close the multisig diff.
