# config-pubkey — Pinocchio `Pubkey`-state regression fixture

Regression fixture for [issue #71](https://github.com/QEDGen/solana-skills/issues/71).

`Pubkey` state fields lower to a raw `[u8; 32]` in the zeropod struct (no Pod
scalar wrapper), so the Pinocchio scaffold must treat them differently from
scalar fields:

- **read by value** — no `.get()` (that method only exists on Pod wrappers),
- **compare against a deref'd account key** — `*ctx.<acct>.key() == __state.<f>`,
- **assign via the deref'd value** in effects — `__state.<f> = *self.<acct>.key();`
  (no `.into()`), and
- **bind account refs through `self`** in the effect body (the handler method),
  not the guard fn's `ctx`.

Before the fix, this spec passed `qedgen check` + proptest but the generated
crate failed `cargo build` (`.get()` on `[u8; 32]`, `&Pubkey` vs `[u8; 32]`
mismatch, `ctx` unresolved in the `self`-bound handler method).

The scalar `nonce` field is included so the gate also confirms scalars still
read through `.get()`.

Exercised by `config_pubkey_pinocchio_scaffold_compiles` in
`crates/qedgen/tests/codegen_smoke.rs` (regenerates from this spec and runs
`cargo build`).
