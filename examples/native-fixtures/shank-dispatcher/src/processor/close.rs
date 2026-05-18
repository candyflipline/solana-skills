use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, pubkey::Pubkey,
};

pub fn process_close(_program_id: &Pubkey, _accounts: &[AccountInfo]) -> ProgramResult {
    msg!("close");
    Ok(())
}
