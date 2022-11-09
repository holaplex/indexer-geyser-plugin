//! Support logic for configuring queues with suffixed names

use std::fmt::Write;

use clap::{Arg, ArgGroup, ArgMatches, Command};

use crate::{Error, Result};

/// A suffix for an AMQP object, to avoid name collisions with staging or debug
/// builds
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl clap::Args for Suffix {
    fn augment_args(cmd: Command) -> Command {
        cmd.arg(
            Arg::new("staging")
                .num_args(0)
                .value_parser(clap::builder::BoolishValueParser::new())
                .default_missing_value("true")
                .long("staging")
                .env("STAGING")
                .help("Use a staging queue suffix rather than a debug or production one"),
        )
        .arg(
            Arg::new("suffix")
                .num_args(1)
                .value_parser(clap::builder::NonEmptyStringValueParser::new())
                .required(false)
                .help("An optional debug queue suffix")
                .conflicts_with("staging"),
        )
        .group(ArgGroup::new("Suffix").args(["staging", "suffix"]))
    }

    fn augment_args_for_update(cmd: Command) -> Command {
        Self::augment_args(cmd)
    }
}

impl clap::FromArgMatches for Suffix {
    fn from_arg_matches(matches: &ArgMatches) -> Result<Self, clap::Error> {
        Ok(if matches.get_one("staging").copied().unwrap_or_default() {
            Self::Staging
        } else if let Some(suffix) = matches.get_one("suffix") {
            Self::Debug(String::clone(suffix))
        } else {
            Self::Production
        })
    }

    fn update_from_arg_matches(&mut self, matches: &ArgMatches) -> Result<(), clap::Error> {
        *self = Self::from_arg_matches(matches)?;
        Ok(())
    }
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

#[cfg(test)]
mod tests {
    use super::Suffix;
    use clap::{Args, FromArgMatches};

    fn parse<I: IntoIterator>(it: I) -> Result<Suffix, clap::Error>
    where
        I::Item: Into<std::ffi::OsString> + Clone,
    {
        let cmd = clap::Command::new("rmq-test");
        let cmd = Suffix::augment_args(cmd);
        let matches = cmd.try_get_matches_from(it)?;

        Suffix::from_arg_matches(&matches)
    }

    #[test]
    fn test_suffix() {
        assert!(matches!(parse(["test", "--staging"]), Ok(Suffix::Staging)));

        assert!(matches!(parse(["test", "test"]), Ok(Suffix::Debug(_))));
        if let Ok(Suffix::Debug(d)) = parse(["test"]) {
            assert_eq!(d, "test");
        }

        assert!(matches!(parse(["test"]), Ok(Suffix::Production)));
    }
}
