use hashbrown::HashSet;
use solana_program::instruction::CompiledInstruction;
use solana_program::pubkey::Pubkey;

use crate::{config::Instructions, Error, Heuristic, Result};

/// Helper for performing screening logic on Solana instructions
#[derive(Debug)]
pub struct Selector {
    programs: HashSet<Pubkey>,
    screen_tokens: Heuristic<bool>,
}

impl Selector {
    /// Construct a new selector from the given configuration block
    ///
    /// # Errors
    /// Fails if a program address is incorrectly specified
    pub fn from_config(config: Instructions) -> Result<Self> {
        let Instructions {
            programs,
            all_token_calls,
        } = config;

        let programs = programs
            .into_iter()
            .map(|s| s.parse::<Pubkey>())
            .collect::<Result<_, _>>()
            .map_err(|e| Error::InstructionConfig("programs", e.into()))?;

        let mut ret = Self {
            programs,
            screen_tokens: Heuristic::Used(!all_token_calls),
        };

        // Don't screen token calls if we're never going to return them
        if !ret.programs.contains(&spl_token::id()) {
            ret.screen_tokens = Heuristic::Unused;
        }

        Ok(ret)
    }

    /// Returns true if this selector will never select anything
    #[inline]
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    /// Returns true if the given instruction addressed to the given program
    /// has been requested by this selector's configuration
    #[inline]
    #[must_use]
    pub fn is_selected(&self, pgm: &Pubkey, ins: &CompiledInstruction) -> bool {
        if !self.programs.contains(pgm) {
            return false;
        }

        if self.screen_tokens.into_inner() && *pgm == spl_token::id() {
            if let [8, rest @ ..] = ins.data.as_slice() {
                let amt = rest.try_into().map(u64::from_le_bytes);

                if !matches!(amt, Ok(1)) {
                    return false;
                }

                debug_assert_eq!(
                    ins.data,
                    spl_token::instruction::TokenInstruction::Burn { amount: 1_u64 }.pack(),
                );
            } else {
                return false;
            }
        }

        true
    }
}
