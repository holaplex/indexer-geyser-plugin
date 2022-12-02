//! Support logic for configuring queues with suffixed names

use std::fmt::Write;

use crate::{Error, Result};

/// A suffix for an AMQP object, to avoid name collisions with staging or debug
/// builds
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub enum Suffix {
    /// This is a production name
    Production,
    /// This is a staging name, to be treated similarly to production
    Staging,
    /// This is a debug name, identified further with a unique name
    Debug(String),
    /// This is always a production name, even when compiled in debug mode.
    /// Should only be used if you know what you're doing!
    ///
    /// This variant cannot be constructed from arguments.
    ProductionUnchecked,
}

impl Suffix {
    #[inline]
    pub(crate) fn is_debug(&self) -> bool {
        matches!(self, Self::Debug(_))
    }

    pub(crate) fn format(&self, mut prefix: String) -> Result<String> {
        match self {
            Self::Production if cfg!(debug_assertions) => {
                return Err(Error::InvalidQueueType(
                    "Debug builds must specify a unique debug suffix for all AMQP names",
                ))
            },
            Self::Production | Self::ProductionUnchecked => (),
            Self::Staging => write!(prefix, ".staging").unwrap_or_else(|_| unreachable!()),
            Self::Debug(s) => write!(prefix, ".debug.{}", s).unwrap_or_else(|_| unreachable!()),
        }

        Ok(prefix)
    }
}
