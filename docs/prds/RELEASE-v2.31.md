# QEDGen v2.31 — Multi-Target Greenfield Codegen (Quasar + Pinocchio)

**Date:** 2026-05-28

## Headline

**`qedgen init` / `qedgen codegen` now scaffold three framework targets,
not one.** Alongside Anchor, the `--target` flag accepts:

- **`quasar`** — Blueshift `quasar_lang`: `#![no_std]`, `Ctx<X>`, explicit
  instruction discriminators, `quasar-pod` zero-copy state.
- **`pinocchio`** — a complete MIR-native `#![no_std]` scaffold:
  `entrypoint!` + 1-byte-discriminant dispatch, `zeropod` zero-copy state,
  `&AccountInfo` account structs with `.handler()` methods, signer +
  `requires` guards, a per-program error enum, checked scalar effects, SPL
  Token CPIs, and plain-struct events.

A trivial spec or an SPL-transfer spec (`call Token.transfer(...)`) now
`cargo build`s end-to-end for `--target pinocchio` from the CLI — locked in
by a regenerate-from-spec compile gate so it can't silently drift.

Per-target CPI dispatch is the connective tissue: a single `try_emit_cpi`
two-axis match (`target` × `is_spl_token`) routes SPL Token calls to the
right shape per framework — `anchor_spl::token::*` (Anchor),
`quasar_spl::TokenCpi` method chain (Quasar), or
`pinocchio_token::instructions::*` struct + `.invoke()` (Pinocchio).

## What's in

| Item | Status |
|---|---|
| Per-target CPI dispatch (`try_emit_cpi` — `target` × `is_spl_token`) | shipped |
| Quasar SPL Token CPI — transfer / mint_to / burn / close | shipped |
| Pinocchio SPL Token CPI — transfer / mint_to / burn / initialize_account / close | shipped |
| Quasar impl-targeted Kani harness (`emit_kani_impl_quasar`) | shipped |
| Pinocchio impl-targeted Kani harness (`emit_kani_impl_pinocchio`) | shipped — validated end-to-end (`cargo kani` catches a real token overflow in ~1s) |
| **Pinocchio greenfield scaffold** — MIR-native, all sub-generators | shipped (slice 6) |
| — `#![no_std]` lib + `entrypoint!` + byte-discriminant dispatch | shipped |
| — `zeropod` zero-copy state (scalar pods + discriminant-tag enums) | shipped |
| — `&AccountInfo` account structs + `.handler()` + `process_<name>` wrapper | shipped |
| — guards (signer + `requires`), error enum (`#[repr(u32)]` + `From<…> for ProgramError`) | shipped |
| — checked / saturating / wrapping scalar effects (`+=` / `+=!` / `+=?`) | shipped |
| — SPL Token CPIs from `call Interface.handler(...)` sites | shipped |
| — plain `#[derive(Clone)]` event structs (no `#[event]` macro) | shipped |
| `qedgen codegen --target pinocchio` emits the scaffold (not just `init`) | shipped |
| Greenfield fixture + compile gate (`vault-greenfield` + `codegen_smoke`) | shipped |

## How to use it

```bash
# Greenfield Pinocchio program from a spec:
qedgen init --name myprog --spec myprog.qedspec --target pinocchio
qedgen codegen --spec myprog.qedspec --target pinocchio   # regen scaffold

# Quasar:
qedgen init --name myprog --spec myprog.qedspec --target quasar
```

Both `init` and `codegen` route all three targets through the MIR codegen
path. Pinocchio is MIR-only (no `QEDGEN_LEGACY_CODEGEN` arm). The
verification backends (`--kani` / `--proptest` / `--lean` / `--integration`
/ `--ci`) are spec-driven and target-agnostic — they run for any target.

A `call Token.transfer(from = …, to = …, amount = …, authority = …)` site
lowers to a real `pinocchio_token::instructions::Transfer { … }.invoke()?;`
in the handler body. The `transfers { … }` sugar stays agent-fill on every
target by design (codegen owns deterministic translation; the agent owns the
CPI/authority business logic).

## Scope — deferred to a follow-on

These are additive, not blockers; the scaffold + SPL CPI milestone is
self-contained.

| Item | Reason |
|---|---|
| Generic (non-SPL) Pinocchio CPI | raw `invoke_signed` + arg serialization — own design pass (non-SPL `call` sites emit a documented breadcrumb today) |
| PDA-signed `invoke_signed` (Quasar + Pinocchio) | spec must surface seed/bump fields first |
| Imported account-type mirrors for Pinocchio | needs the `zeropod` decode shape; emits a **clean error** (not a panic) until then |
| Pinocchio `ref_impls` / unit + integration test-gen | the build-order step-5 tail |
| Pinocchio Kani-impl over custom (non-SPL) state | needs the MemoryLayout pipeline |

## Verification matrix

| Gate | Result |
|------|--------|
| `cargo test --release` | 1008 (bin) + 24 (macros) + snapshot suites — 0 failed |
| `cargo test --test codegen_snapshot` | 6/6 (Anchor/Quasar byte-unchanged) |
| `cargo test --test {mir,kani,proptest}_snapshot` | 6/6 each |
| `codegen_smoke::vault_pinocchio_scaffold_compiles` | regen-from-spec + `cargo build` clean |
| `cargo fmt --check` | clean |
| `cargo clippy --release -- -D warnings` | clean |
| `bash scripts/check-version-consistency.sh` | 2.31.0 consistent |
| `bash scripts/check-readme-drift.sh` | 19/19 commands documented |
| `bash scripts/check-lake-build.sh --strict` | 3/3 warm-cached clean; remaining 8 need CI's `.lake` cache (no Lean touched this release) |
| `qedgen check --frozen` (8 bundled qed.toml specs) | no stale locks; locks byte-identical to v2.30 |
| `old(...)` pre/post harness (issue-8 corpus) | binary `fn p(pre, post)` signature emitted |
| `cargo audit --deny warnings` (+ documented ignores) | clean |
| `cargo deny check` | advisories / bans / licenses / sources ok |

Supply-chain note: `RUSTSEC-2026-0097` (the `rand` ignore) is now flagged by
`cargo deny` as "advisory not detected" — the vulnerable crate is no longer
in the tree. Safe to prune from `deny.toml` + the CI ignore list in a later
pass.

## Cross-references

- `crates/qedgen/src/codegen.rs` — `try_emit_cpi`, `emit_spl_{anchor,quasar,pinocchio}`, `emit_pinocchio_program_lib`, `emit_pinocchio_effect_body`
- `crates/qedgen/src/codegen_mir.rs` — MIR-native Pinocchio scaffold (state / lib / instructions / errors / events)
- `examples/pinocchio-fixtures/vault-greenfield/` — greenfield spec backing the compile gate
