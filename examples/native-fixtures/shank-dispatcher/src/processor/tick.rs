use solana_program::{
    account_info::AccountInfo, clock::Clock, entrypoint::ProgramResult, msg, pubkey::Pubkey,
    sysvar::Sysvar,
};

/// `process_tick` is the canonical permissionless shape: no signer
/// check, no authority comparison — the handler advances global state
/// based on the clock and can be called by anyone.
///
/// v2.20 §S2.2 fixture: handler should classify as `permissionless`.
pub fn process_tick(_program_id: &Pubkey, _accounts: &[AccountInfo]) -> ProgramResult {
    let clock = Clock::get()?;
    let timestamp = clock.unix_timestamp;
    msg!("tick at {}", timestamp);
    Ok(())
}
