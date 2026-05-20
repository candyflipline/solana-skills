# Release v2.22.1 — DSL reference cleanup (doc-only hotfix)

Documentation hotfix. Removes three entries from
`references/qedspec-dsl.md` that documented constructs the chumsky
parser doesn't semantically support. No code changes; no
behaviour changes; users authoring `.qedspec` files against the
documented surface are unaffected because all 45 bundled examples
already comply.

## What's in

### Drop unsupported keywords from the DSL reference (`74b879f`)

Audited every keyword the doc referenced against the parser's
`top_item` choice, `HandlerClause` enum, and reserved `KEYWORDS`
list. Three drift items removed:

1. **`schema X { ... }` top-level block.** Not in `top_item` —
   the chumsky parser never accepts it. The doc's "Reusable clause
   fragments. Handlers include them with `include`" section was
   aspirational.

2. **`include schema_name` handler clause.** The parser recognises
   the `Include(String)` `HandlerClause` variant, but
   `chumsky_adapter::HandlerClause::Include(_)` carries the comment
   `// Schema includes: forward-compat; ignored in phase 1.` and
   discards it. `ast.rs` docstring also says "phase 1 rejects." The
   clause type-checks but has zero downstream effect.

3. **`on ident` / `when ident` / `then ident` sugar clauses.**
   Listed in the handler-clauses table as "sugar, prefer signature"
   but `HandlerClause` has no `On` / `When` / `Then` variants and
   the parser doesn't accept these tokens in a handler body.

### Example audit

Confirmed zero use of any removed construct across all 45 bundled
`.qedspec` files under `examples/`:

  $ grep -rnE 'schema|include' --include="*.qedspec" examples/
  examples/regressions/issue-42-conditional/fee_router.qedspec:25://   - Drift fingerprint includes per-arm structure
  examples/regressions/issue-8/repro-03-duplicate-theorem.qedspec:15:// Fix site: ... format_name includes

  $ grep -rnE '^\s*(on|when|then)\s+[A-Z]\w*\s*$' --include="*.qedspec" examples/
  (no matches)

Only matches are comment-text occurrences of the substring
"includes" in two regression-test specs — no `include` or `schema`
clause actually written.

## What stays

The `state` sugar, `takes` clause, `aborts_total`, `permissionless`,
`establishes`, `invariant`, `match`, `transfers`, `emits`, `effect`,
`modifies`, `let`, `requires`, `ensures`, `auth`, `accounts`, `call`
clauses all verified against their `HandlerClause` variants. Built-in
expression atoms (`now()`, `mul_div_floor`, `mul_div_ceil`, `old`,
`forall`, `exists`, `sum`, `match ... with`, `is .Variant`, function
application) all match the parser's `atom_base` choice. sBPF pragma
items (`pubkey`, `errors`, `instruction`, `discriminant`, `entry`,
`exit`, `input_layout`, `insn_layout`, `guard`, `checks`, `fuel`,
`scope`, `flow`, `through`, `within`, `cpi`, `after`) all match the
parser's `pragma_item` whitelist.

## Pre-release gates

- [x] `cargo fmt --check`
- [x] `cargo clippy -- -D warnings`
- [x] `cargo test` — 724 passed (unchanged from v2.22.0)
- [x] `bash scripts/check-version-consistency.sh` — 2.22.1
- [x] No example uses any removed keyword
- [x] `CLAUDE.md` ↔ `claude.md` byte-identical

## Deferred

- **Drop the forward-compat `HandlerClause::Include` parser stub
  itself.** v2.23 cleanup item per [[feedback_cleanup_v3]] (minors
  stay additive; structural cleanup batched into next minor). The
  v2.22.1 fix is doc-only so no spec that previously parsed stops
  parsing.

## Footer — relationship to existing memories

- [[feedback_keep_declarative]] — the doc cleanup follows the
  "tight surface" principle: fewer documented constructs is better
  when those constructs don't materialise downstream.
- [[feedback_cleanup_v3]] — parser-side stub cleanup deferred.
