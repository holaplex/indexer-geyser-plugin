//! Queue configuration for Solana Geyser plugins intended to communicate
//! with `holaplex-indexer`.

use std::time::Duration;

use serde::{Deserialize, Serialize};
pub use solana_program::pubkey::Pubkey;
use solana_transaction_status::EncodedConfirmedTransactionWithStatusMeta;

use crate::{
    queue_type::{Binding, QueueProps, RetryProps},
    suffix::Suffix,
    Result,
};

/// Message data for an account update
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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

/// Message data for an instruction notification
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
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
}

/// Message data for an instruction notification
#[derive(Debug, Serialize, Deserialize)]
#[serde(transparent)]
pub struct TransactionNotify {
    /// the transactions
    pub transaction: EncodedConfirmedTransactionWithStatusMeta,
}

/// A message transmitted by a Geyser plugin
#[derive(Debug, Serialize, Deserialize)]
#[serde(untagged)]
pub enum Message {
    /// Indicates an account should be updated
    AccountUpdate(AccountUpdate),
    /// Indicates an instruction was included in a **successful** transaction
    InstructionNotify(InstructionNotify),
    /// Indicates an instruction was included in a **successful** transaction
    TransactionNotify(Box<TransactionNotify>),
}

impl Message {
    /// the routing key to use for this message type
    #[must_use]
    pub fn routing_key(&self) -> Option<&str> {
        match self {
            Message::AccountUpdate(_) => Some("account"),
            Message::InstructionNotify(_) => Some("instruction"),
            Message::TransactionNotify(_) => Some("transaction"),
        }
    }
}

/// AMQP configuration for Geyser plugins
#[derive(Debug, Clone)]
pub struct QueueType {
    props: QueueProps,
}

/// Network hint for declaring exchange and queue names
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::Display)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, strum::EnumString, strum::Display)]
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
            "{}{}.messages",
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
                binding: Binding::Direct(String::from("unused")),
                prefetch: 4096,
                max_len_bytes: if suffix.is_debug() || matches!(startup_type, StartupType::Normal) {
                    100 * 1024 * 1024 // 100 MiB
                } else {
                    8 * 1024 * 1024 * 1024 // 8 GiB
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
