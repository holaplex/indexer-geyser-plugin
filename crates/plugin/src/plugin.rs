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
        GeyserPlugin, GeyserPluginError, ReplicaAccountInfo, ReplicaAccountInfoV2,
        ReplicaAccountInfoVersions, ReplicaTransactionInfoVersions, Result,
    },
    metrics::{Counter, Metrics},
    prelude::*,
    selector::{AccountShim, AccountShimV2, CompiledInstructionShim},
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

    fn process_instructions<'a>(
        self: &Arc<Self>,
        instructions: impl IntoIterator<Item = (InstructionIndex, &'a CompiledInstruction)>,
        keys: &AccountKeys,
        slot: u64,
        txn_signature: &[u8],
    ) {
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

        let mut any_sent = false;
        for ins in instructions {
            match process_instruction(&self.ins_sel, ins, keys, slot, txn_signature) {
                Ok(Some(m)) => {
                    any_sent = true;
                    self.spawn(|this| async move {
                        this.producer.send(m).await;
                        this.metrics.ins_sends.log(1);

                        Ok(())
                    });
                },
                Ok(None) => (),
                Err(e) => {
                    warn!("Error processing instruction: {:?}", e);
                    self.metrics.errs.log(1);
                },
            }
        }

        if any_sent {
            self.metrics.txn_sends.log(1);
        }
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

                let update = match account {
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

                        AccountUpdate {
                            key: Pubkey::new_from_array(pubkey.try_into()?),
                            lamports,
                            owner: Pubkey::new_from_array(owner.try_into()?),
                            executable,
                            rent_epoch,
                            data: data.to_owned(),
                            write_version,
                            slot,
                            is_startup,
                        }
                    },

                    ReplicaAccountInfoVersions::V0_0_2(acct) => {
                        if !this.acct_sel.is_selected(&AccountShimV2(acct), is_startup) {
                            return Ok(());
                        }

                        let ReplicaAccountInfoV2 {
                            pubkey,
                            lamports,
                            owner,
                            executable,
                            rent_epoch,
                            data,
                            write_version,
                            txn_signature: _, // TODO: send this?
                        } = *acct;

                        AccountUpdate {
                            key: Pubkey::new_from_array(pubkey.try_into()?),
                            lamports,
                            owner: Pubkey::new_from_array(owner.try_into()?),
                            executable,
                            rent_epoch,
                            data: data.to_owned(),
                            write_version,
                            slot,
                            is_startup,
                        }
                    },
                };

                this.spawn(|this| async move {
                    this.producer.send(Message::AccountUpdate(update)).await;
                    this.metrics.acct_sends.log(1);

                    Ok(())
                });

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
        self.with_inner(
            || GeyserPluginError::Custom(anyhow!(UNINIT).into()),
            |this| {
                if this.ins_sel.is_empty() {
                    return Ok(());
                }

                match transaction {
                    ReplicaTransactionInfoVersions::V0_0_1(tx) => {
                        if tx.transaction_status_meta.status.is_err() {
                            this.metrics.txn_errs.log(1);
                            return Ok(());
                        }

                        this.metrics.txn_recvs.log(1);

                        let msg = tx.transaction.message();
                        this.process_instructions(
                            msg.instructions()
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
                                ),
                            &msg.account_keys(),
                            slot,
                            tx.signature.as_ref(),
                        );
                    },
                    ReplicaTransactionInfoVersions::V0_0_2(tx) => {
                        if tx.transaction_status_meta.status.is_err() {
                            this.metrics.txn_errs.log(1);
                            return Ok(());
                        }

                        this.metrics.txn_recvs.log(1);

                        let msg = tx.transaction.message();
                        this.process_instructions(
                            msg.instructions()
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
                                ),
                            &msg.account_keys(),
                            slot,
                            tx.signature.as_ref(),
                        );
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
