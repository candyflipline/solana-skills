# Quasar CPI emission — spike design

**Status**: proposal, awaiting review
**Author**: @abishekk92 (drafted with Claude)
**Branch**: `feat/2.30`
**Scope**: end-to-end slice that lights up `--target quasar` for SPL Token
`transfer` in `qedgen codegen`. Sets up the dispatch shape every
follow-on Quasar / Pinocchio CPI slice plugs into. **Out of scope**: other SPL
handlers (mint_to / burn / initialize_account / close_account), generic Quasar
CPI, Quasar Kani-impl, anything Pinocchio.

---

## 1. Why this slice

`feat/2.30` (commit `43a532d`) plugged the bleeding by gating
`try_emit_anchor_cpi` and `kani_impl::generate` on `Target::Anchor` — Quasar /
Pinocchio specs that hit either path now fall through to the existing
`todo!()` / no-op shape instead of emitting unbuildable `anchor_lang::*`.
That's the negative: nothing wrong is emitted. This spike does the positive:
emit something correct for Quasar.

Choosing **SPL Token `transfer`** as the spike:

- Smallest end-to-end test. Quasar already has the scaffold + state codegen +
  error dispatch wired (`Target::Quasar` is matched in ~30 places across
  `codegen.rs` and `codegen_mir.rs`). Only the CPI body is missing.
- Forces the dispatch design. Refactoring `try_emit_anchor_cpi` into a
  per-target dispatch is the surface every follow-on slice plugs into
  (other Quasar SPL handlers, generic Quasar CPI, Pinocchio CPI). Get this
  shape right once.
- SPL `transfer` is THE canonical CPI in deployed Solana programs. Whatever
  shape we land here is exercised by nearly every escrow / swap / vault.
- Has a validation path. We can re-codegen `examples/rust/escrow` with
  `--target quasar` and attempt to compile against `quasar-spl 0.0.0`.

## 2. Quasar CPI shape (from `quasar-lang-0.0.0` + `quasar-spl-0.0.0`)

Read directly from the registry source at
`~/.cargo/registry/src/index.crates.io-*/quasar-{lang,spl}-0.0.0/`.

**Core type**: `CpiCall<'a, const ACCTS: usize, const DATA: usize>` —
const-generic, stack-allocated CPI builder.

```rust
// quasar-lang::cpi::mod
pub struct CpiCall<'a, const ACCTS: usize, const DATA: usize> { … }

impl<'a, const ACCTS: usize, const DATA: usize> CpiCall<'a, ACCTS, DATA> {
    pub fn invoke(&self) -> ProgramResult { … }
    pub fn invoke_signed(&self, seeds: &[Seed]) -> ProgramResult { … }
    pub fn invoke_with_signers(&self, signers: &[Signer]) -> ProgramResult { … }
}
```

**SPL Token convenience trait**: `quasar_spl::instructions::TokenCpi`,
auto-implemented on `Program<Token>`, `Program<Token2022>`,
`TokenInterface`. The full transfer call is a one-liner:

```rust
// quasar-spl::instructions::mod (snippet)
pub trait TokenCpi: AsAccountView {
    fn transfer<'a>(
        &'a self,
        from: &'a impl AsAccountView,
        to: &'a impl AsAccountView,
        authority: &'a impl AsAccountView,
        amount: impl Into<u64>,
    ) -> CpiCall<'a, 3, 9> { … }
}
```

**Side-by-side**, same `call Token.transfer(from = src, to = dst, amount = n,
authority = auth)`:

```rust
// Anchor (what we emit today)
use anchor_spl::token::{self, Transfer};
let cpi_accounts = Transfer {
    from:      self.src.to_account_info(),
    to:        self.dst.to_account_info(),
    authority: self.auth.to_account_info(),
};
let cpi_program = self.token_program.to_account_info();
token::transfer(CpiContext::new(cpi_program, cpi_accounts), n)?;
```

```rust
// Quasar (what we'll emit after this spike)
self.token_program
    .transfer(&self.src, &self.dst, &self.auth, n)
    .invoke()?;
```

PDA-signed variant (when the authority is a PDA, follow-on slice — not in
this spike):

```rust
self.token_program
    .transfer(&self.src, &self.dst, &self.vault_authority, n)
    .invoke_signed(&[Seed::from(b"vault"), Seed::from(&[vault_bump])])?;
```

**Anchor surface**: imports `anchor_spl::token::{self, Transfer}`, constructs
account-struct + `CpiContext`, calls free fn. ~6 lines.
**Quasar surface**: trait method on `Program<Token>`, single line, no extra
imports beyond the already-emitted `quasar_spl::*`. ~3 lines.

Quasar's design simplifies emission. The dispatch is purely "which target",
not "should we use trait X or wrapper Y."

## 2b. Pinocchio CPI shape (from `pinocchio-token-0.3.0`)

Read directly from
`~/.cargo/registry/src/index.crates.io-*/pinocchio-token-0.3.0/`.

**Core shape**: `struct + .invoke()`. No trait-method sugar (unlike Quasar),
no CpiContext wrapper (unlike Anchor). Each SPL Token instruction has a
struct in `pinocchio_token::instructions::*`; the user constructs it with
references to `AccountInfo` and calls `.invoke()` (or `.invoke_signed(&[Signer])`
for PDA-signed CPIs).

```rust
// pinocchio-token::instructions::transfer (snippet)
pub struct Transfer<'a> {
    pub from: &'a AccountInfo,
    pub to: &'a AccountInfo,
    pub authority: &'a AccountInfo,
    pub amount: u64,
}

impl Transfer<'_> {
    pub fn invoke(&self) -> ProgramResult { … }
    pub fn invoke_signed(&self, signers: &[Signer]) -> ProgramResult { … }
}
```

Same `call Token.transfer(from = src, to = dst, amount = n, authority = auth)`
emits:

```rust
// Pinocchio (what we'll emit)
pinocchio_token::instructions::Transfer {
    from:      &self.src,
    to:        &self.dst,
    authority: &self.auth,
    amount:    n,
}.invoke()?;
```

**vs the other two targets** side-by-side:

| Aspect | Anchor (~6 lines) | Quasar (~1 line) | Pinocchio (~6 lines) |
|---|---|---|---|
| Shape | builder + free fn | trait method chain | struct + `.invoke()` |
| Token program ref | `self.token_program` field | `self.token_program` field | `pinocchio_token::ID` const |
| Account ref | `.to_account_info()` | `&self.<field>` | `&self.<field>` |
| Import needed | `anchor_spl::token::{self, Transfer}` | none (in prelude) | none (qualified path) |
| PDA-signed variant | `CpiContext::new_with_signer(...)` | `.invoke_signed(&[Seed::from(...)])` | `.invoke_signed(&[Signer::from(&seeds)])` |

**Note on the token program account**: Pinocchio's `Transfer` struct does NOT
take a token-program reference. The program ID is baked in via `crate::ID`
inside the `invoke_signed` implementation. So the spec's
`token_program : program` account declaration is *unused* at the CPI call
site for Pinocchio (still needed in the spec for the Anchor / Quasar
targets). This is purely a presence-not-a-bug situation; the dispatch
emitter for Pinocchio simply doesn't reference the resolved
`token_program` name.

**Mint_to and the other handlers**: pinocchio_token's field names diverge
from the canonical SPL naming for some handlers (`MintTo.account` instead
of `to`; `MintTo.mint_authority` instead of `authority`). The spike emitter
handles this via the same `(struct_field, spec_arg_name)` mapping table the
Anchor emitter uses — see §4.

## 3. Current dispatch + the gap

`crates/qedgen/src/codegen.rs:2045-2495` is the CPI surface.
Today's dispatch tree (post-commit `43a532d`):

```
try_emit_anchor_cpi(call, handler, spec, target)         // codegen.rs:2052
├── if target != Anchor → None                           // (added by 43a532d)
├── if iface.program_id == SPL_TOKEN_PROGRAM_ID
│     → emit_spl_token_cpi(call, handler, spec)          // codegen.rs:2079
│       ├── "transfer"            → emit_spl(…)
│       ├── "mint_to"             → emit_spl(…)
│       ├── "burn"                → emit_spl(…)
│       ├── "initialize_account"  → emit_spl(…)
│       └── "close_account"       → emit_spl(…)
│         └── emit_spl(…)                                // codegen.rs:2437
│             → "use anchor_spl::token::{…}"
│             → "CpiContext::new(cpi_program, cpi_accounts)"
│             → "token::<fn>(cpi, …)?;"
└── else → emit_generic_anchor_cpi(call, handler, iface, spec)  // codegen.rs:2278
            → "use anchor_lang::prelude::*"
            → "AnchorSerialize" args
            → "invoke(&Instruction { … }, &accounts)?;"
```

Every emission below the target gate is Anchor-shaped. Quasar takes the
`None` exit and gets a `// Spec call: … — needs fill` comment + `todo!()`
in the handler body — correct but no value-add.

Cargo.toml already emits `quasar-spl = { version = "0.0.0" }` when
`needs_spl` is true (`codegen.rs:5045-5053`), so the dep machinery is in
place — only the body emission is missing.

The Quasar `Accounts` struct codegen already declares `Program<Token>` /
`Account<Token>` correctly (the existing `--target quasar` scaffold uses
`map_type_quasar` at `codegen.rs:458` and the wrapper-struct logic at
`codegen.rs:152-167`). The CPI body just needs to *use* what's already
declared.

## 4. Proposed dispatch refactor

Two-axis dispatch: `(target, interface_kind)`. Today it's a one-axis switch
on interface_kind buried inside an Anchor-only function.

```rust
// codegen.rs (new shape)

/// Per-target CPI emitter. Dispatches on target first, then on the
/// callee interface kind (SPL Token canonical program-id vs generic).
fn try_emit_cpi(
    call: &ParsedCall,
    handler: &ParsedHandler,
    spec: &ParsedSpec,
    target: Target,
) -> Option<String> {
    let iface = spec.interfaces.iter().find(|i| i.name == call.target_interface)?;
    let is_spl_token = iface.program_id.as_deref() == Some(SPL_TOKEN_PROGRAM_ID);

    match (target, is_spl_token) {
        (Target::Anchor,    true)  => emit_spl_token_cpi_anchor(call, handler, spec),
        (Target::Anchor,    false) => emit_generic_cpi_anchor(call, handler, iface, spec),
        (Target::Quasar,    true)  => emit_spl_token_cpi_quasar(call, handler, spec),
        (Target::Quasar,    false) => None, // out of spike — follow-on slice
        (Target::Pinocchio, true)  => emit_spl_token_cpi_pinocchio(call, handler, spec),
        (Target::Pinocchio, false) => None, // out of spike — follow-on slice
    }
}
```

**Pinocchio emitter shape** (added in this iteration of the spike):

```rust
fn emit_spl_token_cpi_pinocchio(call, handler, spec) -> Option<String> {
    match call.target_handler.as_str() {
        "transfer" => emit_spl_pinocchio(
            call, handler, spec, "Transfer",
            // (pinocchio_struct_field, spec_arg_name)
            &[("from", "from"), ("to", "to"), ("authority", "authority")],
            Some("amount"),
        ),
        _ => None,
    }
}

fn emit_spl_pinocchio(call, handler, spec, struct_name, fields, scalar) -> Option<String> {
    // Emits:
    //   pinocchio_token::instructions::<Struct> {
    //       <field>: &self.<resolved>,
    //       …
    //       amount: n,
    //   }.invoke()?;
}
```

**Caller still skips Pinocchio scaffold today**. `main.rs:3117` bails on
`--target pinocchio` without backend flags; with backend flags it proceeds
but skips the Rust scaffold (`main.rs:3132 if !pinocchio_no_scaffold`).
That means the Pinocchio CPI emitter is **dead code from the CLI** until
slice 6 (Pinocchio scaffold) lands. We still implement + unit-test it now
because:

- The dispatch shape stays consistent. Quasar + Pinocchio land in the
  same `match (target, is_spl_token)` block; readers see them as
  parallel branches instead of a hole.
- When the scaffold lands, CPI is already there — no rebase pain.
- Unit tests calling `try_emit_cpi(_, _, _, Target::Pinocchio)` directly
  exercise the emission string, same way `kani_impl` tests exercise
  emission without invoking the full CLI.
- The committed snapshot fixture (validation layer (b)) can include the
  Pinocchio emission alongside Anchor + Quasar so future drift is
  caught structurally.

**Renames** (mechanical):
- `try_emit_anchor_cpi` → `try_emit_cpi`
- `emit_spl_token_cpi` → `emit_spl_token_cpi_anchor`
- `emit_generic_anchor_cpi` → `emit_generic_cpi_anchor`
- `emit_spl` → `emit_spl_anchor` (it's the per-handler Anchor template)

**New**:
- `emit_spl_token_cpi_quasar(call, handler, spec) -> Option<String>` —
  dispatches on `call.target_handler`. Spike implements `"transfer"` only;
  other handlers return `None` and fall through to the existing
  `todo!()` shape. Follow-on slice fills the rest.
- `emit_spl_quasar(call, handler, spec, method_name, account_args, scalar_arg)`
  — per-handler Quasar template. One-liner emission:

  ```rust
  out.push_str("        ");
  out.push_str(&format!("self.{}", token_program_name));
  out.push_str(&format!(".{}({})", method_name, args_joined));
  out.push_str(".invoke()?;\n");
  ```

  Account args are rendered as `&self.<resolved_name>`; scalar args go
  through the existing `resolve_call_arg_for_amount`.

**Why split per-target emitter rather than parametrize the existing one?**
Tried the parametrization sketch. The Anchor template emits
`account_struct + CpiContext + free_fn` (6 lines, 3 named pieces). The
Quasar template is a method-chain (1 line, 2 named pieces). The shapes
share so little structure that the parametrized version reads as `if
target == Anchor { … 30 lines … } else { … 5 lines … }` — i.e. two
functions wearing a trenchcoat. Cleaner to split.

The dispatch fn (`try_emit_cpi`) absorbs the target switch in one place
and stays under 30 lines. Snapshot tests cover both branches without
cross-target leakage.

## 5. Spike scope (what lands in the first commits)

This spike now lands across **two commits** on `feat/2.30`:

**Commit 1 (DONE — `d56a2ad`)**: Quasar SPL transfer.

1. Rename `try_emit_anchor_cpi` → `try_emit_cpi`; rename
   `emit_spl_token_cpi` → `emit_spl_token_cpi_anchor`;
   rename `emit_generic_anchor_cpi` → `emit_generic_cpi_anchor`;
   rename `emit_spl` → `emit_spl_anchor`. Mechanical, no behavior change.
2. Add `emit_spl_token_cpi_quasar` + `emit_spl_quasar` per §4. Implement
   `"transfer"` only.
3. Update `try_emit_cpi` dispatch to route Quasar SPL `transfer` to the
   new emitter. Other Quasar paths stay `None` for this spike.
4. Tests: `cpi_emits_quasar_spl_transfer` (positive) +
   `cpi_quasar_spl_mint_to_falls_through_to_none` (anti-regression for
   spike scope) + `cpi_skips_emission_for_pinocchio` (Pinocchio still
   falls through).

**Commit 2 (THIS extension)**: Pinocchio SPL transfer.

1. Add `emit_spl_token_cpi_pinocchio` + `emit_spl_pinocchio` per §2b +
   §4. Implement `"transfer"` only.
2. Update `try_emit_cpi` dispatch to route Pinocchio SPL `transfer` to
   the new emitter. Other Pinocchio paths stay `None`.
3. Replace `cpi_skips_emission_for_pinocchio` with three tests:
   `cpi_emits_pinocchio_spl_transfer` (positive),
   `cpi_pinocchio_spl_mint_to_falls_through_to_none` (anti-regression
   for spike scope), `cpi_pinocchio_non_spl_falls_through_to_none`
   (generic Pinocchio CPI still unimplemented).
4. Note: emitter is dead code from the CLI today (scaffold gate at
   `main.rs:3117`); follows the §4 rationale for landing now anyway.

Out (each is a separate follow-on slice in §8):

- Quasar SPL `mint_to`, `burn`, `initialize_account`, `close_account` —
  mechanical adds to `emit_spl_token_cpi_quasar`'s match arm.
- Pinocchio SPL `mint_to`, `burn`, `initialize_account`, `close_account` —
  mechanical adds to `emit_spl_token_cpi_pinocchio`'s match arm.
- PDA-signed CPI for Quasar / Pinocchio (`invoke_signed` with
  seeds/signers). Needs spec to surface the seed/bump fields; today's
  Anchor emitter doesn't emit signed CPI either.
- Generic (non-SPL) Quasar CPI — needs `BufCpiCall` (variable-length
  Borsh data). Distinct shape; warrants its own design pass.
- Generic (non-SPL) Pinocchio CPI — needs `pinocchio::cpi::invoke_signed`
  with raw `Instruction` + `AccountMeta` construction. Borsh-serialize
  args inline. Distinct shape.
- Quasar Kani-impl harness — needs `Ctx<X>` instead of `Context<X>` and
  Quasar's `parse_accounts` shape. Plumbing only; same dispatch pattern
  applies.
- Pinocchio Rust scaffold (slice 6) — must land before CPI is reachable
  from the CLI. Independent of the emitter.

## 6. Validation plan

Three layers, smallest-to-largest:

**(a) Inline unit tests** in `crates/qedgen/src/codegen.rs`. The two new
tests in §5. Cheap, run on every `cargo test`. Catch regressions in the
emission string itself.

**(b) Snapshot test** against a committed spec. Pick
`examples/rust/escrow/escrow.qedspec` (or whichever bundled spec has the
cleanest `call Token.transfer(…)` site). Add a snapshot test in
`crates/qedgen/tests/codegen_quasar_snapshot.rs` that emits with
`Target::Quasar` and diffs against a committed reference file. Mirrors
the existing `tests/{mir,kani,codegen,proptest}_snapshot.rs` pattern
from v2.30.

**(c) Compile validation** (the real test). Generate the full Quasar
scaffold for an escrow-shape spec to a temp dir, then run
`cargo check` against it with a `quasar-spl = { version = "0.0.0" }`
dep. If the generated code compiles, the CPI body is well-formed against
the actual API surface. Wire this as an `#[ignore]` test by default
(quasar-spl is not on CI's critical path); document in the SKILL how to
run locally.

The pre-release checklist in CLAUDE.md already includes
`scripts/check-lake-build.sh` for Lean; we'll add an analogous
`scripts/check-quasar-build.sh --strict` only when Quasar reaches parity
across enough surfaces to justify the CI cost. Not in this spike.

## 7. Kani harness target-correctness

The Kani surface has **two** harness types with different target-correctness
profiles. The spike doesn't touch either, but the design needs to be
explicit so follow-on slices don't conflate them.

### 7a. Spec-model harness (`kani.rs` / `kani_mir.rs`) — stays framework-neutral

The spec-model harness operates purely on the spec's translated transition
function. The file header at `kani.rs:72` already declares:

> These proofs verify the spec's transition design using Kani bounded model
> checking. They operate on a pure model of the state machine (derived from
> the qedspec), independent of framework (Quasar/Anchor) types.

Verified: `grep` for `anchor_lang|anchor_spl|quasar_lang|pinocchio|Context<|Ctx<`
across both files returns zero hits. The emission imports `kani` only,
defines the state struct from the spec, runs `let post = transition(pre,
args); kani::assert(ensures);`. No framework code is called or imported.

**Conclusion**: leave alone. Adding `target: Target` to `kani::generate` and
`kani_mir::generate` would be defensive plumbing for a hypothetical future
regression — per `[[feedback-cleanup-v3]]`, don't refactor when the bug
doesn't manifest. Document the invariant: "the spec-model harness must not
import or call framework code." Lint or code review enforces it. The
v2.30 MIR snapshot tests already catch any emission drift.

### 7b. Impl-targeted harness (`kani_impl.rs`) — per-target shapes required

The impl-targeted harness calls the user's REAL handler. It IS framework-
dependent by construction. Today it's Anchor-only (gated by commit
`43a532d`). Per-target shapes:

**Anchor (today)** — `crates/qedgen/src/kani_impl.rs` emission:
```rust
// Builds Anchor's Context<MyHandler>
let ctx = build_my_handler();          // returns Context<'_, MyHandler>
let result = MyHandler::handler(ctx, arg);
// post-state assertions
```

**Quasar (new — slice 5 in §8)** — emit `Ctx<MyHandler>` shape. From
`quasar-lang-0.0.0/src/context.rs:51`:
```rust
// Build symbolic Ctx<MyAccounts>
let accounts: MyAccounts = build_my_accounts();  // ParseAccounts impl
let program_id_bytes: [u8; 32] = kani::any();
let data: &[u8] = &[];                            // dispatch! consumes disc
let ctx = Ctx::<MyAccounts> {
    accounts,
    bumps: kani::any(),
    program_id: &program_id_bytes,
    data,
};
let result = my_handler(ctx, arg);                // Result<(), ProgramError>
```

Key shape differences vs Anchor: `Ctx<T>` wraps `accounts: T` + `bumps:
T::Bumps` + raw `program_id: &[u8; 32]` rather than Anchor's `Context<T>`
with `ctx.accounts.*`. Return type is `Result<(), ProgramError>` not
`Result<()>` (Anchor's alias). Pre/post snapshot reads from
`ctx.accounts.<field>` rather than `accounts.<field>`.

**Pinocchio (new — slice 8 in §8)** — no derive macros, no `Ctx` wrapper.
Pinocchio's process entrypoint takes raw slices:
```rust
// Symbolic raw accounts + data
const N: usize = 4;                               // from spec's account count
let accounts: [AccountInfo; N] = kani::any();
let data: [u8; M] = kani::any();
let program_id: Pubkey = kani::any();
let result = pinocchio_program::process_instruction(&program_id, &accounts, &data);
```

Pre/post field access requires deserializing account data (via the spec's
declared layout) rather than reaching through a wrapper. Likely needs
explicit `MemoryLayout` integration per
`[[feedback_memorylayout_sources]]`.

### 7c. Dispatch refactor for `kani_impl.rs`

When slice 5 (Quasar) lands, refactor `kani_impl::generate_from_spec` the
same way §4 refactors `try_emit_cpi`:

```rust
pub fn generate_from_spec(
    spec: &ParsedSpec,
    output_path: &Path,
    explicit_flag: bool,
    target: Target,
) -> Result<()> {
    // Existing gates (auto-trigger, ensures-empty) stay.
    match target {
        Target::Anchor    => emit_kani_impl_anchor(spec, output_path, explicit_flag),
        Target::Quasar    => emit_kani_impl_quasar(spec, output_path, explicit_flag),
        Target::Pinocchio => emit_kani_impl_pinocchio(spec, output_path, explicit_flag),
    }
}
```

The `target != Anchor → return Ok(())` short-circuit added by commit
`43a532d` becomes `target == Pinocchio → return Ok(())` once Quasar lands,
then disappears entirely once Pinocchio lands.

## 8. Extrapolation roadmap

This spike sets the shape every subsequent CPI/target slice plugs into.
The full set of slices to reach parity:

| # | Slice | Cost | Blocker on prior |
|---|-------|------|------------------|
| 1 | **DONE (`d56a2ad`)** — Quasar SPL `transfer` + dispatch refactor | shipped | — |
| 1b | **DONE (this extension)** — Pinocchio SPL `transfer` emitter (dead code until slice 6) | shipped | 1 |
| 2 | Quasar SPL `mint_to` / `burn` / `initialize_account` / `close_account` | ~half day | 1 (uses same emitter) |
| 2b | Pinocchio SPL `mint_to` / `burn` / `initialize_account` / `close_account` | ~half day | 1b |
| 3 | Quasar generic (non-SPL) CPI via `BufCpiCall` | ~1-2 days | 1 |
| 4 | Quasar PDA-signed CPI (`invoke_signed` w/ Seed) — affects both SPL and generic | ~1-2 days | 2 + 3 |
| 5 | Quasar Kani-impl harness (`Ctx<X>` shape per §7b) | ~2-3 days | 2 (gives a compilable target to verify) |
| 6 | Pinocchio scaffold (`#![no_std]`, raw account slices, no derive macros) — unblocks 1b from the CLI | ~3-5 days | — (independent) |
| 7 | Pinocchio generic (non-SPL) CPI via raw `pinocchio::cpi::invoke_signed` + Borsh | ~2-3 days | 6 |
| 8 | Pinocchio Kani-impl harness (raw `&[AccountInfo]` per §7b) | ~2-3 days | 6 + 7 |
| 9 | Per-target snapshot infra + CI gates | ~1 day | 2 + 7 (need real emission to snapshot) |

Total parity: roughly 3-4 weeks of focused work. The spike is the cheapest
slice that unblocks every later one.

## 9. Risks + open questions

- **`Program<Token>` field name**: the spike assumes the Accounts struct
  field for the token program is conventionally `token_program`. Verified
  against `examples/rust/escrow` and the Quasar scaffold output. If a spec
  names it differently, `find_token_program_account` (`codegen.rs`) already
  handles the resolution by interface-account-block matching, not by name.
  No additional logic needed; the Quasar emitter pulls the resolved name
  through the same helper.
- **`Account<Token>` vs `InterfaceAccount<Token>`**: the spike emits
  `&self.<account>` regardless of wrapper type. `AsAccountView` is
  implemented for both, so the trait method dispatch handles it. Verified
  in `quasar-spl-0.0.0/src/interface/mod.rs:28`.
- **Tests reading from cargo registry path**: validation layer (c) assumes
  `~/.cargo/registry/src/.../quasar-spl-0.0.0` exists. On a cold checkout
  cargo populates this on first build. CI handles it via the existing
  workspace build. Local devs run `cargo fetch` if missing.
- **Symbol-mangling on macOS**: per
  `[[reference_macos_linker_workaround]]`, the qedgen binary already uses
  `symbol-mangling-version = "v0"` to dodge macOS ld's symbol-length cap.
  Adding a `cargo check` step in test (c) inherits the workspace config —
  no new workaround needed.
- **Pinocchio Cargo.toml dep**: the Pinocchio CPI emitter requires
  `pinocchio-token = "0.3.0"` in the generated crate's Cargo.toml. The
  current Cargo.toml emitter (`codegen.rs:5038-5055`) `unreachable!()`s on
  `Target::Pinocchio`. Adding the dep is part of slice 6 (Pinocchio
  scaffold) — until then, the emitter writes correct CPI text but the
  surrounding crate doesn't exist.
- **Pinocchio version stability**: `pinocchio-token` is `0.3.0` (as of
  this writing). The crate is pre-1.0 and APIs can shift. Pin the exact
  version we generate against; bump it explicitly when we re-validate.
  Like the Quasar `quasar-spl = "0.0.0"` pin.

## 11. Pinocchio Kani-impl detailed design (slice 8)

The only real brownfield Pinocchio gap. This section turns §7b's sketch
into something implementable.

### 11a. What the user has, what we emit

Brownfield Pinocchio user has:
- `program/src/lib.rs` with `entrypoint!(process_instruction)`
- `program/src/<handler>.rs` per handler: `pub fn process_<name>(accounts: &[AccountInfo], data: &[u8]) -> ProgramResult`
- Account data parsed unsafely via `borrow_mut_data_unchecked()` + `bytemuck::from_bytes` or a custom `Account::load_mut(...)` helper
- A `.qedspec` next to the program (or a synthesised one from `crucible_brownfield`)

We emit (**revised by M1 findings** — see `examples/pinocchio-fixtures/kani-smoke/src/lib.rs` for the validated shape):
- `program/src/kani_impl.rs` (or per-handler `kani_impl_<name>.rs`) — a new module **inside the user's lib**, gated by `#[cfg(kani)]`. NOT a separate test crate.
- One `#[kani::proof]` per handler with at least one `ensures` clause.

The **revised** placement is forced by two findings from M1:

1. **`#![no_std]` libs need explicit `extern crate kani`**. Without
   `#[cfg(kani)] extern crate kani;` at the lib root, `cargo kani`
   fails with "Failed to detect Kani functions." Brownfield users need
   either this one-line addition (cleanest) or a peer module that
   our codegen injects.
2. **Kani only scans the lib, not `tests/*.rs`**. The original §11a
   assumption that we could ride Cargo's `tests/` → std-binary
   promotion was wrong. `cargo kani`'s default harness discovery
   walks the lib's own modules only. The harness must live inside the
   lib (gated by `cfg(kani)` so non-Kani builds skip it).

Implication: the codegen emitter writes into the user's `src/`, which
crosses a brownfield boundary we previously avoided. Options:

- **Option A** (recommended): emit a new file `src/kani_impl.rs` in the
  user's program crate + add a `#[cfg(kani)] mod kani_impl;` line to
  their `src/lib.rs`. One-line modification to `lib.rs`; the actual
  harness body lives in our generated module.
- **Option B**: emit a peer crate at `<root>/kani_harness/` that
  declares the user's program as a `[dependencies]` path dep. Avoids
  modifying user code but requires plumbing the peer crate into
  `cargo kani`'s discovery (passing `--manifest-path` explicitly).
- **Option C**: ask the user to add the `extern crate kani` +
  `mod kani_impl` lines themselves, document in the SKILL. Cleanest
  separation; highest user friction.

The first commit lands Option A. If brownfield users push back on the
`src/lib.rs` modification, we re-evaluate.

### 11b. Symbolic AccountInfo construction

Base template: the existing **Miri** harness at
`examples/pinocchio-fixtures/_harness/account.rs:78-127`. It manually
constructs Pinocchio's wire format (n_accounts header, per-account
[dup_flag|flags|pad|key|owner|lamports|data_len|data|pad|rent_epoch],
then instruction_data, then program_id), then `Box::leak`s the buffer and
hands the raw pointer to Pinocchio's parser.

For Kani, we keep the layout and **substitute `kani::any()` for the
concrete field values**:

```rust
// emit_symbolic_pinocchio_accounts (sketched)
fn build_symbolic_<handler>() -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&(N_ACCOUNTS as u64).to_le_bytes());
    for _ in 0..N_ACCOUNTS {
        buf.push(0xff); // dup_flag — concrete (deduplication is out of scope)
        buf.push(kani::any::<bool>() as u8);   // is_signer
        buf.push(kani::any::<bool>() as u8);   // is_writable
        buf.push(false as u8);                  // executable — concrete (not exercised)
        buf.extend_from_slice(&[0u8; 4]);
        let key:   [u8; 32] = kani::any();
        let owner: [u8; 32] = kani::any();
        buf.extend_from_slice(&key);
        buf.extend_from_slice(&owner);
        buf.extend_from_slice(&kani::any::<u64>().to_le_bytes()); // lamports
        buf.extend_from_slice(&(ACCOUNT_DATA_LEN as u64).to_le_bytes());
        for _ in 0..ACCOUNT_DATA_LEN {
            buf.push(kani::any::<u8>());
        }
        // pad to 8-byte boundary (concrete)
        let pad = (8 - (ACCOUNT_DATA_LEN % 8)) % 8;
        buf.extend_from_slice(&vec![0u8; pad]);
        buf.extend_from_slice(&kani::any::<u64>().to_le_bytes()); // rent_epoch
    }
    buf.extend_from_slice(&(INSTRUCTION_DATA_LEN as u64).to_le_bytes());
    for _ in 0..INSTRUCTION_DATA_LEN {
        buf.push(kani::any::<u8>());
    }
    buf.extend_from_slice(&kani::any::<[u8; 32]>()); // program_id
    buf
}
```

`N_ACCOUNTS`, `ACCOUNT_DATA_LEN`, `INSTRUCTION_DATA_LEN` come from the
spec's per-handler `accounts {…}` block + MemoryLayout (see §11d).

### 11c. Handler invocation + pre/post snapshots

```rust
#[kani::proof]
fn verify_transfer_impl_ensures_0() {
    let buf = build_symbolic_transfer();
    let ptr = Box::leak(buf.into_boxed_slice()).as_mut_ptr();

    // Use Pinocchio's runtime parser the same way entrypoint! does.
    let (program_id, accounts, instruction_data) = unsafe {
        pinocchio::entrypoint::deserialize::<MAX_ACCOUNTS>(ptr)
    };

    // Pre-snapshot: deserialize account[i] into the typed View per
    // MemoryLayout. For SPL Token accounts this is
    // pinocchio_token::state::Account::load(...).
    let pre_src_amount  = unsafe { TokenAccount::load_unchecked(&accounts[0]).amount() };
    let pre_dst_amount  = unsafe { TokenAccount::load_unchecked(&accounts[1]).amount() };

    let result = user_crate::transfer::process_transfer(accounts, instruction_data);

    if result.is_ok() {
        let post_src_amount = unsafe { TokenAccount::load_unchecked(&accounts[0]).amount() };
        let post_dst_amount = unsafe { TokenAccount::load_unchecked(&accounts[1]).amount() };

        // Translated ensures: post.dst.amount == pre.dst.amount + amount
        let amount = u64::from_le_bytes(instruction_data[..8].try_into().unwrap());
        assert!(post_dst_amount == pre_dst_amount + amount);
    }
}
```

The `let pre_X = …` / `let post_X = …` shape mirrors what the Anchor
emitter does today (`kani_impl.rs:rewrite_pre_post_paths`). The
difference is the field-access function — Anchor uses
`ctx.accounts.<field>`, Pinocchio uses `<typed_view>::load(...)?.<field>`.

### 11d. MemoryLayout requirement

Per `[[feedback_memorylayout_sources]]`: brownfield Pinocchio gets layout
via IDL (Anchor Borsh / Pinocchio `mem::offset_of!` probe). The Kani-impl
emitter needs to:

1. For each `state` field referenced by an `ensures` clause, look up the
   account that carries it (via `on_account` or
   `find_canonical_state_account_name`).
2. Look up that account's layout in the MemoryLayout map.
3. Emit the typed-view load + field access (`pinocchio_token::state::Account::load(...).amount()` for SPL Token; user-provided typed view via the spec's `layout = "..."` attribute for custom state).

For the first commit of slice 8, we restrict to **SPL Token accounts only**
(layouts known via `pinocchio_token::state`). Custom-state Pinocchio
programs require the MemoryLayout pipeline to land first; tracked as 8b.

### 11e. Open questions / risks

- **Does `cargo kani` work against `no_std` lib dependencies?**
  **ANSWERED — yes, with caveats** (M1 smoke test at
  `examples/pinocchio-fixtures/kani-smoke/`, commit `<TODO>`).
  Kani 0.67.0 builds + verifies against a `pinocchio = "0.8"` lib dep in
  ~0.5s. The two caveats are now §11a Findings 1+2: lib needs `extern
  crate kani`, and harnesses must live inside the lib (not in `tests/`).
- **BMC unwind cost on symbolic byte buffers**: every byte of account
  data + instruction data is symbolic. For a 165-byte SPL Token account
  this is 165 × 8 = 1320 symbolic bits per account, × N accounts.
  May need `#[kani::unwind(N)]` annotations or explicit
  `kani::assume(...)` bounds to keep proofs tractable. Per-handler
  tuning likely needed.
- **Pinocchio version drift**: the wire format in `_harness/account.rs:80-98`
  is annotated "Pinocchio v0.6.x". The current registry has
  `pinocchio-0.10.1`. If the wire format shifted between 0.6 and 0.10,
  the harness will silently produce mis-parsed AccountInfo. Validation
  step (§11f) explicitly re-checks against 0.10.1's
  `account_info.rs:from_raw_parts`.
- **PDA validation**: Pinocchio handlers re-derive PDAs manually via
  `find_program_address`. Kani can't symbolically execute SHA256
  efficiently. For PDA-validating handlers, emit
  `kani::assume(claimed_pda == expected_pda)` — punt the actual hash
  verification, trust the assumption, document it.
- **`unsafe` in the harness**: every `borrow_mut_data_unchecked()` call
  is `unsafe`. Kani doesn't object to `unsafe` (it just checks UB), but
  the emitted harness inherits the same `// SAFETY:` discipline as the
  user's handler. Mirror their pattern.

### 11f. Validation plan for slice 8

Three milestones, smallest-to-largest. Don't ship the emitter without
clearing all three.

**M1 — Kani-on-no_std smoke test** (**DONE** — landed at
`examples/pinocchio-fixtures/kani-smoke/`).

Originally planned against `ptoken-transfer` but redirected mid-flight:
that fixture's source uses `pinocchio_token::state::Account` /
`Account::load_mut`, an API that doesn't exist in the actual
`pinocchio-token = 0.3.0` crate it depends on (real API is
`TokenAccount` + `from_account_info_unchecked`, immutable-only). The
fixture is source-rotted relative to its declared dep.

Switched to a self-contained smoke fixture under `kani-smoke/` to
isolate the M1 question ("does Kani work against no_std deps") from
the orthogonal fixture-source rot. Result: ✅ verified successfully in
<1s. Findings captured in §11a (harness placement) and §11e (open
questions resolved).

Repairing `ptoken-transfer`'s source to match the real
`pinocchio-token 0.3.0` API is a prerequisite for M2 but not M1. Add
to the M2 task list.

**M2 — Hand-crafted harness against real fixture** (~1 day + fixture
repair). Two pieces:
1. **Repair `ptoken-transfer` fixture** to match real
   `pinocchio-token 0.3.0` API (rename `Account` → `TokenAccount`,
   replace `load_mut` / `set_amount` with the actual immutable
   accessor pattern + raw pointer mutation if needed for the audit
   patterns the fixture exercises).
2. **Write the conservation harness** that asserts
   `pre.src.amount + pre.dst.amount == post.src.amount +
   post.dst.amount`. Per the research findings, the fixture uses
   unchecked arithmetic at `transfer.rs:80-81`, so this should
   produce a Kani counterexample. Validates the harness shape catches
   real bugs.

**M3 — Codegen the harness** (~1-2 days): build `emit_kani_impl_pinocchio`
that generates the exact string from M2 from the spec + program crate
name. Snapshot test against the M2 reference. Wire into the dispatch
per §7c.

### 11g. Implementation skeleton (commit map)

The slice lands across 3-4 commits on `feat/2.30`:

1. **Smoke test** (M1 — **DONE**): self-contained fixture at
   `examples/pinocchio-fixtures/kani-smoke/` validates Kani works
   against `no_std` Pinocchio. Captured two design-shifting findings:
   harness lives in lib (not `tests/`), and lib needs `extern crate
   kani`. See §11a.
2. **Reference harness** (M2): hand-write a real
   `tests/kani_impl.rs` that asserts the conservation ensures.
   Commit captures the target shape we'll codegen toward.
3. **Codegen + dispatch refactor** (M3): land
   `emit_kani_impl_pinocchio` + the §7c dispatch refactor in
   `kani_impl::generate_from_spec`. Anchor branch keeps working,
   Pinocchio branch reaches the new emitter, Quasar still no-ops
   (slice 5 follow-on). Snapshot test against M2.
4. **Optional cleanup**: extract shared helpers between Anchor and
   Pinocchio emitters if duplication warrants it. Don't pre-optimize;
   wait until Quasar also lands.

## 10. Commit shape

Single commit on `feat/2.30`:

```
feat(quasar): emit SPL Token transfer CPI via TokenCpi trait

Refactors try_emit_anchor_cpi into a per-target dispatch
(try_emit_cpi) and lands the first Quasar SPL emitter. Quasar
SPL transfer now emits a one-line method chain
(self.token_program.transfer(...).invoke()?) instead of falling
through to the post-43a532d todo!() stub.

Other Quasar SPL handlers (mint_to, burn, …) and generic Quasar
CPI continue to fall through — staged as follow-on slices per
docs/design/quasar-cpi-spike.md.
```

If validation layer (c) reveals an issue with `cargo check` against
quasar-spl, that's a second commit (potentially a real bug fix, not just
a doc tweak).
