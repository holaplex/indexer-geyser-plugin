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

#[derive(Debug, Clone, Copy)]
enum Heuristic<T> {
    Used(T),
    Unused,
}

impl<T> Heuristic<T> {
    fn try_get(&self) -> Option<&T> {
        match self {
            Self::Used(v) => Some(v),
            Self::Unused => None,
        }
    }

    fn get(&self) -> &T {
        match self {
            Self::Used(v) => v,
            Self::Unused => panic!("Attempted to use heuristic marked as unused"),
        }
    }

    fn get_mut(&mut self) -> &mut T {
        match self {
            Self::Used(v) => v,
            Self::Unused => panic!("Attempted to use heuristic marked as unused"),
        }
    }

    fn into_inner(self) -> T {
        match self {
            Self::Used(v) => v,
            Self::Unused => panic!("Attempted to use heuristic marked as unused"),
        }
    }
}

#[derive(Debug)]
pub struct AccountSelector {
    owners: HashSet<[u8; 32]>,
    pubkeys: HashSet<[u8; 32]>,
    mints: HashSet<Pubkey>,
    startup: Option<bool>,
    token_reg: Heuristic<Option<HashSet<Pubkey>>>,
}

impl AccountSelector {
    pub fn from_config(config: Accounts) -> Result<Self> {
        let Accounts {
            owners,
            all_tokens,
            pubkeys,
            mints,
            startup,
        } = config;

        let owners = owners
            .into_iter()
            .map(|s| s.parse().map(Pubkey::to_bytes))
            .collect::<Result<_, _>>()
            .context("Failed to parse account owner keys")?;

        let pubkeys = pubkeys
            .into_iter()
            .map(|s| s.parse().map(Pubkey::to_bytes))
            .collect::<Result<_, _>>()
            .context("Failed to parse account pubkeys")?;

        let mints = mints
            .into_iter()
            .map(|s| s.parse())
            .collect::<Result<_, _>>()
            .context("Failed to parse token account mint addresses")?;

        let mut ret = Self {
            owners,
            pubkeys,
            mints,
            startup,
            token_reg: Heuristic::Used(if all_tokens {
                None
            } else {
                Some(HashSet::new())
            }),
        };

        // Don't screen tokens if we're never going to return them
        if !ret.owners.contains(TOKEN_KEY.as_ref()) {
            ret.token_reg = Heuristic::Unused;
        }

        Ok(ret)
    }

    /// Lazy-load the token addresses.  Fails if token addresses are not wanted
    /// or if they have already been loaded.
    pub fn init_token_registry(&mut self, addrs: HashSet<Pubkey>) {
        assert!(self.token_reg.get().as_ref().unwrap().is_empty());
        *self.token_reg.get_mut() = Some(addrs);
    }

    #[inline]
    pub fn startup(&self) -> StartupType {
        StartupType::new(self.startup)
    }

    #[inline]
    pub fn screen_token_registry(&self) -> bool {
        self.token_reg.try_get().map_or(false, Option::is_some)
    }

    #[inline]
    pub fn is_selected(&self, acct: &ReplicaAccountInfo, is_startup: bool) -> bool {
        let ReplicaAccountInfo { owner, data, .. } = *acct;

        if self.startup.map_or(false, |s| is_startup != s) {
            return false;
        }

        if self.pubkeys.contains(acct.pubkey) {
            return true;
        }

        let token = once_cell::unsync::Lazy::new(|| {
            if owner == TOKEN_KEY.as_ref() && data.len() == TokenAccount::get_packed_len() {
                TokenAccount::unpack_from_slice(data).ok()
            } else {
                None
            }
        });

        if !self.mints.is_empty() && token.map_or(false, |t| self.mints.contains(&t.mint)) {
            return true;
        }

        if !self.owners.contains(owner) {
            return false;
        }

        let maybe_not_nft = self.token_reg.get().as_ref().and_then(|reg| {
            let token = token.as_ref()?;

            Some(token.amount > 1 || reg.contains(&token.mint))
        });

        if maybe_not_nft.unwrap_or(false) {
            return false;
        }

        true
    }
}

#[derive(Debug)]
pub struct InstructionSelector {
    programs: HashSet<Pubkey>,
    screen_tokens: Heuristic<bool>,
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

        let mut ret = Self {
            programs,
            screen_tokens: Heuristic::Used(!all_token_calls),
        };

        // Don't screen token calls if we're never going to return them
        if !ret.programs.contains(&TOKEN_KEY) {
            ret.screen_tokens = Heuristic::Unused;
        }

        Ok(ret)
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

        if self.screen_tokens.into_inner() && *pgm == TOKEN_KEY {
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
