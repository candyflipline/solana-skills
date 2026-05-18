use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    pubkey::Pubkey,
};

/// `process_close` is the trader-gated shape: requires a signer, but
/// the signer's identity is open — no comparison against a stored
/// authority field. Any wallet can sign and close their own resource.
///
/// v2.20 §S2.2 fixture: handler should classify as `trader_gated`.
pub fn process_close(_program_id: &Pubkey, accounts: &[AccountInfo]) -> ProgramResult {
    let owner = &accounts[0];
    if !owner.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    msg!("close");
    Ok(())
}
