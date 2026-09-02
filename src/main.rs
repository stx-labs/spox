use std::path::PathBuf;

use bitcoin::Address;
use clap::{Parser, Subcommand, ValueEnum};
use spox::bitcoin::chain_tip::BitcoinChainTipPoller;
use spox::bitcoin::wallet::load_or_create_wallet;
use spox::config::Settings;
use spox::context::Context;
use spox::deposit_monitor::process_monitored_deposits;
use spox::dispatch::run_on_chain_tips;
use spox::error::Error;
use spox::reward_claim_process::process_reward_claims;
use spox::stacks::node::StacksClient;
use spox::storage::Storage as _;
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

fn setup_wallet(context: &Context) -> Result<(), Error> {
    let Some(ref wallet) = context.settings().node_wallet else {
        return Ok(());
    };

    load_or_create_wallet(context.bitcoin_client(), &wallet.name)
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

    setup_wallet(&context)?;

    let bitcoin_rpc = context.bitcoin_client().clone();
    let chain_tip_poller = BitcoinChainTipPoller::start(bitcoin_rpc, config.polling_interval).await;
    let rx1 = chain_tip_poller.new_receiver();
    let rx2 = chain_tip_poller.new_receiver();

    tokio::join!(
        run_on_chain_tips(process_monitored_deposits, rx1, context.clone()),
        run_on_chain_tips(process_reward_claims, rx2, context),
    );

    Ok(())
}
