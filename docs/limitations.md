# QEDGen — known limitations

A running list of constructs the spec parser accepts but downstream codegen
can't (yet) lower to a non-vacuous harness. Each entry includes the lint
`qedgen check` emits when the shape appears, plus the supported workaround.

## Quantifier shapes (v2.20 §S1.1)

QEDGen lowers `forall <binder> : <T>, body` to a per-slot predicate
`<prop>_at(s, <binder>)` plus a Kani / proptest harness that binds
`<binder>` symbolically (`kani::any::<T>()` / `any::<T>()`). That covers
**single-binder forall** over any of:

- Integer primitives (`U8…U128`, `I8…I128`)
- `Bool`
- `Fin[N]` (bounded index type)
- Named record / sum / lifecycle-state types declared with `type`

Shapes that *don't* fit the above produce the P5
`unsupported_quantifier_shape` lint at `qedgen check` time and skip the
harness emission for that property. The categories below trip the lint.

### `nested-quantifiers`

```
property foo : forall d : Distribution, forall c : Claim, P(d, c)
```

**Why it fails:** the harness model binds *one* symbolic value per
property; nested quantifiers need a Cartesian product the BMC / proptest
loop can't enumerate in a single test.

**Workaround:** split into two single-binder properties.

```
property foo_outer : forall d : Distribution, P_d(d)
property foo_inner : forall c : Claim,        P_c(c)
```

If the body genuinely needs both at once (e.g. `P(d, c)` doesn't
factor), use a record literal to combine them into one binder type:

```
type Pair = { d : Distribution, c : Claim }
property foo : forall p : Pair, P(p.d, p.c)
```

### `unbounded-binder`

```
property foo : forall v : Vec<U64>, sum(v) >= 0
property foo : forall v : List<Account>, …
```

**Why it fails:** `Vec<T>` / `List<T>` / `Set<T>` are unbounded; neither
Kani's symbolic execution nor proptest's strategy generation can enumerate
all possible values.

**Workaround:** use `Map[N] T` for a bounded collection (stored as a
spec field rather than a binder), or bound the quantifier to one element
via a primitive index:

```
type AccountMap = Map[MAX_ACCOUNTS] Account
type Idx = Fin[MAX_ACCOUNTS]

property foo : forall i : Idx, s.accounts[i].balance >= 0
```

### `exists-quantifier`

```
property witness : exists d : Distribution, d.balance > 0
```

**Why it fails:** v2.20 only lowers `forall`. Witnessing an `exists`
needs a constructive proof at the harness level — currently outside the
mechanical-codegen contract.

**Workaround:** rephrase as the matching `forall` invariant if the
intent is "every element satisfies P":

```
property all_positive : forall d : Distribution, d.balance > 0
```

If the intent is genuinely "there is at least one" (a liveness-style
statement), encode it as a `cover` or `liveness` declaration instead of
a property.

---

## Pubkey state fields (v2.20 §S1.3)

`Pubkey`-typed state fields are not supported by v2.20 codegen. Declaring
one on a State type produces the P6
`pubkey_state_field_unsupported` lint at `qedgen check` time.

```
type State
  | Active of {
      authority : Pubkey,   // ← P6
      balance   : U64,
    }
```

**Why it fails:** the proptest harness generator filters Pubkey fields
out of the generated `struct State { ... }` (no clean `proptest::Arbitrary`
lowering for `Pubkey` in the current pipeline), while handler bodies still
emit `s.authority = authority`. Net effect: a generated `tests/proptest.rs`
that doesn't compile (`no field 'authority' on State`).

**Workaround — two options.** Pick whichever fits the property you're
trying to verify:

1. **Replace with `[u8; 32]`** — `Pubkey`'s on-chain representation. Kani
   and proptest can both enumerate symbolic 32-byte arrays, so the field
   stays in state and downstream `s.authority == admin` comparisons work
   unchanged.

   ```
   type State
     | Active of {
         authority : [u8; 32],
         balance   : U64,
       }
   ```

2. **Model the value as a handler parameter** instead of state. If the
   pubkey doesn't need to persist across calls — e.g. a one-shot admin
   check at handler entry — pass it as a parameter rather than storing
   it.

   ```
   handler set_admin (new_admin : Pubkey) : State.Active -> State.Active {
     // Pubkey-as-parameter is supported; the elision bug is state-only.
     // No state field needed.
   }
   ```

   `Pubkey`-typed handler parameters are accepted by `qedgen check` and
   codegen; the constraint is specifically on `field : Pubkey` inside
   declared state.

**What v2.21 may change:** Option B of the S1.3 ADR — lower `Pubkey` state
fields to `[u8; 32]` in the generated structs automatically — is the
follow-up. Until then, P6 is the user-visible gate.

---

## Past limitations (lifted)

- **`forall` over wider-than-U8 binder types verified vacuously** —
  fixed in v2.20 §S1.1. The harness layer now binds the symbol via
  `kani::any` / proptest `any::<T>()` and calls `<prop>_at(&s, <binder>)`
  instead of the legacy `<prop>(&s) ⟶ true` stub.

## Reporting

When `qedgen check` emits a P5 lint pointing at one of the categories
above and you've hit a *new* shape that should be supported, open an
issue at <https://github.com/qedgen/solana-skills/issues> with the
fragment that doesn't lower.
