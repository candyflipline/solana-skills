//! Deliberately buggy Anchor program used by the v2.21 Slice 1
//! Crucible-crash-first regression fixture. See ../README.md.
//!
//! Both handlers panic on every invocation — Crucible's brownfield
//! protocol-mode harness should surface both bugs as `Finding`s without
//! a `.qedspec` ever being written.

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
}

#[derive(Accounts)]
pub struct Empty<'info> {
    /// Stand-in unchecked account; the bug fires before any account
    /// access happens so the contents don't matter.
    /// CHECK: not validated; brownfield demo only.
    pub stub: AccountInfo<'info>,
}
