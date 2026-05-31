# QEDGen v2.32 — (draft / in progress)

**Date:** TBD
**Status:** accumulating — not yet tagged.

## Theme

Per the v2.30→v2.32 sequencing, v2.32 is the **legacy-codegen deletion**
release: make MIR the sole path and remove the env-var-gated legacy
modules where the MIR port is complete. Plus the usual stream of
real-world codegen fixes.

## What's landed so far

| Item | Source | Status |
|---|---|---|
| Narrow `let X = mul_div_*(...)` to u64 at the binding site | PR #68 (@0xharp) | merged |
| Pinocchio: `Pubkey` state fields read by value (no `.get()`), guards compare against deref'd `*ctx.<acct>.key()` | issue #71 | fixed |
| Pinocchio: effect body binds account refs through `self` (handler method), not the guard fn's `ctx`; `Pubkey` set assigns the deref'd value (no `.into()`) | issue #71 | fixed |
| `qedgen setup`: embed `CommandBuilders.lean` (the embedded `Spec.lean` imports it) so the validation workspace `lake build`s | issue #71 | fixed |
| Kani impl-targeted harness: integer handler-param PDA seeds serialize via `to_le_bytes()` (not `u64::as_ref()`) | issue #71 | fixed |
| Regression tests: Pinocchio Pubkey guard/effect shape, integer-param seed, embedded-module import-closure | issue #71 | added |
| `config-pubkey` Pinocchio compile-gate fixture (Pubkey state + account-key effect/guard) — verified `cargo build`s | issue #71 | added |

## Known follow-ups surfaced by #71 (not yet done)

- ~~No Pinocchio fixture exercises `Pubkey` state.~~ **Done** — added the
  `config-pubkey` fixture + `config_pubkey_pinocchio_scaffold_compiles`
  gate. (The compile gates remain `#[ignore]`d / on-demand, matching the
  Anchor gates — they need the framework toolchain to build.)
- **State-account resolution** can't resolve a PDA-named state account
  (e.g. `hub_authority` holding `HubAccount`); the init effect bails with
  a `TODO` breadcrumb instead of emitting the writes.
- **Bundled examples need a support-dir regen** — `bundled-stdlib-demo`'s
  `lean_solana/QEDGen/Solana/` is missing `CommandBuilders.lean` (latent
  until something imports `QEDGen.Solana.Spec`). Re-run codegen on the
  bundled examples and `bash scripts/check-lake-build.sh --strict`.
- **Quasar/Anchor R28 PDA-seed check** (`codegen.rs`) has the same
  param-vs-state-field seed ambiguity as the Kani path — audit whether a
  handler-param seed is mis-rendered as `ctx.<acct>.<seed>.as_ref()`.

## Legacy deletion (the v2.32 headline — scope TBD)

Per CLAUDE.md / [[project_v229_v230_sequencing]]: v2.32 deletes the Lean +
Kani legacy paths (~11K LoC) once their `QEDGEN_LEGACY_*` soak is clean.
The codegen + proptest legacy stays until v3.0 because `generate_guards`
and the proptest body aren't yet MIR-direct. Final scope pending owner
decision.
