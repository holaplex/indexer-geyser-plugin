use std::{env, sync::Arc};

use anyhow::Context;
use hashbrown::HashSet;
use indexer_rabbitmq::geyser::{
    AccountUpdate, InstructionIndex, InstructionNotify, Message, SlotStatus as RmqSlotStatus,
    SlotStatusUpdate,
};
use selector::{AccountSelector, InstructionSelector};
use solana_geyser_plugin_interface::geyser_plugin_interface::SlotStatus;
use solana_program::{instruction::CompiledInstruction, message::AccountKeys};

use serde::Deserialize;

use crate::{
    config::Config,
    interface::{
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoVersions,
        ReplicaTransactionInfoVersions, Result,
    },
    metrics::{Counter, Metrics},
    prelude::*,
    selector::{AccountShim, CompiledInstructionShim},
    sender::Sender,
};

const UNINIT: &str = "RabbitMQ plugin not initialized yet!";

#[inline]
fn custom_err<E: Into<Box<dyn std::error::Error + Send + Sync + 'static>>>(
    counter: &'_ Counter,
) -> impl FnOnce(E) -> GeyserPluginError + '_ {
    |e| {
        counter.log(1);
        GeyserPluginError::Custom(e.into())
    }
}

#[derive(Debug)]
pub(crate) struct Inner {
    rt: tokio::runtime::Runtime,
    producer: Sender,
    acct_sel: AccountSelector,
    ins_sel: InstructionSelector,
    metrics: Arc<Metrics>,
}

impl Inner {
    pub fn spawn<F: std::future::Future<Output = anyhow::Result<()>> + Send + 'static>(
        self: &Arc<Self>,
        f: impl FnOnce(Arc<Self>) -> F,
    ) {
        self.rt.spawn(f(Arc::clone(self)));
    }
}

/// An instance of the plugin
#[derive(Debug, Default)]
#[repr(transparent)]
pub struct GeyserPluginRabbitMq(Option<Arc<Inner>>);

#[derive(Deserialize)]
struct TokenItem {
    address: String,
}

#[derive(Deserialize)]
struct TokenList {
    tokens: Vec<TokenItem>,
}

impl GeyserPluginRabbitMq {
    const TOKEN_REG_URL: &'static str = "https://raw.githubusercontent.com/solana-labs/token-list/main/src/tokens/solana.tokenlist.json";

    async fn load_token_reg() -> anyhow::Result<HashSet<Pubkey>> {
        let res: TokenList = reqwest::get(Self::TOKEN_REG_URL)
            .await
            .context("HTTP request failed")?
            .json()
            .await
            .context("Failed to parse response JSON")?;

        res.tokens
            .into_iter()
            .map(|TokenItem { address }| address.parse())
            .collect::<StdResult<_, _>>()
            .context("Failed to convert token list")
    }

    fn expect_inner(&self) -> &Arc<Inner> {
        self.0.as_ref().expect(UNINIT)
    }

    #[inline]
    fn with_inner<T>(
        &self,
        uninit: impl FnOnce() -> GeyserPluginError,
        f: impl FnOnce(&Arc<Inner>) -> anyhow::Result<T>,
    ) -> Result<T> {
        match self.0 {
            Some(ref inner) => f(inner).map_err(custom_err(&inner.metrics.errs)),
            None => Err(uninit()),
        }
    }
}

impl GeyserPlugin for GeyserPluginRabbitMq {
    fn name(&self) -> &'static str {
        "GeyserPluginRabbitMq"
    }

    fn on_load(&mut self, cfg: &str) -> Result<()> {
        solana_logger::setup_with_default("info");

        let metrics = Metrics::new_rc();

        let version;
        let host;

        {
            let ver = env!("CARGO_PKG_VERSION");
            let git = option_env!("META_GIT_HEAD");
            // TODO
            // let rem = option_env!("META_GIT_REMOTE");

            {
                use std::fmt::Write;

                let mut s = format!("v{}", ver);

                if let Some(git) = git {
                    write!(s, "+git.{}", git).unwrap();
                }

                version = s;
            }

            // TODO
            // let rustc_ver = env!("META_RUSTC_VERSION");
            // let build_host = env!("META_BUILD_HOST");
            // let target = env!("META_BUILD_TARGET");
            // let profile = env!("META_BUILD_PROFILE");
            // let platform = env!("META_BUILD_PLATFORM");

            host = hostname::get()
                .map_err(custom_err(&metrics.errs))?
                .into_string()
                .map_err(|_| anyhow!("Failed to parse system hostname"))
                .map_err(custom_err(&metrics.errs))?;
        }

        let (amqp, jobs, metrics_conf, mut acct_sel, ins_sel) = Config::read(cfg)
            .and_then(Config::into_parts)
            .map_err(custom_err(&metrics.errs))?;

        let startup_type = acct_sel.startup();

        if let Some(config) = metrics_conf.config {
            const VAR: &str = "SOLANA_METRICS_CONFIG";

            if env::var_os(VAR).is_some() {
                warn!("Overriding existing value for {}", VAR);
            }

            env::set_var(VAR, config);
        }

        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .thread_name("geyser-rabbitmq")
            .worker_threads(jobs.limit)
            .max_blocking_threads(jobs.blocking.unwrap_or(jobs.limit))
            .build()
            .map_err(custom_err(&metrics.errs))?;

        let producer = rt.block_on(async {
            let producer = Sender::new(
                amqp,
                format!("geyser-rabbitmq-{}@{}", version, host),
                startup_type,
                Arc::clone(&metrics),
            )
            .await
            .map_err(custom_err(&metrics.errs))?;

            if acct_sel.screen_token_registry() {
                acct_sel.init_token_registry(
                    Self::load_token_reg()
                        .await
                        .map_err(custom_err(&metrics.errs))?,
                );
            }

            Result::<_>::Ok(producer)
        })?;

        self.0 = Some(Arc::new(Inner {
            rt,
            producer,
            acct_sel,
            ins_sel,
            metrics,
        }));

        Ok(())
    }

    fn update_account(
        &mut self,
        account: ReplicaAccountInfoVersions,
        slot: u64,
        is_startup: bool,
    ) -> Result<()> {
        self.with_inner(
            || GeyserPluginError::AccountsUpdateError { msg: UNINIT.into() },
            |this| {
                this.metrics.acct_recvs.log(1);

                match account {
                    ReplicaAccountInfoVersions::V0_0_1(acct) => {
                        if !this.acct_sel.is_selected(&AccountShim(acct), is_startup) {
                            return Ok(());
                        }

                        let ReplicaAccountInfo {
                            pubkey,
                            lamports,
                            owner,
                            executable,
                            rent_epoch,
                            data,
                            write_version,
                        } = *acct;

                        let key = Pubkey::new_from_array(pubkey.try_into()?);
                        let owner = Pubkey::new_from_array(owner.try_into()?);
                        let data = data.to_owned();

                        this.spawn(|this| async move {
                            this.producer
                                .send(Message::AccountUpdate(AccountUpdate {
                                    key,
                                    lamports,
                                    owner,
                                    executable,
                                    rent_epoch,
                                    data,
                                    write_version,
                                    slot,
                                    is_startup,
                                }))
                                .await;
                            this.metrics.acct_sends.log(1);

                            Ok(())
                        });
                    },
                };

                Ok(())
            },
        )
    }

    fn update_slot_status(
        &mut self,
        slot: u64,
        parent: Option<u64>,
        status: SlotStatus,
    ) -> Result<()> {
        self.with_inner(
            || GeyserPluginError::SlotStatusUpdateError { msg: UNINIT.into() },
            |this| {
                this.metrics.status_recvs.log(1);

                this.spawn(|this| async move {
                    this.producer
                        .send(Message::SlotStatusUpdate(SlotStatusUpdate {
                            slot,
                            parent,
                            status: match status {
                                SlotStatus::Processed => RmqSlotStatus::Processed,
                                SlotStatus::Rooted => RmqSlotStatus::Rooted,
                                SlotStatus::Confirmed => RmqSlotStatus::Confirmed,
                            },
                        }))
                        .await;
                    this.metrics.status_sends.log(1);

                    Ok(())
                });

                Ok(())
            },
        )
    }

    fn notify_transaction(
        &mut self,
        transaction: ReplicaTransactionInfoVersions,
        slot: u64,
    ) -> Result<()> {
        #[inline]
        fn process_instruction(
            sel: &InstructionSelector,
            (index, ins): (InstructionIndex, &CompiledInstruction),
            keys: &AccountKeys,
            slot: u64,
            txn_signature: &[u8],
        ) -> anyhow::Result<Option<Message>> {
            if !sel.is_selected(|i| keys.get(i as usize), &CompiledInstructionShim(ins))? {
                return Ok(None);
            }

            let program = *keys
                .get(ins.program_id_index as usize)
                .ok_or_else(|| anyhow!("Couldn't get program ID for instruction"))?;

            let accounts = ins
                .accounts
                .iter()
                .map(|i| {
                    keys.get(*i as usize).map_or_else(
                        || Err(anyhow!("Couldn't get input account for instruction")),
                        |k| Ok(*k),
                    )
                })
                .collect::<StdResult<Vec<_>, _>>()?;

            let data = ins.data.clone();

            Ok(Some(Message::InstructionNotify(InstructionNotify {
                program,
                data,
                accounts,
                slot,
                txn_signature: txn_signature.to_vec(),
                index,
            })))
        }

        self.with_inner(
            || GeyserPluginError::Custom(anyhow!(UNINIT).into()),
            |this| {
                if this.ins_sel.is_empty() {
                    return Ok(());
                }

                this.metrics.txn_recvs.log(1);

                match transaction {
                    ReplicaTransactionInfoVersions::V0_0_1(tx) => {
                        if matches!(tx.transaction_status_meta.status, Err(..)) {
                            return Ok(());
                        }

                        let msg = tx.transaction.message();
                        let keys = msg.account_keys();

                        let txn_signature = tx.signature.as_ref();

                        for ins in msg
                            .instructions()
                            .iter()
                            .enumerate()
                            .map(|(i, ins)| (InstructionIndex::TopLevel(i), ins))
                            .chain(
                                tx.transaction_status_meta
                                    .inner_instructions
                                    .iter()
                                    .flatten()
                                    .flat_map(|ins| {
                                        ins.instructions.iter().enumerate().map(|(i, inner)| {
                                            (InstructionIndex::Inner(ins.index, i), inner)
                                        })
                                    }),
                            )
                        {
                            match process_instruction(
                                &this.ins_sel,
                                ins,
                                &keys,
                                slot,
                                txn_signature,
                            ) {
                                Ok(Some(m)) => {
                                    this.spawn(|this| async move {
                                        this.producer.send(m).await;
                                        this.metrics.ins_sends.log(1);

                                        Ok(())
                                    });
                                },
                                Ok(None) => (),
                                Err(e) => {
                                    warn!("Error processing instruction: {:?}", e);
                                    this.metrics.errs.log(1);
                                },
                            }
                        }
                    },
                }

                Ok(())
            },
        )
    }

    fn account_data_notifications_enabled(&self) -> bool {
        true
    }

    fn transaction_notifications_enabled(&self) -> bool {
        let this = self.expect_inner();
        !this.ins_sel.is_empty()
    }
}
