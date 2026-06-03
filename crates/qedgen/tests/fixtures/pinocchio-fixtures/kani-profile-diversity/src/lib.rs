pub type Pubkey = [u8; 32];
pub type ProgramResult = Result<(), ProgramError>;

pub struct ProgramError;

pub struct AccountInfo;

impl AccountInfo {
    pub fn key(&self) -> &Pubkey {
        unimplemented!()
    }

    pub fn is_signer(&self) -> bool {
        unimplemented!()
    }

    pub fn is_writable(&self) -> bool {
        unimplemented!()
    }
}

pub const ASSOCIATED_TOKEN_PROGRAM_ID: Pubkey = [8u8; 32];
pub const TOKEN_PROGRAM_ID: Pubkey = [9u8; 32];
pub const VAULT_AUTHORITY_SEED: &[u8] = b"vault-authority";
pub const SPL_TOKEN_ID: Pubkey = [10u8; 32];

pub fn next_account_info<'a, I>(_iter: &mut I) -> Result<&'a AccountInfo, ProgramError>
where
    I: Iterator<Item = &'a AccountInfo>,
{
    unimplemented!()
}

pub fn require_key(_account: &AccountInfo, _key: &Pubkey) -> ProgramResult {
    Ok(())
}

pub fn require_token_account(
    _account: &AccountInfo,
    _mint: &Pubkey,
    _owner: &Pubkey,
) -> ProgramResult {
    Ok(())
}

pub fn read_mint_decimals(_mint: &AccountInfo) -> Result<u8, ProgramError> {
    Ok(6)
}

pub fn derive_vault_authority(program_id: &Pubkey, lane_id: u8) -> (Pubkey, u8) {
    pinocchio::pubkey::find_program_address(&[VAULT_AUTHORITY_SEED, &[lane_id]], program_id)
}

pub fn derive_token_vault(authority: &Pubkey, mint: &Pubkey) -> (Pubkey, u8) {
    pinocchio::pubkey::find_program_address(
        &[authority.as_ref(), TOKEN_PROGRAM_ID.as_ref(), mint.as_ref()],
        &ASSOCIATED_TOKEN_PROGRAM_ID,
    )
}

pub fn process_instruction(
    program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let (tag, data) = instruction_data.split_first().unwrap();
    match *tag {
        1 => process_move_tokens(program_id, accounts, data),
        2 => process_move_batch(program_id, accounts, data),
        3 => process_touch_config(program_id, accounts, data),
        4 => process_route_ata(program_id, accounts, data),
        5 => process_partial_fallback(program_id, accounts, data),
        _ => Ok(()),
    }
}

fn process_move_tokens(
    _program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let [source, destination, authority, mint, token_program, ..] = accounts else {
        return Ok(());
    };
    let _amount = u64::from_le_bytes(instruction_data.get(0..8).unwrap().try_into().unwrap());
    require_token_account(source, mint.key(), authority.key())?;
    require_token_account(destination, mint.key(), authority.key())?;
    require_key(token_program, &SPL_TOKEN_ID)?;
    let _decimals = read_mint_decimals(mint)?;
    Ok(())
}

fn process_move_batch(
    _program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let [batch_state, ..] = accounts else {
        return Ok(());
    };
    let _transfer_count = instruction_data.get(0).copied().unwrap_or(0);
    if !batch_state.is_writable() {
        return Ok(());
    }
    Ok(())
}

fn process_touch_config(
    _program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let account_info_iter = &mut accounts.iter();
    let config = next_account_info(account_info_iter)?;
    let _max_fee_bps = u16::from_le_bytes(instruction_data.get(0..2).unwrap().try_into().unwrap());
    if !config.is_writable() {
        return Ok(());
    }
    Ok(())
}

fn process_route_ata(
    program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let [authority, mint, vault, ..] = accounts else {
        return Ok(());
    };
    let lane_id = u8::from_le_bytes(instruction_data.get(0..1).unwrap().try_into().unwrap());
    let authority_key = derive_vault_authority(program_id, lane_id).0;
    require_key(authority, &authority_key)?;
    require_key(vault, &derive_token_vault(&authority_key, mint.key()).0)?;
    Ok(())
}

fn process_partial_fallback(
    _program_id: &pinocchio::pubkey::Pubkey,
    accounts: &[AccountInfo],
    _instruction_data: &[u8],
) -> ProgramResult {
    let [scratch, ..] = accounts else {
        return Ok(());
    };
    if scratch.is_writable() {
        Ok(())
    } else {
        Ok(())
    }
}

mod pinocchio {
    pub mod pubkey {
        pub type Pubkey = [u8; 32];

        pub fn find_program_address(_seeds: &[&[u8]], _program_id: &Pubkey) -> (Pubkey, u8) {
            unimplemented!()
        }
    }
}
