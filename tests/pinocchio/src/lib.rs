//! Shape of a Pinocchio program using solex: `no_std`, hand-rolled dispatch.
#![no_std]

use pinocchio::{AccountView, Address, ProgramResult, error::ProgramError};

solex::declare_id!();

pub const TREASURY_OWNER: &str = solex::var!("treasury_owner");

#[solex::gated]
#[repr(u8)]
pub enum Instruction {
    Open = 1,
    #[channel(devnet)]
    OpenWithHooks = 10,
}

#[solex::gated]
pub fn process_instruction(
    _program_id: &Address,
    _accounts: &mut [AccountView],
    data: &[u8],
) -> ProgramResult {
    match data.first().copied() {
        Some(1) => Ok(()),
        #[channel(devnet)]
        Some(10) => open_with_hooks(),
        _ => Err(ProgramError::InvalidInstructionData),
    }
}

/// Runtime epoch gate on a Pinocchio handler.
#[solex::channel([devnet, 900])]
fn open_with_hooks() -> ProgramResult {
    Ok(())
}

pub fn id_matches(a: &Address) -> bool {
    a == &ID
}
