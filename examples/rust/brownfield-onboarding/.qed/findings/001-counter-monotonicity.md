# [HIGH] bump — broken_invariant — monotonicity violation

## Summary

`bump` decrements the counter instead of incrementing it. Under any
unpaused call with `delta > 0` the counter advances *downward* —
underflowing on the first call from `counter == 0` and propagating
the wrong value forward.

This is a one-character typo (`wrapping_sub` instead of `checked_add`
or `wrapping_add`) that no compiler diagnostic, type check, or doc
comment surfaces. It looks fine on inspection — the audit caught it
via the monotonicity property below.

## Category

`broken_invariant` — Family 2 in `finding_to_spec.md` (arithmetic
with intent drift; the spec promises forward progress, the
implementation walks backward).

## Citation

- **File:** `program/src/lib.rs`
- **Line:** 22 (the body of `bump` — the arithmetic statement)
- **Pre-fix code:**
  ```rust
  s.counter = s.counter.wrapping_sub(delta);
  ```
- **Post-fix code:**
  ```rust
  s.counter = s.counter.checked_add(delta).ok_or(Error::Overflow)?;
  ```

## Repro status: fired

Mollusk repro emitted at
`target/qedgen-repros/audit/001-counter-monotonicity.rs` with
counterexample `(State { counter: 0, paused: 0 }, delta: 1) →
post.counter = u64::MAX - 0` (the underflow value).

## Spec construct that locks this in

From `finding_to_spec.md` Family 2 → "monotonic counter under a
gate":

```
property counter_monotonic :
  state.counter >= old(state.counter)
  preserved_by all
```

The `old(...)` reference makes this a Binary property in v2.23 — the
generated proptest harness emits

```rust
fn counter_monotonic(pre: &State, post: &State) -> bool {
    post.counter >= pre.counter
}
```

and captures `let pre = s.clone(); let mut post = s;` before the
handler call. On the buggy `wrapping_sub` variant the test fires red
with the same counterexample as the Mollusk repro.

## Failure mode (regression net)

Any future patch that re-introduces a subtraction, swaps `+=` for
`-=`, or otherwise causes `bump` to write a value less than the
pre-state value into `counter` fails the proptest harness with:

```
counter_monotonic must hold after bump (binary: pre/post)
  pre.counter = 0, delta = 1, post.counter = 18446744073709551615
```

## Authority / threat model context

`bump` is `permissionless`. Any caller can drive arbitrary `delta`
values, so the bug is exploitable by any tx — there's no signer gate
that would have constrained the inputs. The pause flag offers no
protection because the bug fires on the unpaused path. This
elevates the severity from a routine arithmetic bug to a HIGH —
public exploit, no signer cost, immediate impact on every reader of
`state.counter`.
