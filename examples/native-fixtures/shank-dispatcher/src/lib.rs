//! Synthesised native Solana program with a Shank-style central-match
//! dispatcher. Used by `qedgen probe --bootstrap` (v2.20 §S2.1) to exercise
//! handler discovery against a representative pre-Anchor program shape.
//!
//! Generic 3-handler example. Not modelled after any specific deployed
//! program — just the canonical pattern.

use solana_program::{
    account_info::AccountInfo, entrypoint, entrypoint::ProgramResult, msg, program_error::ProgramError,
    pubkey::Pubkey,
};

use crate::instruction::WidgetInstruction;
use crate::processor::{
    close::process_close, initialize::process_initialize_widget, tick::process_tick,
};

pub mod instruction;
pub mod processor;

entrypoint!(process_instruction);

pub fn process_instruction(
    program_id: &Pubkey,
    accounts: &[AccountInfo],
    instruction_data: &[u8],
) -> ProgramResult {
    let instruction = WidgetInstruction::try_from(instruction_data)
        .map_err(|_| ProgramError::InvalidInstructionData)?;
    match instruction {
        WidgetInstruction::InitializeWidget { capacity } => {
            msg!("ix: InitializeWidget");
            process_initialize_widget(program_id, accounts, capacity)
        }
        WidgetInstruction::Tick => {
            msg!("ix: Tick");
            process_tick(program_id, accounts)
        }
        WidgetInstruction::Close => {
            msg!("ix: Close");
            process_close(program_id, accounts)
        }
    }
}
