# Release v2.19.1 — Auditor skill-surface refactor

v2.19.1 is a patch release. **No Rust code changes, no CLI surface
changes, no behavior changes for spec-mode or codegen users.** The
release is a focused refactor of the auditor skill's loaded context:
the 1212-line `skills/qedgen-auditor/exploits.md` corpus file is
retired, its load-bearing patterns are promoted into SKILL.md
categories, and its narrative content moves to
`docs/security-primer.md` outside the loaded skill surface.

The motivation is empirical. A blind pre-audit walk against a
mature DeFi target hit 6/3/1 (HIT / PARTIAL / MISS) against
published audit-firm findings *without referencing `exploits.md`
once*. The reasoning chains ran on SKILL.md's category catalog and
the cross-cutting 3a / 3b / 3c walks. `exploits.md` was paying for
context budget every audit while contributing nothing to the
finding chain. v2.19.1 removes that cost.

## What's in

### Slice 1 — 10 new SKILL.md categories

Patterns that were load-bearing in `exploits.md` but absent from
the prior catalog are now first-class categories with per-runtime
predicates and Corpus lines. All ten are spec-less; they apply
during the per-handler walk:

| Category | Severity | Shape |
|---|---|---|
| `rounding_direction_round_trip` | HIGH | Bidirectional conversion pair (`A→B` + `B→A`) rounding the same direction on both legs → unbounded round-trip profit |
| `duplicate_mutable_accounts_aliasing` | HIGH | Same-typed mutable account parameters with no `from.key != to.key` assertion → no-op transfer with side-effects firing |
| `twap_gameable_single_block` | HIGH | TWAP window ≤ 1-2 slots, sampling spot pool inside same tx → flash-loan-amplified manipulation |
| `liquidation_rounding_dust_accumulation` | MEDIUM | Liquidation rounds collateral seize and debt repay both down → repeat-slice dust strategy |
| `flash_loan_amplified_governance` | HIGH | Governance vote-weight read at vote time from live balance, not snapshot → flash-loan + vote + repay |
| `authority_transfer_missing_nominate_accept` | MEDIUM | Single-ix `set_authority` with no two-step handshake → fat-finger / compromised-key catastrophic |
| `missing_rent_exemption_check_on_init` | HIGH | Init accepts caller-supplied lamports without `Rent::minimum_balance` floor → mid-op GC + reinit |
| `realloc_zero_init_data_leak` | HIGH | Anchor `realloc(new_size, false)` → grown tail contains stale adjacent heap |
| `sentinel_null_key_array_short_circuit` | MEDIUM | `Pubkey::default()` used as "empty slot" sentinel + weak signer validation → zero-key signer bypass |
| `permissionless_instruction_no_rate_limit` | MEDIUM | Permissionless handler mutating meaningful state with no per-caller / per-slot cap → DoS via state-bloat |

### Slice 2 — Corpus lines updated on existing categories

Twelve existing categories now name the public incidents and
recurring audit-firm patterns the prior `exploits.md` entries
contributed, condensed to one-liner Corpus pointers:

- `account_type_confusion` — Wormhole sysvar-instructions spoof
  ($326M), Cashio fake-account chain ($52.8M), Crema Finance fake
  tick account ($8.8M); cross-reference to
  `field_chain_missing_root_anchor` for the field-level forgery
  sub-class.
- `missing_owner_check` — typed-account-with-untyped-owner pattern
  with explicit scope-pair clarifier to `token_account_role_anchoring`.
- `arbitrary_cpi` — CPI without program-id check on Token CPI;
  Anchor's `Program<T>` typed wrapper closes the primitive
  structurally.
- `arithmetic_overflow_wrapping` — most-cited recurring primitive
  across Solana audit reports; cross-references
  `rounding_direction_round_trip` for the asymmetric-rounding
  sub-class.
- `close_account_redirection` — Jet Protocol C-ratio close-account
  bypass (2022, $25M near-miss; private disclosure).
- `discriminator_collision` — `unpack_unchecked` and similar
  recurring shapes.
- `pda_seed_collision` — PDA sharing across authority domains,
  authority not stored in PDA seeds.
- `unvalidated_remaining_accounts` — permissionless account-add
  via remaining_accounts (governance-hijack-lite sub-shape).
- `account_not_reloaded_after_cpi` — pairs with
  `token_2022_extension_arithmetic_skew` on fee-on-transfer mints.
- `init_without_is_initialized` — canonical Cashio-shape
  kill-chain when paired with `pda_lifecycle_reuse_after_close`.
- `oracle_staleness` — Mango ($114M), Solend USDH ($1.26M),
  Nirvana ($3.5M), Loopscale RateX ($5.8M); cross-reference to
  `twap_gameable_single_block`.
- `frontrunnable_no_slippage` — sandwich / MEV against AMM swap,
  Mango perp-market manipulation, permissionless `claim` / `crank`
  frontrun.
- `lamport_write_demotion` — King of the SOL freeze pattern
  (OtterSec public blog post — attribution restored from prior
  over-strip).
- `qed_hash_drift_or_forgery` — trusted upstream binary not pinned
  recurring shape.

### Slice 3 — `docs/security-primer.md`

New file, **outside the loaded skill surface**. Three parts:

1. **Named on-chain exploits** (9 entries with full narrative):
   Wormhole, Cashio, Mango, Crema, Saber, Solend USDH, Nirvana,
   Loopscale, Jet Protocol. Each entry names the SKILL.md
   categories it maps to so an auditor reading the primer can
   pivot back to the working catalog.
2. **Operational / off-chain threat-model incidents** (5 entries):
   Raydium admin-key trojan, Cypher economic-loop, durable-nonce
   admin-takeover, mobile wallet Sentry-telemetry key leak,
   `@solana/web3.js` supply-chain backdoor. Each entry is framed
   as an auditor question rather than a code pattern, because the
   bugs were operational, not in the program.
3. **Out-of-scope reference** (1 entry): wallet SDK and end-user
   signing flows are outside the QEDGen Auditor's coverage by
   design; escalate to dedicated wallet-security review when
   auditing projects that ship both program code and a wallet /
   signing companion.

The primer is reference material — not loaded by the auditor on
every invocation. The skill loads SKILL.md only; `docs/` lives in
the repo for human readers but `npx skills add` doesn't include it
in the agent context.

### Slice 4 — Public-blog attributions restored

Three attributions over-stripped in the 2026-05-17 copyright sweep
are restored as nominative use:

- Saber / SPL token-swap rounding → Neodyme public disclosure (now
  cited inline in `rounding_direction_round_trip` Corpus).
- King of the SOL → OtterSec public blog post (now cited in
  `lamport_write_demotion` Corpus).
- Wormhole sysvar post-mortem → OtterSec / Wormhole joint
  post-mortem (now cited in `account_type_confusion` Corpus).

Proprietary PDF finding IDs (OS-EPS-XXX, SHIELD_PHNX_XXX) remain
stripped — those reference paid-report taxonomies, not public
disclosures.

## What's out

- **`skills/qedgen-auditor/exploits.md`** is removed from the
  repository. Auditor invocations no longer load it.
- **No new probe categories, no new spec-mode codegen rules, no
  new CLI flags.** v2.19.1 is purely a skill-surface refactor.
- **No retroactive Phoenix or rewards audit re-write.** Existing
  audit artifacts under `~/code/audits/` use the v2.19.0
  catalog's reference style by design.

## Token-budget impact

The auditor's loaded context drops by roughly one full corpus
file. `exploits.md` was 1212 lines (~64 KB) and 57 entries.
SKILL.md grew from 1412 to ~1640 lines net of the 10 promoted
categories and Corpus-line edits. Net reduction in loaded-skill
token count: ~50%.

The win is not raw size — it's *signal density*. Every line in
the loaded surface now contributes to a working predicate or
chain primitive; nothing decorative survives. Auditors who want
historical / operational context know exactly where to find it
(`docs/security-primer.md`); the catalog stays lean for the
investigative loop.

## Upgrade notes

No action required for users on v2.19.0:

- The CLI is byte-identical to v2.19.0 (no `qedgen` commands
  changed surface).
- The Rust code is unchanged (`cargo test` / `cargo clippy` /
  `cargo fmt` are sanity-only this release).
- Bundled examples have not regenerated — same Lean / proptest /
  Kani artifacts ship.

Skill users: `npx skills add qedgen/solana-skills` (or local
re-install) picks up the refactored SKILL.md automatically.
Existing `.claude/skills/qedgen-auditor/exploits.md` files in
your project tree will go stale on next sync and can be deleted
manually if `skills add` doesn't prune them.

## Why a patch release, not a minor

Per `feedback_minor_release_completeness.md`: minors are for
additive features; patches are for documentation and quality
improvements. v2.19.1 is the latter, with one caveat — the 10
promoted categories *are* additive to the catalog. The patch
classification holds because:

- No CLI surface change.
- No spec-mode or codegen behavior change.
- No backwards-incompatible change. The categories added are pure
  additions; users who relied on `exploits.md` directly (if anyone
  did — it was never a user-facing artifact, only auditor-loaded
  context) lose a corpus file but gain a better-organized SKILL.md.

Ships ahead of v2.20 to clear the auditor catalog before the v2.20
work touches the same surface.
