# vault-greenfield

A **greenfield** Pinocchio spec — the source for the Pinocchio scaffold
compile gate. Unlike the sibling audit fixtures (`ptoken-transfer/`,
`pata-create/`, …), this is NOT a hand-authored program with an
`expected_findings.json` golden. It is a `.qedspec` that `qedgen codegen
--target pinocchio` turns into a buildable crate.

It exercises the full slice-6 scaffold surface in one spec:

- zeropod zero-copy state (`State | Active of { total : U64 }`)
- signer + `requires` guards
- a per-program error enum (`InvalidAmount`, `MathOverflow`, `MathUnderflow`)
- checked scalar effects (`total += amount` / `total -= amount` → `checked_add`
  / `checked_sub` with the declared error variants)
- an SPL Token CPI via `call Token.transfer(...)` → `pinocchio_token::
  instructions::Transfer { … }.invoke()?;` (slice 2b)

## Gate

`crates/qedgen/tests/codegen_smoke.rs::vault_pinocchio_scaffold_compiles`
regenerates the scaffold from this spec into a tempdir and `cargo build`s
it. Regenerating from the spec (rather than committing a static tree)
means the gate tests *current* codegen output, so it can't silently drift.
CI runs it via `cargo test --test codegen_smoke -- --ignored`.

```sh
qedgen init --name vault --spec vault.qedspec --target pinocchio
cargo build --manifest-path programs/vault/Cargo.toml
```
