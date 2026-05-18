//! Instruction enum + minimal deserialiser. Mirrors the Shank/Phoenix
//! style: one variant per dispatcher arm; `try_from(&[u8])` parses the
//! first byte as a tag.

use solana_program::program_error::ProgramError;

pub enum WidgetInstruction {
    InitializeWidget { capacity: u64 },
    Tick,
    Close,
}

impl TryFrom<&[u8]> for WidgetInstruction {
    type Error = ProgramError;

    fn try_from(value: &[u8]) -> Result<Self, Self::Error> {
        let (tag, rest) = value
            .split_first()
            .ok_or(ProgramError::InvalidInstructionData)?;
        match tag {
            0 => {
                let capacity_bytes: [u8; 8] = rest
                    .get(..8)
                    .ok_or(ProgramError::InvalidInstructionData)?
                    .try_into()
                    .map_err(|_| ProgramError::InvalidInstructionData)?;
                Ok(Self::InitializeWidget {
                    capacity: u64::from_le_bytes(capacity_bytes),
                })
            }
            1 => Ok(Self::Tick),
            2 => Ok(Self::Close),
            _ => Err(ProgramError::InvalidInstructionData),
        }
    }
}
