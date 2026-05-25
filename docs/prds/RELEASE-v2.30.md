# QEDGen v2.30 — MIR Carry-Through + Supply-Chain Gate

**Date:** 2026-05-25
**Sketch:** `docs/design/qedgen-mir-sketch.md`

## Headline

**Every primary codegen now consumes a typed intermediate
representation.** Lean, Kani, Anchor/Quasar, and proptest all route
through `mir::Mir` by default. Cross-codegen divergence — a new spec
feature one backend understands and the others silently ignore — is
now a compile-time obligation rather than a runtime drift. The
codegen surface isn't smaller in v2.30 (parallel `*_mir.rs` modules
sit alongside legacy modules behind escape-hatch env vars); the
divergence-prevention payoff lands immediately for any new feature
added against MIR.

v2.30 also adds a recurring supply-chain audit gate
(`cargo-audit` + `cargo-deny` on every push/PR) — formalizing the
read-only audit that ran during this release into a CI-enforced
policy.

## What's in

| Item | Status |
|---|---|
| `mir::Mir` typed IR | shipped (Phase 0) |
| Lean codegen → MIR-default | shipped (Phase 1–2; all 6 pilots byte-identical) |
| Kani codegen → MIR-default | shipped (Phase 3a–3f; all 6 pilots byte-identical) |
| Anchor/Quasar codegen → MIR-default | shipped (Phase 4a–4i; 9 of 10 sub-generators MIR-direct) |
| Proptest codegen → MIR-default | shipped (Phase 5 — pure-delegation scaffold) |
| Snapshot suites per backend | shipped (`{mir,kani,codegen,proptest}_snapshot`; 4 × 6 = 24 fixture locks) |
| Multi-account `mod <name>` wrapping (Kani) | shipped (Phase 3e — closes lending byte-equivalence) |
| Multi-account renderer (Lean) | shipped (Phase 2 — closes lending byte-equivalence) |
| Pilot-scope guards (sBPF + records → legacy) | shipped (Lean, Kani) |
| Supply-chain CI gate (`cargo-audit` + `cargo-deny`) | shipped (`deny.toml`, `.github/workflows/ci.yml`) |
| `ParsedSpec: Clone` (for multi-account scope-builder) | shipped (Phase 2) |

Two items stay deferred to v3.0:

| Item | Reason |
|---|---|
| `codegen::generate_guards` MIR-direct port (636L) | Per-handler `requires` / `effects` / `auth` / `status` emission is deeply coupled to `ParsedHandler` fields with no clean structural seam. Needs lifting requires + effects into typed `Stmt` nodes first. |
| `proptest_gen` MIR-direct sub-emitter ports (2,110L body) | Same deferral as guards. The MIR scaffold delegates the full emit to legacy; sub-emitters carry per-handler `arb_state` / preservation / invariant / guard / overflow / sequence walks. |

## How to use it

Transparent — no flag to enable. `qedgen codegen` routes every
backend through MIR by default. The escape hatches exist for
contingency:

```bash
QEDGEN_LEGACY_LEAN=1     qedgen codegen --spec my.qedspec --lean
QEDGEN_LEGACY_KANI=1     qedgen codegen --spec my.qedspec --kani
QEDGEN_LEGACY_CODEGEN=1  qedgen codegen --spec my.qedspec  # --target anchor / quasar
QEDGEN_LEGACY_PROPTEST=1 qedgen codegen --spec my.qedspec --proptest
```

Two carve-outs that always route to legacy regardless of flag:
- **sBPF specs** (`pragma sbpf`) for Lean + Kani — MIR doesn't lift
  pragmas yet (Phase-0 scaffold).
- **Record-bearing specs** (`type T { … }`) for Lean — Lean MIR's
  indexed-state path doesn't emit `structure T` + `instance :
  Inhabited T`. Other three backends route normally.

## Removal roadmap

The escape hatches are temporary. They prove the MIR side safe in
production over one minor cycle; if no `QEDGEN_LEGACY_*` workaround
reports come in:

- **v2.31** — soak. No code change unless escape-hatch reports
  surface a bug.
- **v2.32** — delete `lean_gen.rs` (8,677L) + `kani.rs` (2,385L) and
  the corresponding env vars. ~11K LoC removed.
- **v3.0** — port `generate_guards` + the proptest body to
  MIR-direct (above deferrals), then delete `codegen.rs` (7,572L) +
  `proptest_gen.rs` (2,110L) and the remaining two env vars. ~10K
  more LoC removed.

If you hit an issue that needs an escape hatch, file at
https://github.com/QEDGen/solana-skills/issues so we can confirm
the soak gate before deletion.

## Snapshot gates (CI)

`cargo test --test mir_snapshot` (Lean) ·
`cargo test --test kani_snapshot` ·
`cargo test --test codegen_snapshot` ·
`cargo test --test proptest_snapshot` — every backend gates 6 pilot
fixtures against checked-in references. Any unintended drift fails
the gate immediately. Refresh after intentional codegen changes
with `UPDATE_SNAPSHOTS=1 cargo test --test <suite>`.

## Supply-chain gate (CI)

New `supply-chain` job in `.github/workflows/ci.yml` runs:
```bash
cargo audit --deny warnings --ignore RUSTSEC-2024-0436 --ignore RUSTSEC-2024-0388
cargo deny check
```

Policy in `deny.toml`: zero unignored RustSec vulnerabilities,
permissive license allowlist (MIT / Apache-2.0 / BSD / ISC), only
`crates.io` as a dep source (no git-branch pins). Two RUSTSEC IDs
are explicitly ignored — both are upstream "unmaintained" tags on
transitive Solana SDK / Arkworks deps, not exploitable.

Install locally:
```bash
cargo install --locked cargo-audit cargo-deny
```

## Verification matrix

| Gate | Result |
|------|--------|
| `cargo test --bins` | 990 passing |
| `cargo test --test mir_snapshot` | 6/6 |
| `cargo test --test kani_snapshot` | 6/6 |
| `cargo test --test codegen_snapshot` | 6/6 |
| `cargo test --test proptest_snapshot` | 6/6 |
| `cargo fmt --check` | clean |
| `cargo clippy --lib --bins -- -D warnings` | clean |
| `bash scripts/check-lake-build.sh` | 3/3 warm-cached |
| `cargo build --workspace --release` | clean |
| MIR-default ≡ forced-legacy on 7 pilots (codegen) | byte-identical |
| Supply-chain audit (372 unique crates) | clean — zero CRIT/HIGH/MED/LOW |

## Cross-references

- `docs/design/qedgen-mir-sketch.md` — design sketch with phase-by-phase deliverables
- `deny.toml` — supply-chain policy
- `crates/qedgen/src/{lean_gen_mir,kani_mir,codegen_mir,proptest_gen_mir}.rs` — the four MIR-consuming codegens
- `crates/qedgen/src/mir.rs` — typed IR
