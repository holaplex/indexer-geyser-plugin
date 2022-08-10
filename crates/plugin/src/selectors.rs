use hashbrown::HashSet;
use indexer_rabbitmq::geyser::StartupType;
use solana_program::{instruction::CompiledInstruction, program_pack::Pack};
use spl_token::state::Account as TokenAccount;

use crate::{
    config::{Accounts, Instructions},
    interface::ReplicaAccountInfo,
    plugin::TOKEN_KEY,
    prelude::*,
};

#[derive(Debug)]
pub struct AccountSelector {
    owners: HashSet<[u8; 32]>,
    startup: Option<bool>,
    token_addresses: Option<HashSet<Pubkey>>,
}

impl AccountSelector {
    pub fn from_config(config: Accounts) -> Result<Self> {
        let Accounts {
            owners,
            all_tokens,
            startup,
        } = config;

        let owners = owners
            .into_iter()
            .map(|s| s.parse().map(Pubkey::to_bytes))
            .collect::<Result<_, _>>()
            .context("Failed to parse account owner keys")?;

        Ok(Self {
            owners,
            startup,
            token_addresses: if all_tokens {
                None
            } else {
                Some(HashSet::new())
            },
        })
    }

    /// Lazy-load the token addresses.  Fails if token addresses are not wanted
    /// or if they have already been loaded.
    pub fn init_tokens(&mut self, addrs: HashSet<Pubkey>) {
        assert!(self.token_addresses.as_ref().unwrap().is_empty());
        self.token_addresses = Some(addrs);
    }

    #[inline]
    pub fn startup(&self) -> StartupType {
        StartupType::new(self.startup)
    }

    #[inline]
    pub fn screen_tokens(&self) -> bool {
        self.token_addresses.is_some()
    }

    #[inline]
    pub fn is_selected(&self, acct: &ReplicaAccountInfo, is_startup: bool) -> bool {
        let ReplicaAccountInfo { owner, data, .. } = *acct;

        if self.startup.map_or(false, |s| is_startup != s) || !self.owners.contains(owner) {
            return false;
        }

        if owner == TOKEN_KEY.as_ref() && data.len() == TokenAccount::get_packed_len() {
            if let Some(ref addrs) = self.token_addresses {
                let token_account = TokenAccount::unpack_from_slice(data);

                if let Ok(token_account) = token_account {
                    if token_account.amount > 1 || addrs.contains(&token_account.mint) {
                        return false;
                    }
                }
            }
        }

        true
    }
}

#[derive(Debug)]
pub struct InstructionSelector {
    programs: HashSet<Pubkey>,
    screen_tokens: bool,
}

impl InstructionSelector {
    pub fn from_config(config: Instructions) -> Result<Self> {
        let Instructions {
            programs,
            all_token_calls,
        } = config;

        let programs = programs
            .into_iter()
            .map(|s| s.parse())
            .collect::<Result<_, _>>()
            .context("Failed to parse instruction program keys")?;

        Ok(Self {
            programs,
            screen_tokens: !all_token_calls,
        })
    }

    #[inline]
    pub fn is_empty(&self) -> bool {
        self.programs.is_empty()
    }

    #[inline]
    pub fn is_selected(&self, pgm: &Pubkey, ins: &CompiledInstruction) -> bool {
        if !self.programs.contains(pgm) {
            return false;
        }

        if self.screen_tokens && *pgm == TOKEN_KEY {
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
