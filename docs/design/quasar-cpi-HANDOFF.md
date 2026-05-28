# Multi-target codegen — session handoff

**Branch**: `feat/2.30` (8 commits past `main`, working tree clean)
**Companion**: `quasar-cpi-spike.md` (full design + rationale; this file is
the resume-here punch list, will go stale — trust the code + git log over
this if they disagree)
**Last session**: 2026-05-28

---

## TL;DR

Extending `qedgen` codegen from Anchor-only to first-class Quasar + Pinocchio,
one slice at a time. So far: per-target CPI dispatch + SPL `transfer` for both
Quasar and Pinocchio, the full Pinocchio impl-targeted Kani harness codegen
(validated end-to-end — catches real overflow bugs in 1.38s), AND the Quasar
impl-targeted Kani harness (slice 5 — struct-based, reuses the Anchor symbolic-
accounts + per-handler emitters since the Quasar scaffold emits the same
`impl <Pascal> { fn handler(&mut self, …) }` method shape). Nothing is merged;
it all lives on `feat/2.30`.

## Commits on the branch (oldest → newest)

```
43a532d  fix: gate Anchor-shaped CPI + Kani-impl on Target::Anchor   (the 2 bugs that started this)
70d20dd  docs: quasar-cpi-spike — per-target CPI dispatch + Kani plan
d56a2ad  feat(quasar): SPL transfer CPI via TokenCpi trait
a0776d5  feat(pinocchio): SPL transfer CPI via struct + .invoke()
daf1b31  docs: §11 Pinocchio Kani-impl detailed design
d68fbff  feat(pinocchio): M1 — cargo kani works against no_std lib
9b29950  feat(pinocchio): M2 — Kani catches token overflow (hand-written)
a7f7374  feat(pinocchio): M3 — codegen the impl-targeted Kani harness
8d33e97  docs: session handoff for the multi-target codegen work
(WIP)    feat(quasar): slice 5 — impl-targeted Kani harness (emit_kani_impl_quasar)
```

## Slice status (from quasar-cpi-spike.md §8 roadmap)

| Slice | Status | Notes |
|-------|--------|-------|
| CPI dispatch refactor (`try_emit_cpi`) | ✅ done | `codegen.rs` — two-axis `(target, is_spl_token)` match |
| Quasar SPL `transfer` | ✅ done | `emit_spl_token_cpi_quasar` / `emit_spl_quasar` |
| Pinocchio SPL `transfer` | ✅ done | `emit_spl_token_cpi_pinocchio` / `emit_spl_pinocchio` (dead code from CLI until scaffold slice 6) |
| Pinocchio Kani-impl M1/M2/M3 | ✅ done | `emit_kani_impl_pinocchio` in `kani_impl.rs`; validated end-to-end |
| Quasar Kani-impl (slice 5) | ✅ done | `emit_kani_impl_quasar` in `kani_impl.rs`; struct-based, reuses the Anchor `emit_symbolic_accounts_module` + `emit_handler_harness` (Quasar scaffold emits the same `.handler()` method shape). CLI redirects `tests/kani_impl.rs` → `src/kani_impl.rs` for Quasar too. Unit-tested + eyeballed; NOT end-to-end `cargo kani` validated (see gotcha 7) |
| Quasar SPL mint_to/burn/close (slice 2) | ✅ done | arms added to `emit_spl_token_cpi_quasar`. `initialize_account` deliberately stays `None` — quasar-spl exposes only `initialize_account3` (owner is a raw `&Address`, no rent sysvar), which doesn't fit the positional account-view helper. Field maps verified against `quasar-spl-0.0.0/src/instructions/mod.rs` |
| Pinocchio SPL mint_to/burn/init/close (slice 2b) | ✅ done | all four arms added to `emit_spl_token_cpi_pinocchio`; field-name divergences (`MintTo.account`←`to`, `MintTo.mint_authority`, `Burn.account`←`from`, `InitializeAccount.rent_sysvar`←`rent`) verified against `pinocchio-token-0.3.0/src/instructions/`. NOTE: re-applied by hand on the correct base after the bg worktree agent landed on stale `main` (fd017a6) — see gotcha 8 |
| Quasar generic (non-SPL) CPI (slice 3) | ⬜ | needs `BufCpiCall` (variable-len Borsh) — own design pass |
| Quasar/Pinocchio PDA-signed CPI (slice 4) | ⬜ | `invoke_signed` w/ seeds; spec must surface seed/bump fields |
| Pinocchio scaffold (slice 6) | ✅ done | MIR-native scaffold (lib + entrypoint + byte-dispatch, zeropod state, `&AccountInfo` account structs + `.handler()`, guards, errors, scalar effects). Steps 1–5 + the milestone close (2026-05-28): SPL `call` CPIs wired into the handler body via `try_emit_cpi(_, Pinocchio)`, AND the `codegen` command (not just `init`) now emits the Pinocchio scaffold. A `call Token.transfer(...)` spec `cargo build`s end-to-end from the CLI. |
| Pinocchio generic CPI (slice 7) | ⬜ | raw `pinocchio::cpi::invoke_signed` + Borsh (non-SPL `call` sites emit a slice-7 breadcrumb today) |
| Pinocchio events / ref_impls / tests | ⬜ | `emit_events` + `emit_imported_mirror` still `unreachable!()` for Pinocchio (guarded by early-return; only event/import specs hit them). `transfers {}` sugar stays agent-fill on every target. |
| Pinocchio greenfield fixture + build gate | ✅ done | `examples/pinocchio-fixtures/vault-greenfield/vault.qedspec` (zeropod state + checked effects + guards + errors + SPL transfer CPI) + `codegen_smoke::vault_pinocchio_scaffold_compiles` regenerates from the spec and `cargo build`s it. The existing CI step (`cargo test --test codegen_smoke -- --ignored`) auto-runs it. |
| Pinocchio Kani-impl custom state (slice 8b) | ⬜ | non-SPL accounts need the MemoryLayout pipeline |

Recommended next: **slice 7** (generic non-SPL Pinocchio CPI — raw
`invoke_signed` + Borsh) or **slice 4** (PDA-signed `invoke_signed` with
spec-surfaced seed/bump). Pinocchio events / ref_impls / test-gen
(`emit_events` + `emit_imported_mirror` still `unreachable!()`) is the
remaining scaffold gap, only hit by event/import specs.

## Critical gotchas (these cost real time — don't re-discover)

1. **Kani only scans the lib, NOT `tests/*.rs`.** Impl-targeted harnesses must
   live in `src/` (gated by `#[cfg(kani)]`). `redirect_kani_impl_to_src` in
   `main.rs` handles the path rewrite for Pinocchio AND Quasar (slice 5).
   **Anchor still lands in `tests/`** — its tests/-placement is the pre-existing
   default and it relies on `crate::<Pascal>` (which only resolves from inside
   the lib), so the Anchor harness has the same latent discovery/resolution gap.
   Fixing Anchor is deliberately out of slice-5 scope (it'd change long-standing
   default behavior + Anchor snapshot expectations).

2. **`#![no_std]` libs need `#[cfg(kani)] extern crate kani;`** at the crate
   root or `cargo kani` errors "Failed to detect Kani functions." The two
   `src/lib.rs` lines (`extern crate kani` + `mod kani_impl`) are still
   USER-ADDED today — documented in the generated file header. Auto-injection
   waits for slice 6.

3. **Wire-format `AccountInfo` construction blows BMC budget.** The
   `_harness/account.rs` Box::leak'd-buffer + `deserialize` approach hit 270M
   SAT clauses / timed out. The working approach (M2/M3) builds `Account`-layout
   structs on the stack via a `#[repr(C)] AccountLayout` mirror +
   `transmute::<*mut AccountLayout, AccountInfo>`. ~250× faster. If you touch the
   harness shape, keep stack allocation.

4. **Kani's automatic overflow/UB checks do the verification.** The base harness
   does NOT need an explicit `ensures` assertion — just build symbolic accounts
   + call the real handler, and Kani's built-in arithmetic checks catch the bug.
   The M2/M3 harnesses caught the overflow this way before any assert ran.

5. **The `ptoken-transfer` fixture was source-rotted** (used
   `pinocchio_token::state::Account` / `load_mut` / `set_amount` — none exist in
   `pinocchio-token 0.3.0`; real API is `TokenAccount` +
   `from_account_info_unchecked`, immutable + raw-pointer mutation). Repaired in
   M2. Also bumped `pinocchio = "0.6"` → `"0.8"` to match pinocchio-token's
   transitive pin (mixed versions = "multiple different versions of crate
   `pinocchio`" errors).

6. **RESOLVED (2026-05-28): Pinocchio CPI emitter is now live from the CLI.**
   The `emit_spl_pinocchio` `&self.<acct>` shape that §12a claimed was
   "correct as-is" was WRONG: the handler struct fields are `&'a AccountInfo`,
   and `pinocchio_token`'s CPI struct fields take `&'a AccountInfo`, so the
   emitter must pass `self.<acct>` — a leading `&` yields `&&AccountInfo` and
   won't compile. Fixed in `emit_spl_pinocchio` + the 5 Pinocchio CPI unit
   tests. Both `init` and `codegen` now emit the scaffold and reach the
   emitter; verified by `cargo build` on a `call Token.transfer(...)` spec.
   (The Kani-impl path was already reachable via `--kani-impl`.)

7. **The struct-based harnesses (Anchor + Quasar) are agent-fill skeletons —
   NOT end-to-end `cargo kani` validated.** `build_<handler>() -> crate::<Pascal>`
   has a `todo!()` body: constructing a symbolic `Account<T>` / `&'info mut
   Account<T>` requires real account-data wiring the agent fills in. So the
   emitted file does NOT compile out of the box (by design). Only the Pinocchio
   harness builds accounts concretely (stack `#[repr(C)]` layout) and therefore
   reaches a real `cargo kani` run. Slice 5 delivers Quasar at *parity with
   Anchor* (validated by unit tests + an eyeballed generation: see
   `/tmp/quasar-kani-check` reproduction in the slice-5 commit msg), not at
   Pinocchio's end-to-end bar. Closing the `todo!()` for struct targets is the
   "symbolic-accounts infra" slice (§8 slice 9-adjacent), still open.
   RESOLVED (2026-05-28 spike): a Quasar crate **does** build + verify under
   `cargo kani` on the host (trivial proof `VERIFICATION:- SUCCESSFUL`, 0.52s;
   cargo-kani 0.67.0). The whole `quasar-lang`/`quasar-spl`/`quasar-derive` +
   `solana-*-view` dep graph compiles clean on the host target; no
   `extern crate kani` needed (the crate is `std` on host — its `no_std`
   `cfg_attr` only fires on `target_os = "solana"`). So the ONLY blocker to a
   green slice-5 run is the `todo!()` account-wiring above — no Quasar/Kani
   incompatibility. Worth committing a `quasar-fixtures/kani-smoke/` analogue
   to the Pinocchio one (currently a throwaway at `/tmp/quasar-kani-smoke`).

8. **`Agent` worktree isolation branches from `main`, NOT the feat/2.30 tip.**
   Observed 2026-05-28: two background worktree agents both cut from `fd017a6`
   (the main tip = "Merge PR #69"), 6 commits behind HEAD `8d33e97`. The
   slice-2+2b agent then re-implemented the per-target CPI dispatch from
   scratch (it didn't exist on the stale base) — unmergeable; only its registry
   research was salvageable. **Do branch-tip-dependent work in the foreground**,
   or have the agent verify+rebase its base first. Base-independent
   investigation agents (the kani spike) are fine to parallelize. Always
   `git merge-base <worktree-branch> HEAD` before merging agent output.

## Key files

- `crates/qedgen/src/codegen.rs` — `try_emit_cpi` dispatch (~line 2052) +
  `emit_spl_*_{anchor,quasar,pinocchio}` emitters. `to_snake_case` now
  `pub(crate)`.
- `crates/qedgen/src/kani_impl.rs` — `generate_from_spec` target dispatch
  (3-arm `match target`), `emit_kani_impl_anchor`, `emit_kani_impl_quasar`
  (slice 5 — reuses `emit_symbolic_accounts_module` + `emit_handler_harness`,
  both now take a `framework: &str` label so no "Anchor" leaks into Quasar
  output), `emit_kani_impl_pinocchio` + `PINOCCHIO_SCAFFOLD` const +
  `emit_pinocchio_handler_harness`.
- `crates/qedgen/src/main.rs` — `redirect_kani_impl_to_src` (~line 1110); the
  codegen dispatch redirects to `src/` for `Pinocchio | Quasar` (~line 3239).
- `examples/pinocchio-fixtures/kani-smoke/` — M1 smoke fixture.
- `examples/pinocchio-fixtures/ptoken-transfer/src/kani_impl.rs` — M2
  hand-written reference harness (the "what good looks like" artifact).

## How to re-validate (sanity before building on this)

```sh
# 1. Unit tests + gates
cargo test --release --bin qedgen          # 998 pass
cargo clippy --release -- -D warnings
cargo fmt --check

# 2. M1 smoke — Kani vs no_std (≈1s)
cd examples/pinocchio-fixtures/kani-smoke
cargo kani --harness smoke_kani_builds_against_no_std_pinocchio_lib

# 3. M2 reference harness — catches the overflow (≈1.1s)
cd examples/pinocchio-fixtures/ptoken-transfer
cargo kani --harness verify_transfer_preserves_token_conservation
# expect VERIFICATION:- FAILED, "attempt to add with overflow" at transfer.rs:99

# 4. M3 end-to-end (regenerate the harness from a spec, run it)
#    Spec lives nowhere committed — recreate it (see commit a7f7374 message
#    or quasar-cpi-spike.md §11f M3 for the shape), generate into a git-init'd
#    temp dir with `--target pinocchio --kani-impl`, drop the emitted
#    src/kani_impl.rs into a copy of ptoken-transfer, run cargo kani.

# 5. Quasar Kani-impl emission (slice 5 — codegen only, no cargo kani):
cargo test --release --bin qedgen kani_impl::tests::quasar_target_emits_handler_harness
#    Eyeball the real output:
TMP=$(mktemp -d); cd "$TMP" && git init -q
printf 'spec Vault\nstate { balance : U64, lp_supply : U64 }\nhandler deposit (amount : U64) {\n  accounts { authority : signer, writable\n    vault : writable }\n  requires amount > 0 else InvalidAmount\n  modifies [balance, lp_supply]\n  ensures state.balance == old(state.balance) + amount\n  effect { balance += amount }\n}\n' > vault.qedspec
qedgen init --name vault --spec vault.qedspec --target quasar
qedgen codegen --spec vault.qedspec --target quasar --kani-impl
cat programs/src/kani_impl.rs   # Quasar header, build_deposit()->crate::Deposit, accounts.handler(amount)
```

## When resuming

1. `git checkout feat/2.30` (it's the current branch).
2. Skim `quasar-cpi-spike.md` §4 (dispatch), §7 (Kani target-correctness),
   §11 (Pinocchio Kani detail) — that's the load-bearing context.
3. Pick a slice from the table above. Slice 5 (Quasar Kani-impl) is DONE.
   Slice 2/2b are warm-up SPL coverage; slice 6 (Pinocchio scaffold) is the
   big unblocker that lights up the Pinocchio CPI emitter from the CLI.
4. Nothing is committed to `main` and no PR is open — the branch is a clean
   staging ground.
