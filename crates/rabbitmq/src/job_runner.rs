//! Queue configuration for dispatching background jobs.

use std::time::Duration;

use serde::{Deserialize, Serialize};

use crate::{
    geyser::StartupType,
    queue_type::{Binding, QueueProps, RetryProps},
    suffix::Suffix,
    Result,
};

/// Message data for a slot reindex request
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct SlotReindex {
    /// The slot ID to refresh
    pub slot: u64,
    /// The startup type of the AMQP queue to target
    pub startup: StartupType,
}

/// Message data for a job dispatch request
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Message {
    /// Refresh a table of cached data
    RefreshTable(String),
    /// Reindex a given slot
    ReindexSlot(SlotReindex),
}

/// AMQP configuration for job runners
#[derive(Debug, Clone)]
pub struct QueueType {
    props: QueueProps,
}

impl QueueType {
    /// Construct a new queue configuration given the expected sender and queue
    /// suffix configuration
    ///
    /// # Errors
    /// This function fails if the given queue suffix is invalid.
    pub fn new(sender: &str, suffix: &Suffix) -> Result<Self> {
        let exchange = format!("{}.jobs", sender);
        let queue = suffix.format(format!("{}.runner", exchange))?;

        Ok(Self {
            props: QueueProps {
                exchange,
                queue,
                binding: Binding::Fanout,
                prefetch: 1,
                max_len_bytes: 100 * 1024 * 1024, // 100 MiB
                auto_delete: suffix.is_debug(),
                retry: Some(RetryProps {
                    max_tries: 5,
                    delay_hint: Duration::from_secs(5),
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

/// The type of an search indexer producer
#[cfg(feature = "producer")]
pub type Producer = crate::producer::Producer<QueueType>;
/// The type of an search indexer consumer
#[cfg(feature = "consumer")]
pub type Consumer = crate::consumer::Consumer<QueueType>;
