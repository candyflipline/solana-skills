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
