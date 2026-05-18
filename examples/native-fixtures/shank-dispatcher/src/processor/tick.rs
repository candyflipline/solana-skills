use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, pubkey::Pubkey,
};

pub fn process_tick(_program_id: &Pubkey, _accounts: &[AccountInfo]) -> ProgramResult {
    msg!("tick");
    Ok(())
}
