//! Deliberately buggy Anchor program used by the v2.21 Slice 1
//! Crucible-crash-first + Slice §S1.2 lamport-conservation regression
//! fixtures. See ../README.md.
//!
//! Three handlers, each demonstrating a different protocol-invariant
//! violation that Crucible's brownfield harness surfaces without a
//! `.qedspec` ever being written:
//!
//!   * `run`   — divide-by-zero panic (intrinsic Crucible detector).
//!   * `maybe` — `Option::unwrap` on `None` (same intrinsic detector).
//!   * `drain` — routes all `source` lamports into `target` with no
//!               authority check, triggering the v2.21 §S1.2 lamport-
//!               inflation guard when the fuzzer picks `target` as a
//!               tracked signer.

use anchor_lang::prelude::*;

declare_id!("11111111111111111111111111111111");

#[program]
pub mod buggy_anchor {
    use super::*;

    /// Divides a constant by zero — fires `runtime_panic` on iteration 1.
    pub fn run(ctx: Context<Empty>) -> Result<()> {
        let _ = ctx;
        let zero: u32 = 0;
        let _ = 100u32 / zero;
        Ok(())
    }

    /// Unwraps a `None` — fires `runtime_panic` on iteration 1.
    pub fn maybe(ctx: Context<Empty>) -> Result<()> {
        let _ = ctx;
        let value: Option<u32> = None;
        let _ = value.unwrap();
        Ok(())
    }

    /// Sweeps every lamport from `source` into `target` with no
    /// authority check. When Crucible's fuzzer happens to pick a
    /// tracked signer for `target`, the brownfield harness's
    /// `assert_no_signer_inflation` guard fires — surfacing a drain
    /// shape (lamports appeared on a signer from outside the tracked
    /// set) without any spec annotation.
    pub fn drain(ctx: Context<DrainAccounts>) -> Result<()> {
        let source = &ctx.accounts.source;
        let target = &ctx.accounts.target;
        let credit = source.lamports();
        **source.try_borrow_mut_lamports()? = 0;
        **target.try_borrow_mut_lamports()? =
            target.lamports().checked_add(credit).unwrap();
        Ok(())
    }
}

#[derive(Accounts)]
pub struct Empty<'info> {
    /// Stand-in unchecked account; the bug fires before any account
    /// access happens so the contents don't matter.
    /// CHECK: not validated; brownfield demo only.
    pub stub: AccountInfo<'info>,
}

#[derive(Accounts)]
pub struct DrainAccounts<'info> {
    /// CHECK: source PDA; no validation, deliberate drain shape.
    #[account(mut)]
    pub source: AccountInfo<'info>,
    /// CHECK: target signer; no validation, deliberate drain shape.
    #[account(mut)]
    pub target: AccountInfo<'info>,
}
