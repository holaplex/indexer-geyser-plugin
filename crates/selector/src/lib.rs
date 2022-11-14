//! Solana validator selector components for `holaplex-indexer`.

#![deny(
    clippy::disallowed_methods,
    clippy::suspicious,
    clippy::style,
    missing_debug_implementations,
    missing_copy_implementations
)]
#![warn(clippy::pedantic, clippy::cargo, missing_docs)]

mod account;
pub mod config;
mod instruction;

pub use account::Selector as AccountSelector;
pub use instruction::Selector as InstructionSelector;

/// An error originating in this crate
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// An error occurred while loading the account selector config
    #[error("Error parsing field {0:?} of account selector configuration: {1}")]
    AccountConfig(
        &'static str,
        #[source] Box<dyn std::error::Error + Send + Sync + 'static>,
    ),
    /// An error occurred while loading the instruction selector config
    #[error("Error parsing field {0:?} of instruction selector configuration: {1}")]
    InstructionConfig(
        &'static str,
        #[source] Box<dyn std::error::Error + Send + Sync + 'static>,
    ),
}

pub(crate) type Result<T, E = Error> = std::result::Result<T, E>;

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
