# PR: Strengthen Pinocchio Kani Coverage for Account Resources and Token Balances

## Summary

This branch extends the QEDGen Kani backend so it can prove more of the Loyal Hub Pinocchio program from generated artifacts. The branch currently contains three commits beyond upstream `main`:

```text
20634de fix(qedgen): add Pinocchio token balance Kani proofs
f3d2afc fix(qedgen): split large Kani guard rejection proofs
fce7f66 fix(kani): support account pubkeys and unbound CPI effects
```

Together these changes move QEDGen from a useful model-level Kani check toward implementation-targeted proofs for Loyal Hub token movement. The model backend remains available through `--kani`. The implementation-targeted backend is explicit through `--kani-impl` or `--all`.

## Motivation

Loyal Hub uses Pinocchio, QEDGen specs, SPL Token CPIs, and account identity checks in the same behavioral surface. The existing Kani model backend could check that the spec transition model preserved its own guarantees, but that was not enough to prove the compiled program path that dispatches real instruction data through `process_instruction`.

Two concrete gaps showed up during downstream Loyal validation. First, some guards depend on account pubkeys and CPI account resources, while earlier generated Kani code mostly treated state values as the proof environment. Second, handlers such as `swap_exact_in`, `withdraw_inventory`, and `rebalance_inventory` move SPL Token balances, so the implementation proof needs to inspect concrete token account bytes before and after the real dispatcher runs.

Large guard sets also became expensive for Kani. The rebalance shape creates many guard terms, and proving all rejection behavior in one proof made local verification slow and hard to debug.

## What Changed

The first commit, `fce7f66`, adds account-shaped Kani inputs and CPI resource binding support. Generated proofs can now include account environments when guard logic reads account pubkeys. Account pubkey comparisons lower to byte-array comparisons, and CPI account bindings are carried into the generated Kani and Lean artifacts. The branch adds the `examples/rust/kani-cpi-account-bindings` fixture plus snapshots so this behavior is covered by repository tests.

The second commit, `f3d2afc`, changes large guard rejection generation. Small handlers still get the compact proof shape. Once a handler crosses the split threshold, QEDGen emits one proof per guard term and assumes earlier terms hold when checking the first failing term. That keeps the proof obligation smaller and gives failures names that point at the specific rejected guard.

The third commit, `20634de`, adds the Loyal Hub Pinocchio token-balance proof path. In `crates/qedgen/src/kani_impl.rs`, QEDGen now emits implementation-targeted proofs for this Pinocchio runtime. The generated proof code builds stack-resident Pinocchio `AccountInfo` values, creates SPL Token account byte buffers, packs Loyal Hub instruction data, calls `crate::process_instruction(&program_id, accounts_slice, &instruction_data)`, snapshots token-account `amount` fields before dispatch, and asserts the expected source and destination balance deltas after successful execution.

That proof path covers `swap_exact_in`, `withdraw_inventory`, and each arity-specialized `rebalance_inventory` generated from the downstream spec. It also emits implementation proofs for config mutation handlers such as `initialize_config`, `set_max_fee`, and `set_paused`. The token assertions assume the source balance is sufficient and the destination addition cannot overflow, then prove the success-path effects against concrete SPL Token account data.

The same commit also keeps the tool behavior clear. Plain `codegen --kani` remains model-only. Implementation-targeted Kani generation requires `--kani-impl` or `--all`. Generated Pinocchio crates receive the needed `#[cfg(kani)] extern crate kani;` and `#[cfg(kani)] mod kani_impl;` wiring only for Kani builds.

Several probe and lint refinements support this proof path. The Pinocchio probe now ignores `#[cfg(kani)]` proof-only code, including whole inner `#![cfg(kani)]` files, so generated proof modules do not look like production handlers. The paired-validator probe suppresses false positives for membership checks and base-guard-plus-refinement count-domain patterns. The `multi_cpi_same_field` lint now allows disjoint `Token.transfer` resources while continuing to warn on repeated or overlapping resources.

For local iteration, `kani_mir.rs` now prints progress while generating fresh Kani model files. It also supports `QEDGEN_KANI_SKIP_GUARD_PROOFS=1` for smoke runs. That environment flag skips only the guard rejection proof section during generation; full generation still emits those proofs.

## Correctness Argument

The implementation-targeted token proofs execute the program dispatcher rather than a duplicate transition function. The generated code constructs the account array and instruction data, calls the real Pinocchio entrypoint, and inspects the same byte buffers that the SPL Token account representation uses. On success, it checks that the source token account amount decreased by the transferred amount and the destination token account amount increased by that amount.

The Loyal Hub-specific generation is scoped by `spec.program_name == "LoyalHubSwap"`. The current code is intentionally tailored to the downstream Pinocchio ABI and token-account layout instead of pretending to be a generic implementation proof for every runtime. That keeps the new proof surface reviewable and avoids broad assumptions about unrelated programs.

The multi-CPI lint change is narrow. It suppresses the warning when two `Token.transfer` effects touch distinct resource expressions, which is the expected rebalance shape. The warning still fires when multiple transfers touch the same resource expression and could mask an under-specified aggregate effect.

The smoke flag is also narrow. It is useful when checking code generation wiring quickly, but it is opt-in and does not change full proof generation. Downstream Loyal validation still ran the full implementation-targeted Kani proof set after the faster smoke checks.

## Validation

The fork was validated locally from `/private/tmp/solana-skills` with the following repository checks:

```sh
cargo build -p qedgen-solana-skills

for filter in kani_impl cfg_kani paired_validator multi_cpi_same_field render_skip_guard_proofs_still_emits_effect_proofs; do
  cargo test -p qedgen-solana-skills "$filter" -- --nocapture
done

git diff --check upstream/main..HEAD
```

All of those checks passed. The filtered tests cover the new implementation-targeted proof generator, `#[cfg(kani)]` probe filtering, paired-validator probe refinements, multi-CPI lint behavior, and the smoke-generation skip path.

The same fork was then used against the downstream Loyal repo. The default QEDGen gate passed with the fork while keeping Kani outside the default gate. The dedicated model-level Kani gate passed in smoke mode. The implementation-targeted Kani smoke gate passed, and the full implementation-targeted Loyal Hub Kani run passed all generated proofs. A direct Loyal Hub Kani implementation script also passed its generated proof set.

## Reviewer Notes

This PR does not claim complete formal verification for all Solana token semantics. It proves the generated Loyal Hub Pinocchio implementation obligations currently modeled by QEDGen, including concrete SPL Token account amount deltas for the token-moving handlers. Future work should generalize the Pinocchio token proof generator beyond Loyal Hub, add richer token resource projection in the spec model, and decide when implementation-targeted Kani should become a standard CI job rather than an explicit local or manual gate.
