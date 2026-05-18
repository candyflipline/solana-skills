use solana_program::{
    account_info::AccountInfo, entrypoint::ProgramResult, msg, program_error::ProgramError,
    pubkey::Pubkey,
};

/// Synthetic config-account state header. Real programs would
/// deserialize via borsh / bytemuck — kept inline here so the fixture
/// stays self-contained.
#[repr(C)]
struct WidgetConfig {
    pub authority: Pubkey,
    pub capacity: u64,
}

/// `process_initialize_widget` is the canonical authority-gated shape:
/// the caller must be the stored authority, and the handler refuses
/// to proceed if the signer key doesn't match the stored authority
/// field.
///
/// v2.20 §S2.2 fixture: handler should classify as `authority_gated`.
pub fn process_initialize_widget(
    _program_id: &Pubkey,
    accounts: &[AccountInfo],
    capacity: u64,
) -> ProgramResult {
    let signer = &accounts[0];
    let config_account = &accounts[1];

    let data = config_account.try_borrow_data()?;
    let config = unsafe { &*(data.as_ptr() as *const WidgetConfig) };

    // The v2.20 §S2.2 `authority_gated` shape: signer's key compared
    // against a stored `.authority` field on the program state.
    if *signer.key != config.authority {
        return Err(ProgramError::IllegalOwner);
    }
    if !signer.is_signer {
        return Err(ProgramError::MissingRequiredSignature);
    }
    msg!("initialize widget: capacity={}", capacity);
    Ok(())
}
