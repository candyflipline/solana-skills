// User-owned. Regenerating the spec does NOT overwrite this file.
// Guard checks live in the sibling `crate::guards` module and ARE
// regenerated on every `qedgen codegen`. Drift between the spec
// handler block and the `spec_hash` below fires a compile_error!
// via the `#[qed(verified, ...)]` macro.

use anchor_lang::prelude::*;
use crate::guards;
use qedgen_macros::qed;
use crate::errors::*;
use crate::state::VaultAccountInner;
use crate::EmergencyClose;

impl<'info> EmergencyClose<'info> {
    #[qed(verified, spec = "../vault.qedspec", handler = "emergency_close", hash = "430a111b688406bc", spec_hash = "fbd50c2e7640978c")]
    #[inline(always)]
    pub fn handler(&mut self) -> Result<()> {
        guards::emergency_close(self)?;
        match &mut self.vault.inner {
            VaultAccountInner::Active { total_deposits, .. } => {
            *total_deposits = 0;
            }
            _ => return Err(VaultError::WrongState.into()),
        }
        // Spec call: Token.transfer (Anchor CPI emitted by v2.8 G4)
        {
            use anchor_spl::token::{self, Transfer};
            let cpi_accounts = Transfer {
                from:      self.vault_ta.to_account_info(),
                to:        self.sink_ta.to_account_info(),
                authority: self.admin.to_account_info(),
            };
            let cpi_program = self.token_program.to_account_info();
            token::transfer(CpiContext::new(cpi_program, cpi_accounts), (*self.vault.inner.total_deposits()))?;
        }
        Ok(())
    }
}
