use crate::interface::ReplicaAccountInfo;
use selector::prelude::*;
use solana_program::instruction::CompiledInstruction;

#[repr(transparent)]
pub struct AccountShim<'a>(pub &'a ReplicaAccountInfo<'a>);

impl<'a> AccountInfo for AccountShim<'a> {
    #[inline]
    fn owner(&self) -> &[u8] {
        self.0.owner
    }

    #[inline]
    fn pubkey(&self) -> &[u8] {
        self.0.pubkey
    }

    #[inline]
    fn data(&self) -> &[u8] {
        self.0.data
    }
}

#[repr(transparent)]
pub struct CompiledInstructionShim<'a>(pub &'a CompiledInstruction);

impl<'a> InstructionInfo<'a> for CompiledInstructionShim<'a> {
    type AccountIndices = std::iter::Copied<std::slice::Iter<'a, u8>>;

    fn program_index(&self) -> u8 {
        self.0.program_id_index
    }

    fn account_indices(&self) -> Self::AccountIndices {
        self.0.accounts.iter().copied()
    }

    fn data(&self) -> &[u8] {
        &self.0.data
    }
}
