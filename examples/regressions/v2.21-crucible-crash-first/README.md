# v2.21 Slice 1 + §S1.2 — Crucible brownfield regression fixture

This fixture pins two v2.21 PRD exit criteria: the §"Slice 1" crash-
first bear-hug (intrinsic panic / unwrap-on-None detection on a
brownfield Anchor program without a `.qedspec`) AND the §S1.2 lamport-
conservation companion (protocol-invariant guard around every
`.send()`).

## What's in `buggy_anchor/`

A minimal Anchor program with three deliberate bugs:

1. **`run` handler** — divides a constant by zero, panicking on the
   first invocation. Demonstrates Crucible's intrinsic panic detector
   firing on a brownfield protocol-mode harness with an empty
   `invariant_test()` body.

2. **`maybe` handler** — calls `.unwrap()` on a `None`-yielding helper.
   Demonstrates the same intrinsic detector via the panic that
   `Option::unwrap` raises on `None`.

3. **`drain` handler** — sweeps all `source` lamports into `target`
   with no authority check. When the fuzzer happens to pick a tracked
   signer pubkey for `target`, qedgen's v2.21 §S1.2 lamport-inflation
   guard (`assert_no_signer_inflation`) fires. Surfaces the drain
   shape without any spec annotation.

`run` and `maybe` are reachable on **every** invocation — a working
Crucible run finds them on iteration 1. `drain` requires the fuzzer to
land on a `target` that overlaps the tracked signer set; coverage-
guided search reaches that case within a typical budget.

## Running the harness

Out of the box this fixture is **not** part of the cargo workspace —
the `Cargo.toml` would otherwise drag `anchor-lang` and the Solana
toolchain into every `cargo build` of the qedgen workspace. To run
the live demo:

```bash
# 1. Copy the fixture out of the repo so it's a standalone crate.
cp -r examples/regressions/v2.21-crucible-crash-first/buggy_anchor /tmp/

# 2. Generate the brownfield Crucible harness (no .qedspec).
qedgen probe --fuzz 0 --root /tmp/buggy_anchor

# 3. Confirm the harness was emitted with the PROTOCOL banner.
head -20 /tmp/buggy_anchor/.qed/fuzz/buggy_anchor/src/main.rs

# 4. Build the program (Anchor) so target/idl/buggy_anchor.json exists,
#    then run the fuzz with a 60-second budget. qedgen symlinks the
#    IDL into the harness automatically.
cd /tmp/buggy_anchor && anchor build
qedgen probe --fuzz 60 --root /tmp/buggy_anchor
```

Expected output: at least one `Finding` with `category_tag =
"runtime_panic"` (action `run` divide-by-zero) and ideally a second
with `category_tag = "runtime_abort"` or `"runtime_panic"` for `maybe`.

## What this fixture validates

- **CLI gate lift** — `qedgen probe --fuzz 0 --root <path>` exits 0
  without a `.qedspec`.
- **Brownfield handler discovery** — both `run` and `maybe` appear as
  `action_*` stubs in the emitted harness.
- **Protocol-mode header** — the emitted `main.rs` carries the
  `Mode: PROTOCOL (no spec)` banner.
- **Empty `invariant_test()` body** — no spec-derived `fuzz_assert!`
  calls; crashes fire only through Crucible's intrinsic detector.
- **`.qed/fuzz/<prog>/` location** — the emitted harness lives under
  the user's `.qed/` ephemeral namespace, not in the program crate's
  `src/`.

The first four are covered by the unit + integration tests in
`crates/qedgen/tests/crucible_brownfield_smoke.rs`. The fifth — the
live fuzz finding the bug — needs Crucible on PATH; this README
documents the manual run.
