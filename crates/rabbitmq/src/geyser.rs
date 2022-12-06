//! Queue configuration for Solana Geyser plugins intended to communicate
//! with `holaplex-indexer`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
pub use solana_program::pubkey::Pubkey;

use crate::{
    queue_type::{Binding, QueueProps, RetryProps},
    suffix::Suffix,
    Result,
};

/// Message data for an account update
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AccountUpdate {
    /// The account's public key
    pub key: Pubkey,
    /// The lamport balance of the account
    pub lamports: u64,
    /// The Solana program controlling this account
    pub owner: Pubkey,
    /// True if the account's data is an executable smart contract
    pub executable: bool,
    /// The next epoch for which this account will owe rent
    pub rent_epoch: u64,
    /// The binary data stored on this account
    pub data: Vec<u8>,
    /// Monotonic-increasing counter for sequencing on-chain writes
    pub write_version: u64,
    /// The slot in which this account was updated
    pub slot: u64,
    /// True if this update was triggered by a validator startup
    pub is_startup: bool,
}

/// The index of an instruction in a transaction
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum InstructionIndex {
    /// This instruction was included directly in the transaction message
    TopLevel(usize),
    /// This is a sub-instruction whose index is represented as
    /// `(parent, child)`
    Inner(u8, usize),
}

/// Message data for an instruction notification
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct InstructionNotify {
    /// The program this instruction was executed with
    pub program: Pubkey,
    /// The binary instruction opcode
    pub data: Vec<u8>,
    /// The account inputs to this instruction
    pub accounts: Vec<Pubkey>,
    /// The slot in which the transaction including this instruction was
    /// reported
    pub slot: u64,
    /// Signature of the transaction enclosing this instruction
    pub txn_signature: Vec<u8>,
    /// The index of this instruction, and if it is a sub-inst
    pub index: InstructionIndex,
}

/// Solana slot status, corresponding to the Geyser interface's enumeration of
/// the same name.
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
#[allow(missing_docs)]
pub enum SlotStatus {
    Processed,
    Rooted,
    Confirmed,
}

/// Message data for a block status update
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SlotStatusUpdate {
    /// The number of the slot that was updated
    pub slot: u64,
    /// The parent of the slot
    pub parent: Option<u64>,
    /// The status of the slot
    pub status: SlotStatus,
}

/// A message transmitted by a Geyser plugin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Indicates an account should be updated
    AccountUpdate(AccountUpdate),
    /// Indicates an instruction was included in a **successful** transaction
    InstructionNotify(InstructionNotify),
    /// Indicates the status of a slot changed
    SlotStatusUpdate(SlotStatusUpdate),
}

/// AMQP configuration for Geyser plugins
#[derive(Debug, Clone)]
pub struct QueueType {
    props: QueueProps,
}

/// Network hint for declaring exchange and queue names
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::Display,
)]
#[strum(serialize_all = "kebab-case")]
pub enum Network {
    /// Use the network ID `"mainnet"`
    Mainnet,
    /// Use the network ID `"devnet"`
    Devnet,
    /// Use the network ID `"testnet"`
    Testnet,
}

/// Startup message hint for declaring exchanges and queues
#[derive(
    Debug,
    Clone,
    Copy,
    PartialEq,
    Eq,
    Hash,
    Serialize,
    Deserialize,
    strum::EnumString,
    strum::Display,
)]
#[strum(serialize_all = "kebab-case")]
pub enum StartupType {
    /// Ignore startup messages
    Normal,
    /// Ignore non-startup messages
    Startup,
    /// Include all messages
    All,
}

impl StartupType {
    /// Construct a [`StartupType`] from the Geyser plugin `startup` filter.
    #[must_use]
    pub fn new(value: Option<bool>) -> Self {
        match value {
            None => Self::All,
            Some(false) => Self::Normal,
            Some(true) => Self::Startup,
        }
    }
}

impl QueueType {
    /// Construct a new queue configuration given the network this validator is
    /// connected to and queue suffix configuration
    ///
    /// # Errors
    /// This function fails if the given queue suffix is invalid.
    pub fn new(network: Network, startup_type: StartupType, suffix: &Suffix) -> Result<Self> {
        let exchange = format!(
            "{}{}.accounts",
            network,
            match startup_type {
                StartupType::Normal => "",
                StartupType::Startup => ".startup",
                StartupType::All => ".startup-all",
            }
        );
        let queue = suffix.format(format!("{}.indexer", exchange))?;

        Ok(Self {
            props: QueueProps {
                exchange,
                queue,
                binding: Binding::Fanout,
                prefetch: 4096,
                max_len_bytes: match (suffix.is_debug(), startup_type) {
                    (true, _) => 100 * 1024 * 1024,                         // 100 MiB
                    (false, StartupType::Normal) => 4 * 1024 * 1024 * 1024, // 4 GiB
                    (false, _) => 50 * 1024 * 1024 * 1024,                  // 50 GiB
                },
                auto_delete: suffix.is_debug(),
                retry: Some(RetryProps {
                    max_tries: 3,
                    delay_hint: Duration::from_millis(500),
                    max_delay: Duration::from_secs(10 * 60),
                }),
            },
        })
    }
}

impl crate::QueueType for QueueType {
    type Message = Message;

    #[inline]
    fn info(&self) -> crate::queue_type::QueueInfo {
        (&self.props).into()
    }
}

/// The type of a Geyser producer
#[cfg(feature = "producer")]
pub type Producer = crate::producer::Producer<QueueType>;
/// The type of a Geyser consumer
#[cfg(feature = "consumer")]
pub type Consumer = crate::consumer::Consumer<QueueType>;
