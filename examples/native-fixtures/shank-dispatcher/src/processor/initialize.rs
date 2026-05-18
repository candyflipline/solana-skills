use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, pubkey::Pubkey,
};

pub fn process_initialize_widget(
    _program_id: &Pubkey,
    _accounts: &[AccountInfo],
    capacity: u64,
) -> ProgramResult {
    msg!("initialize widget: capacity={}", capacity);
    Ok(())
}
