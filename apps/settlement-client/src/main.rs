use std::{
    ffi::OsString,
    process::{Command, Stdio},
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::Deserialize;
use serde_json::Value;

const DEFAULT_API_URL: &str = "http://127.0.0.1:3000";
const SUBMIT_BATCH_SIGNATURE: &str = "submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)";
const FINALIZE_BATCH_SIGNATURE: &str = "finalizeBatch(uint256)";
const DEPOSIT_BOND_SIGNATURE: &str = "depositSequencerBond()";
const SET_SEQUENCER_SIGNATURE: &str = "setSequencer(address)";
const DRY_RUN_RPC_URL: &str = "http://127.0.0.1:8545";
const DRY_RUN_CONTRACT: &str = "0x0000000000000000000000000000000000000000";
const DRY_RUN_PRIVATE_KEY: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000001";

#[derive(Debug, Parser)]
#[command(name = "settlement-client")]
#[command(about = "Submit and finalize local rollup batches on an EVM settlement contract")]
struct Cli {
    #[arg(long, env = "SETTLEMENT_API_URL", default_value = DEFAULT_API_URL)]
    api_url: String,

    #[arg(long, env = "EVM_RPC_URL")]
    rpc_url: Option<String>,

    #[arg(long, env = "ROLLUP_SETTLEMENT_CONTRACT")]
    contract: Option<String>,

    #[arg(long, env = "EVM_PRIVATE_KEY")]
    private_key: Option<String>,

    #[arg(long, env = "CAST_BIN", default_value = "cast")]
    cast_bin: String,

    #[arg(long, global = true)]
    dry_run: bool,

    #[command(subcommand)]
    command: SettlementCommand,
}

#[derive(Debug, Subcommand)]
enum SettlementCommand {
    #[command(about = "Fetch API calldata and call RollupSettlement.submitBatch")]
    SubmitBatch {
        #[arg(long)]
        batch_number: u64,
    },
    #[command(about = "Call RollupSettlement.finalizeBatch after the challenge window")]
    FinalizeBatch {
        #[arg(long)]
        batch_number: u64,
    },
    #[command(about = "Deposit ETH into msg.sender's sequencer bond")]
    DepositBond {
        #[arg(
            long,
            help = "Value accepted by cast, for example 10ether or 10000000000000000000"
        )]
        value: String,
    },
    #[command(about = "Set the contract sequencer address; owner key required")]
    SetSequencer {
        #[arg(long)]
        sequencer: String,
    },
}

#[derive(Debug, Deserialize)]
struct SettlementCalldata {
    signature: String,
    ordered_args: Vec<Value>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let rpc_url =
        required_or_dry_run_default(&cli.rpc_url, cli.dry_run, "rpc URL", DRY_RUN_RPC_URL)?;
    let contract = required_or_dry_run_default(
        &cli.contract,
        cli.dry_run,
        "contract address",
        DRY_RUN_CONTRACT,
    )?;
    let private_key = required_or_dry_run_default(
        &cli.private_key,
        cli.dry_run,
        "private key",
        DRY_RUN_PRIVATE_KEY,
    )?;
    validate_hex_address(&contract, "contract address")?;
    validate_private_key(&private_key)?;

    let command = match &cli.command {
        SettlementCommand::SubmitBatch { batch_number } => {
            let calldata = fetch_settlement_calldata(&cli.api_url, *batch_number).await?;
            if calldata.signature != SUBMIT_BATCH_SIGNATURE {
                bail!(
                    "API returned unsupported settlement signature: {}",
                    calldata.signature
                );
            }
            if calldata.ordered_args.len() != 6 {
                bail!(
                    "API returned {} submitBatch args, expected 6",
                    calldata.ordered_args.len()
                );
            }

            let mut args = vec![contract.clone(), SUBMIT_BATCH_SIGNATURE.to_string()];
            args.extend(calldata.ordered_args.iter().map(cast_arg));
            CastCommand {
                action: "send",
                args,
                value: None,
            }
        }
        SettlementCommand::FinalizeBatch { batch_number } => CastCommand {
            action: "send",
            args: vec![
                contract.clone(),
                FINALIZE_BATCH_SIGNATURE.to_string(),
                batch_number.to_string(),
            ],
            value: None,
        },
        SettlementCommand::DepositBond { value } => CastCommand {
            action: "send",
            args: vec![contract.clone(), DEPOSIT_BOND_SIGNATURE.to_string()],
            value: Some(value.clone()),
        },
        SettlementCommand::SetSequencer { sequencer } => {
            validate_hex_address(&sequencer, "sequencer address")?;
            CastCommand {
                action: "send",
                args: vec![
                    contract.clone(),
                    SET_SEQUENCER_SIGNATURE.to_string(),
                    sequencer.clone(),
                ],
                value: None,
            }
        }
    };

    run_cast(&cli, &rpc_url, &private_key, command)
}

async fn fetch_settlement_calldata(api_url: &str, batch_number: u64) -> Result<SettlementCalldata> {
    let api_url = api_url.trim_end_matches('/');
    let url = format!("{api_url}/rollup/batches/{batch_number}/settlement-calldata");
    let response = reqwest::get(&url)
        .await
        .with_context(|| format!("failed to request settlement calldata from {url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("settlement calldata request failed with {status}: {body}");
    }

    response
        .json::<SettlementCalldata>()
        .await
        .with_context(|| format!("failed to parse settlement calldata from {url}"))
}

#[derive(Debug)]
struct CastCommand {
    action: &'static str,
    args: Vec<String>,
    value: Option<String>,
}

fn run_cast(cli: &Cli, rpc_url: &str, private_key: &str, command: CastCommand) -> Result<()> {
    let mut args = vec![
        OsString::from(command.action),
        OsString::from("--rpc-url"),
        OsString::from(rpc_url),
        OsString::from("--private-key"),
        OsString::from(private_key),
    ];

    if let Some(value) = command.value {
        args.push(OsString::from("--value"));
        args.push(OsString::from(value));
    }

    args.extend(command.args.into_iter().map(OsString::from));

    if cli.dry_run {
        println!("{}", printable_command(&cli.cast_bin, &args));
        return Ok(());
    }

    let status = Command::new(&cli.cast_bin)
        .args(&args)
        .stdin(Stdio::null())
        .status()
        .with_context(|| format!("failed to run {}", cli.cast_bin))?;

    if !status.success() {
        bail!("cast exited with status {status}");
    }
    Ok(())
}

fn cast_arg(value: &Value) -> String {
    match value {
        Value::String(value) => value.clone(),
        Value::Number(value) => value.to_string(),
        Value::Bool(value) => value.to_string(),
        other => other.to_string(),
    }
}

fn printable_command(bin: &str, args: &[OsString]) -> String {
    let mut parts = Vec::with_capacity(args.len() + 1);
    parts.push(shell_quote(bin));
    parts.extend(args.iter().map(|arg| {
        let value = arg.to_string_lossy();
        if value.starts_with("0x") && value.len() > 24 {
            shell_quote(&format!("{}...{}", &value[..10], &value[value.len() - 6..]))
        } else {
            shell_quote(&value)
        }
    }));
    parts.join(" ")
}

fn shell_quote(value: &str) -> String {
    if value
        .bytes()
        .all(|byte| byte.is_ascii_alphanumeric() || b"-_./:=+".contains(&byte))
    {
        value.to_string()
    } else {
        format!("'{}'", value.replace('\'', "'\\''"))
    }
}

fn validate_private_key(private_key: &str) -> Result<()> {
    let hex = private_key.strip_prefix("0x").unwrap_or(private_key);
    if hex.len() != 64 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("private key must be a 32-byte hex string");
    }
    Ok(())
}

fn required_or_dry_run_default(
    value: &Option<String>,
    dry_run: bool,
    label: &str,
    default: &str,
) -> Result<String> {
    if let Some(value) = value {
        return Ok(value.clone());
    }
    if dry_run {
        return Ok(default.to_string());
    }

    bail!("missing {label}; pass the CLI flag or set the matching environment variable")
}

fn validate_hex_address(address: &str, label: &str) -> Result<()> {
    let hex = address.strip_prefix("0x").unwrap_or(address);
    if hex.len() != 40 || !hex.bytes().all(|byte| byte.is_ascii_hexdigit()) {
        bail!("{label} must be a 20-byte hex address");
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn cast_arg_formats_api_values() {
        assert_eq!(cast_arg(&json!(7)), "7");
        assert_eq!(cast_arg(&json!("0xabc")), "0xabc");
    }

    #[test]
    fn validates_addresses() {
        assert!(validate_hex_address(&format!("0x{}", "a".repeat(40)), "address").is_ok());
        assert!(validate_hex_address("abc", "address").is_err());
    }

    #[test]
    fn dry_run_output_redacts_long_hex_values() {
        let args = vec![
            OsString::from("--private-key"),
            OsString::from(format!("0x{}", "a".repeat(64))),
        ];

        let command = printable_command("cast", &args);
        assert!(command.contains("0xaaaaaaaa...aaaaaa"));
        assert!(!command.contains(&"a".repeat(64)));
    }

    #[test]
    fn dry_run_allows_missing_connection_values() {
        assert_eq!(
            required_or_dry_run_default(&None, true, "rpc URL", DRY_RUN_RPC_URL).unwrap(),
            DRY_RUN_RPC_URL
        );
        assert!(required_or_dry_run_default(&None, false, "rpc URL", DRY_RUN_RPC_URL).is_err());
    }
}
