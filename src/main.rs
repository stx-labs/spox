use std::path::PathBuf;
use std::time::Duration;

use bitcoin::Address;
use clap::{Parser, Subcommand, ValueEnum};
use emily_client::apis::deposit_api;
use spox::bitcoin::BlockRef;
use spox::config::Settings;
use spox::context::Context;
use spox::deposit_monitor::DepositMonitor;
use spox::error::Error;
use spox::stacks::node::StacksClient;
use spox::stacks::registry::GET_ADDRESSES_MAX_IDS;
use spox::storage::Storage;
use spox::storage::model::{MonitoredDeposit, MonitoredDepositSource};

#[derive(Debug, Clone, Copy, ValueEnum)]
enum LogOutputFormat {
    Json,
    Pretty,
}

#[derive(Debug, Clone, Parser)]
struct GetDepositAddressArgs {
    #[clap(short = 'n', long = "network", default_value = "bitcoin")]
    pub network: bitcoin::Network,
}

#[derive(Debug, Clone, Parser)]
struct GetRegistryAddressArgs {
    #[clap(short = 'n', long = "network", default_value = "bitcoin")]
    pub network: bitcoin::Network,
    pub id: u64,
}

#[derive(Debug, Subcommand)]
#[allow(clippy::enum_variant_names)]
enum CliCommand {
    GetSignersXonlyKey,
    GetDepositAddress(GetDepositAddressArgs),
    GetRegistryAddress(GetRegistryAddressArgs),
}

/// Command line arguments
#[derive(Debug, Parser)]
#[clap(name = "sPoX")]
struct Args {
    #[command(subcommand)]
    command: Option<CliCommand>,

    /// Optional path to the configuration file. If not provided, it is expected
    /// that all required parameters are provided via environment variables.
    #[clap(short = 'c', long, required = false)]
    config: Option<PathBuf>,

    #[clap(short = 'o', long = "output-format", default_value = "pretty")]
    output_format: LogOutputFormat,
}

async fn fetch_and_create_deposits(
    context: &Context,
    deposit_monitor: &mut DepositMonitor,
    chain_tip: &BlockRef,
) -> Result<(), Error> {
    let emily_config = context.emily_config();

    let deposits = deposit_monitor.get_pending_deposits(chain_tip)?;

    tracing::debug!(count = deposits.len(), "fetched pending deposits");
    if deposits.is_empty() {
        return Ok(());
    }

    for deposit in deposits {
        if let Err(error) = deposit_api::create_deposit(emily_config, deposit.clone()).await {
            tracing::warn!(
                %error,
                txid = %deposit.bitcoin_txid,
                vout = %deposit.bitcoin_tx_output_index,
                "cannot create deposit in emily"
            );
        } else {
            tracing::info!(
                txid = %deposit.bitcoin_txid,
                vout = %deposit.bitcoin_tx_output_index,
                "created deposit in emily"
            );
            deposit_monitor.deposit_created(&deposit.bitcoin_txid, deposit.bitcoin_tx_output_index);
        }
    }

    Ok(())
}

async fn update_from_registry(context: &Context) -> Result<(), Error> {
    let Some(registry) = context.registry() else {
        return Ok(());
    };
    let store = context.storage();

    let last_next_id = store.get_last_next_address_id()?;
    let registry_next_id = registry.get_next_address_id().await?;

    if last_next_id >= registry_next_id {
        return Ok(());
    }

    // TODO: limit the amount of work per single function invocation
    for start in (last_next_id..registry_next_id).step_by(GET_ADDRESSES_MAX_IDS as usize) {
        let end = start
            .saturating_add(GET_ADDRESSES_MAX_IDS as u64)
            .min(registry_next_id);
        let ids: Vec<u64> = (start..end).collect();
        let deposit_addresses = registry.get_addresses(&ids).await?;

        let deposit_addresses_ids: Vec<u64> = deposit_addresses.iter().map(|v| v.id).collect();
        if ids != deposit_addresses_ids {
            return Err(Error::MismatchingRawAddressIds);
        }

        for raw_deposit_address in deposit_addresses {
            let address_id = raw_deposit_address.id;
            match raw_deposit_address.try_into() {
                Ok(deposit_address) => {
                    store.add(deposit_address)?;
                    tracing::info!(address_id, "added new address from registry");
                }
                Err(Error::MissingAddressScripts) => {
                    tracing::debug!(address_id, "skipping empty address id");
                }
                Err(error) => tracing::warn!(
                    %error,
                    address_id,
                    "cannot parse deposit address"
                ),
            };
            store.set_last_next_address_id(address_id.saturating_add(1))?;
        }
    }

    Ok(())
}

async fn runloop(
    context: Context,
    deposit_monitor: &mut DepositMonitor,
    polling_interval: Duration,
) {
    let bitcoin_client = context.bitcoin_client();
    let mut first_poll = true;
    let mut last_chain_tip = None;

    loop {
        if first_poll {
            first_poll = false;
        } else {
            tokio::time::sleep(polling_interval).await;
        }

        let chain_tip = match bitcoin_client.get_chain_tip() {
            Ok(chain_tip) => chain_tip,
            Err(error) => {
                tracing::warn!(
                    %error,
                    "error getting the chain tip"
                );
                continue;
            }
        };

        let is_last_chaintip = last_chain_tip
            .as_ref()
            .is_some_and(|last| last == &chain_tip);

        if is_last_chaintip {
            continue;
        }

        tracing::debug!(%chain_tip, "new block; processing pending deposits");

        let _ = update_from_registry(&context).await.inspect_err(|error| {
            tracing::warn!(
                %error,
                "error updating from address registry"
            )
        });

        match fetch_and_create_deposits(&context, deposit_monitor, &chain_tip).await {
            Ok(()) => {
                last_chain_tip = Some(chain_tip);
            }
            Err(Error::ScanTxOutInProgress) => {
                tracing::info!(
                    "skipping deposit processing: a scantxoutset is already running on bitcoind; will retry next poll"
                );
                // Do not advance `last_chain_tip`: retry the same tip next iteration.
            }
            Err(error) => {
                tracing::warn!(%error, "error processing pending deposits");
                last_chain_tip = Some(chain_tip);
            }
        }
    }
}

async fn get_signers_xonly_key(config: &Settings) -> Result<(), Box<dyn std::error::Error>> {
    let stacks_client = StacksClient::try_from(config)?;
    let sbtc_deployer = &config
        .stacks
        .as_ref()
        .ok_or_else(|| Error::MissingStacksConfig)?
        .sbtc_deployer;

    let signers_aggregate_key = stacks_client
        .get_current_signers_aggregate_key(sbtc_deployer)
        .await?;

    match signers_aggregate_key {
        Some(public_key) => println!("{public_key}"),
        None => return Err(Box::new(Error::NoSignersAggregateKey)),
    }

    Ok(())
}

async fn get_deposit_address(
    monitored: &[MonitoredDeposit],
    args: &GetDepositAddressArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    for deposit in monitored {
        let address = Address::from_script(&deposit.to_script_pubkey(), args.network)?;
        match &deposit.source {
            MonitoredDepositSource::Config(alias) => println!("{alias}: {address}"),
            MonitoredDepositSource::Registry(id) => println!("id={id}: {address}"),
        }
    }
    Ok(())
}

async fn get_registry_address(
    context: &Context,
    args: &GetRegistryAddressArgs,
) -> Result<(), Box<dyn std::error::Error>> {
    let raw_address = context
        .registry()
        .ok_or(Error::NoRegistryConfigured)?
        .get_addresses(&[args.id])
        .await?
        .into_iter()
        .next()
        .ok_or(Error::MismatchingRawAddressIds)?;

    let deposit: MonitoredDeposit = raw_address.try_into()?;
    let address = Address::from_script(&deposit.to_script_pubkey(), args.network)?;
    println!("{address}");
    Ok(())
}

#[tokio::main]
#[tracing::instrument(name = "spox")]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Parse the command line arguments.
    let args = Args::parse();

    // Configure the binary's stdout/err output based on the provided output format.
    let pretty = matches!(args.output_format, LogOutputFormat::Pretty);
    spox::logging::setup_logging("info,spox=debug", pretty);

    // Load the configuration file and/or environment variables.
    let config = Settings::new(args.config).inspect_err(|error| {
        tracing::error!(%error, "failed to construct the configuration");
    })?;

    let monitored = config
        .deposit
        .iter()
        .map(TryInto::try_into)
        .collect::<Result<Vec<_>, Error>>()?;

    let context = Context::try_from(&config)?;

    match args.command {
        Some(CliCommand::GetSignersXonlyKey) => return get_signers_xonly_key(&config).await,
        Some(CliCommand::GetDepositAddress(args)) => {
            return get_deposit_address(&monitored, &args).await;
        }
        Some(CliCommand::GetRegistryAddress(args)) => {
            return get_registry_address(&context, &args).await;
        }
        None => (),
    }

    let store = context.storage();
    for monitored_deposit in monitored {
        store.add(monitored_deposit)?;
    }

    let mut deposit_monitor = DepositMonitor::new(context.clone());

    runloop(context, &mut deposit_monitor, config.polling_interval).await;

    Ok(())
}
