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
Quasar and Pinocchio, plus the full Pinocchio impl-targeted Kani harness
codegen (validated end-to-end — catches real overflow bugs in 1.38s). Nothing
is merged; it all lives on `feat/2.30`.

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
```

## Slice status (from quasar-cpi-spike.md §8 roadmap)

| Slice | Status | Notes |
|-------|--------|-------|
| CPI dispatch refactor (`try_emit_cpi`) | ✅ done | `codegen.rs` — two-axis `(target, is_spl_token)` match |
| Quasar SPL `transfer` | ✅ done | `emit_spl_token_cpi_quasar` / `emit_spl_quasar` |
| Pinocchio SPL `transfer` | ✅ done | `emit_spl_token_cpi_pinocchio` / `emit_spl_pinocchio` (dead code from CLI until scaffold slice 6) |
| Pinocchio Kani-impl M1/M2/M3 | ✅ done | `emit_kani_impl_pinocchio` in `kani_impl.rs`; validated end-to-end |
| **Quasar SPL mint_to/burn/init/close** (slice 2) | ⬜ next | mechanical adds to `emit_spl_token_cpi_quasar` match arm |
| **Pinocchio SPL mint_to/burn/init/close** (slice 2b) | ⬜ next | mechanical adds to `emit_spl_token_cpi_pinocchio` match arm; field-name divergence (`MintTo.account` not `to`, `MintTo.mint_authority` not `authority`) |
| Quasar generic (non-SPL) CPI (slice 3) | ⬜ | needs `BufCpiCall` (variable-len Borsh) — own design pass |
| Quasar/Pinocchio PDA-signed CPI (slice 4) | ⬜ | `invoke_signed` w/ seeds; spec must surface seed/bump fields |
| **Quasar Kani-impl** (slice 5) | ⬜ | `Ctx<X>` shape per §7b; mirror the Pinocchio M3 dispatch |
| Pinocchio scaffold (slice 6) | ⬜ big | `#![no_std]` lib + entrypoint; unblocks Pinocchio CPI from the CLI + lets us auto-inject the kani lib.rs lines |
| Pinocchio generic CPI (slice 7) | ⬜ | raw `pinocchio::cpi::invoke_signed` + Borsh |
| Pinocchio Kani-impl custom state (slice 8b) | ⬜ | non-SPL accounts need the MemoryLayout pipeline |

Recommended next: **slice 2 + 2b** (cheapest, rounds out SPL coverage) OR
**slice 5** (Quasar Kani-impl — highest brownfield value, mirrors the Pinocchio
M3 shape we just built).

## Critical gotchas (these cost real time — don't re-discover)

1. **Kani only scans the lib, NOT `tests/*.rs`.** Impl-targeted harnesses must
   live in `src/` (gated by `#[cfg(kani)]`). `redirect_kani_impl_to_src` in
   `main.rs` handles the path rewrite for Pinocchio.

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

6. **Pinocchio CPI emitter is dead code from the CLI today** — `--target
   pinocchio` skips the Rust scaffold (`main.rs:3132`), so `try_emit_cpi`'s
   Pinocchio arm is never reached. It's unit-tested directly. Lights up when
   slice 6 lands. (The Kani-impl path IS reachable via `--kani-impl`.)

## Key files

- `crates/qedgen/src/codegen.rs` — `try_emit_cpi` dispatch (~line 2052) +
  `emit_spl_*_{anchor,quasar,pinocchio}` emitters. `to_snake_case` now
  `pub(crate)`.
- `crates/qedgen/src/kani_impl.rs` — `generate_from_spec` target dispatch
  (~line 196), `emit_kani_impl_anchor` (extracted), `emit_kani_impl_pinocchio`
  + `PINOCCHIO_SCAFFOLD` const + `emit_pinocchio_handler_harness`.
- `crates/qedgen/src/main.rs` — `redirect_kani_impl_to_src` (~line 1110) +
  codegen dispatch (~line 3210).
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
```

## When resuming

1. `git checkout feat/2.30` (it's the current branch).
2. Skim `quasar-cpi-spike.md` §4 (dispatch), §7 (Kani target-correctness),
   §11 (Pinocchio Kani detail) — that's the load-bearing context.
3. Pick a slice from the table above. Slice 2/2b are warm-up; slice 5 is the
   high-value next step and reuses the M3 dispatch pattern almost verbatim.
4. Nothing is committed to `main` and no PR is open — the branch is a clean
   staging ground.
