//! Fixture: minimal Pinocchio-shaped `process_transfer` mirroring
//! `solana-program/token/pinocchio/program/src/processor/shared/transfer.rs`.
//!
//! Hand-authored to match the patterns called out in PRD v2.19
//! "What real Pinocchio looks like":
//!
//!   - Unchecked load of accounts via
//!     `TokenAccount::from_account_info_unchecked` (SAFETY claim chain).
//!   - Unchecked arithmetic on token amounts (raw `+` / `-` instead of
//!     `checked_add` / `checked_sub`) — canonical-stale-claim probe
//!     target ("token amounts are always within u64 range — cannot
//!     overflow").
//!   - Raw pointer mutation of account bytes (since
//!     `pinocchio_token::state::TokenAccount`'s fields are private and
//!     it exposes no setters — production code mutates via the byte
//!     slice returned by `borrow_mut_data_unchecked`).
//!
//! Real p-token is far larger; this fixture is scoped to the patterns
//! the v2.19 success bar tests + the conservation property the slice 8
//! M2 Kani harness exercises (`docs/design/quasar-cpi-spike.md` §11f).
//!
//! ## API repair (slice 8 M2)
//!
//! The pre-repair version used `pinocchio_token::state::Account` +
//! `Account::load_mut` + `.set_amount(x)` — none of which exist in
//! `pinocchio-token = 0.3.0` (the version this fixture's Cargo.toml
//! pins). M2 repairs the source to use the real API while preserving
//! the audit-target patterns above. Native-mint lamport sync is
//! dropped — it muddies the borrow story without adding a new audit
//! target (the original used `checked_add` for destination lamports
//! anyway, so it wasn't a probe surface).

use pinocchio::{account_info::AccountInfo, program_error::ProgramError, ProgramResult};
use pinocchio_token::state::TokenAccount;

/// Offset of `TokenAccount.amount` in the on-disk layout:
///   mint:   Pubkey  (32 bytes, offset 0)
///   owner:  Pubkey  (32 bytes, offset 32)
///   amount: [u8; 8] (8 bytes,  offset 64)
/// Per `pinocchio_token::state::token::TokenAccount`.
const AMOUNT_OFFSET: usize = 64;

/// Transfer `amount` tokens from `source_account_info` to
/// `destination_account_info`.
///
/// Authority is `authority_info` (must be signer).
pub fn process_transfer(
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let [source_account_info, _mint_info, destination_account_info, authority_info, ..] =
        accounts
    else {
        return Err(ProgramError::NotEnoughAccountKeys);
    };

    if instruction_data.len() < 8 {
        return Err(ProgramError::InvalidInstructionData);
    }
    let amount = u64::from_le_bytes(instruction_data[..8].try_into().unwrap());
    if amount == 0 {
        return Ok(());
    }

    if !authority_info.is_signer() {
        return Err(ProgramError::MissingRequiredSignature);
    }

    // Read source's current amount + owner via the typed view.
    // SAFETY: source_account_info is a writable token account validated
    // by the caller; we hold the immutable view in a narrow scope and
    // do not mutate through it (mutation happens via the byte slice
    // below, after the immutable view is dropped).
    let source_amount_before;
    {
        let src_view = unsafe { TokenAccount::from_account_info_unchecked(source_account_info)? };
        if src_view.owner() != authority_info.key() {
            return Err(ProgramError::IllegalOwner);
        }
        source_amount_before = src_view.amount();
    }

    // Read destination's current amount.
    // SAFETY: destination is a writable token account, validated
    // separately from source (caller guarantees they're distinct).
    let dst_amount_before;
    {
        let dst_view =
            unsafe { TokenAccount::from_account_info_unchecked(destination_account_info)? };
        dst_amount_before = dst_view.amount();
    }

    // === Audit-target arithmetic ===
    // **The amount of a token account is always within the range of
    // the mint supply (`u64`), so this addition cannot overflow.**
    // (Canonical stale-claim probe target — the bound is enforced for
    // the mint supply, not for the sum of two arbitrary account
    // amounts.)
    let dst_amount_after = dst_amount_before + amount;
    let source_amount_after = source_amount_before - amount;

    // Mutate via raw pointer. `TokenAccount`'s fields are private and
    // the type exposes no setters; production code in pinocchio-based
    // SPL programs mutates the on-disk amount through the byte slice
    // returned by `borrow_mut_data_unchecked`.
    //
    // SAFETY: destination_account_info is writable and the immutable
    // typed view from the prior scope is already dropped, so there's
    // no aliasing violation. The offset is from
    // `pinocchio_token::state::token::TokenAccount`'s repr(C) layout.
    unsafe {
        let dst_bytes = destination_account_info.borrow_mut_data_unchecked();
        core::ptr::write_unaligned(
            dst_bytes.as_mut_ptr().add(AMOUNT_OFFSET) as *mut u64,
            dst_amount_after,
        );
        let src_bytes = source_account_info.borrow_mut_data_unchecked();
        core::ptr::write_unaligned(
            src_bytes.as_mut_ptr().add(AMOUNT_OFFSET) as *mut u64,
            source_amount_after,
        );
    }

    Ok(())
}
