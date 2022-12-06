//! Configuration blocks for the Geyser selectors

use hashbrown::HashSet;
use serde;
use serde::Deserialize;

/// Configuration block for [`AccountSelector`](crate::AccountSelector)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Accounts {
    /// The set of account owners to filter by
    #[serde(default)]
    pub owners: HashSet<String>,

    /// A set of account public keys to always select.  This ignores all other
    /// filters except `startup`
    #[serde(default)]
    pub pubkeys: HashSet<String>,

    /// The set of token mints to filter accounts belonging to [`spl_token`] by
    #[serde(default)]
    pub mints: HashSet<String>,

    /// Filter for changing how to interpret the `is_startup` flag.
    ///
    /// This option has three states:
    ///  - `None`: Ignore the `is_startup` flag and send all updates.
    ///  - `Some(true)`: Only send updates when `is_startup` is `true`.
    ///  - `Some(false)`: Only send updates when `is_startup` is `false`.
    #[serde(default)]
    pub startup: Option<bool>,

    /// Set to true to disable heuristics to reduce the number of incoming
    /// token account updates.  Has no effect if the spl-token pubkey is not in
    /// the owners list.
    #[serde(default)]
    pub all_tokens: bool,
}

/// Configuration block for [`InstructionSelector`](crate::InstructionSelector)
#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct Instructions {
    /// The set of target programs to filter by
    #[serde(default)]
    pub programs: HashSet<String>,

    /// Set to true to disable heuristics to reduce the number of incoming
    /// token instructions.  Has no effect if the spl-token pubkey is not in the
    /// programs list.  Currently the heuristics are tailored towards NFT burns,
    /// only passing through instructions whose data indicates a burn of amount
    /// 1.
    #[serde(default)]
    pub all_token_calls: bool,
}
