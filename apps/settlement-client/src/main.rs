use std::{
    ffi::OsString,
    process::{Command, Stdio},
    time::Duration,
};

use anyhow::{bail, Context, Result};
use clap::{Parser, Subcommand};
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};

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

#[derive(Debug, Serialize)]
struct SettlementStatusUpdate<'a> {
    status: &'a str,
    tx_hash: &'a str,
    contract_address: &'a str,
    chain_id: u64,
    block_number: u64,
    gas_used: u64,
    receipt: &'a Value,
}

#[derive(Debug, Serialize)]
struct JsonRpcRequest<'a> {
    jsonrpc: &'static str,
    id: u64,
    method: &'a str,
    params: Value,
}

#[derive(Debug, Deserialize)]
struct JsonRpcResponse {
    result: Option<Value>,
    error: Option<JsonRpcError>,
}

#[derive(Debug, Deserialize)]
struct JsonRpcError {
    code: i64,
    message: String,
}

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct TransactionReceipt {
    transaction_hash: String,
    block_number: String,
    gas_used: String,
    status: String,
    to: Option<String>,
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
                persisted_status: Some(PersistedStatus {
                    batch_number: *batch_number,
                    status: "submitted",
                }),
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
            persisted_status: Some(PersistedStatus {
                batch_number: *batch_number,
                status: "finalized",
            }),
        },
        SettlementCommand::DepositBond { value } => CastCommand {
            action: "send",
            args: vec![contract.clone(), DEPOSIT_BOND_SIGNATURE.to_string()],
            value: Some(value.clone()),
            persisted_status: None,
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
                persisted_status: None,
            }
        }
    };

    let output = run_cast(&cli, &rpc_url, &private_key, command)?;
    if let Some(persisted) = output.persisted_status {
        let chain_id = fetch_chain_id(&rpc_url).await?;
        let (receipt, receipt_value) =
            fetch_verified_receipt(&rpc_url, &output.tx_hash, &contract).await?;
        let block_number = parse_hex_u64(&receipt.block_number, "receipt block number")?;
        let gas_used = parse_hex_u64(&receipt.gas_used, "receipt gas used")?;

        persist_settlement_status(
            &cli.api_url,
            persisted.batch_number,
            persisted.status,
            &output.tx_hash,
            &contract,
            chain_id,
            block_number,
            gas_used,
            &receipt_value,
        )
        .await?;
    }
    Ok(())
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

async fn fetch_chain_id(rpc_url: &str) -> Result<u64> {
    let chain_id: String = json_rpc_required(rpc_url, "eth_chainId", json!([]))
        .await
        .context("failed to fetch EVM chain id")?;
    parse_hex_u64(&chain_id, "EVM chain id")
}

async fn fetch_verified_receipt(
    rpc_url: &str,
    tx_hash: &str,
    contract_address: &str,
) -> Result<(TransactionReceipt, Value)> {
    let expected_hash = tx_hash.to_ascii_lowercase();
    let expected_contract = contract_address.to_ascii_lowercase();

    for attempt in 1..=30 {
        let receipt_value = json_rpc_value(rpc_url, "eth_getTransactionReceipt", json!([tx_hash]))
            .await
            .with_context(|| format!("failed to fetch receipt for {tx_hash}"))?;

        if let Some(receipt_value) = receipt_value {
            let receipt: TransactionReceipt = serde_json::from_value(receipt_value.clone())
                .with_context(|| format!("failed to parse receipt for {tx_hash}"))?;
            if receipt.transaction_hash.to_ascii_lowercase() != expected_hash {
                bail!(
                    "receipt transaction hash {} did not match expected {}",
                    receipt.transaction_hash,
                    expected_hash
                );
            }
            if receipt.status != "0x1" {
                bail!(
                    "settlement transaction {tx_hash} failed with status {}",
                    receipt.status
                );
            }
            let Some(to) = &receipt.to else {
                bail!("receipt for {tx_hash} does not include a target contract address");
            };
            if to.to_ascii_lowercase() != expected_contract {
                bail!(
                    "receipt target {} did not match settlement contract {}",
                    to,
                    expected_contract
                );
            }

            return Ok((receipt, receipt_value));
        }

        if attempt < 30 {
            tokio::time::sleep(Duration::from_secs(1)).await;
        }
    }

    bail!("timed out waiting for receipt for {tx_hash}")
}

async fn json_rpc_required<T: serde::de::DeserializeOwned>(
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<T> {
    let value = json_rpc_value(rpc_url, method, params)
        .await?
        .ok_or_else(|| anyhow::anyhow!("JSON-RPC method {method} returned no result"))?;
    serde_json::from_value(value)
        .with_context(|| format!("failed to decode JSON-RPC result for {method}"))
}

async fn json_rpc_value(rpc_url: &str, method: &str, params: Value) -> Result<Option<Value>> {
    let response = reqwest::Client::new()
        .post(rpc_url)
        .json(&JsonRpcRequest {
            jsonrpc: "2.0",
            id: 1,
            method,
            params,
        })
        .send()
        .await
        .with_context(|| format!("failed to call JSON-RPC method {method} at {rpc_url}"))?;
    let status = response.status();
    if !status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("JSON-RPC method {method} failed with HTTP {status}: {body}");
    }

    let body = response
        .json::<JsonRpcResponse>()
        .await
        .with_context(|| format!("failed to parse JSON-RPC response for {method}"))?;
    if let Some(error) = body.error {
        bail!(
            "JSON-RPC method {method} returned error {}: {}",
            error.code,
            error.message
        );
    }
    Ok(body.result)
}

#[derive(Debug)]
struct CastCommand {
    action: &'static str,
    args: Vec<String>,
    value: Option<String>,
    persisted_status: Option<PersistedStatus>,
}

#[derive(Debug, Clone, Copy)]
struct PersistedStatus {
    batch_number: u64,
    status: &'static str,
}

#[derive(Debug)]
struct CastOutput {
    tx_hash: String,
    persisted_status: Option<PersistedStatus>,
}

fn run_cast(
    cli: &Cli,
    rpc_url: &str,
    private_key: &str,
    command: CastCommand,
) -> Result<CastOutput> {
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
        return Ok(CastOutput {
            tx_hash: String::new(),
            persisted_status: None,
        });
    }

    let output = Command::new(&cli.cast_bin)
        .args(&args)
        .stdin(Stdio::null())
        .output()
        .with_context(|| format!("failed to run {}", cli.cast_bin))?;

    print!("{}", String::from_utf8_lossy(&output.stdout));
    eprint!("{}", String::from_utf8_lossy(&output.stderr));

    if !output.status.success() {
        bail!("cast exited with status {}", output.status);
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    let tx_hash = extract_tx_hash(&stdout)
        .with_context(|| "cast succeeded but no transaction hash was found in stdout")?;
    Ok(CastOutput {
        tx_hash,
        persisted_status: command.persisted_status,
    })
}

async fn persist_settlement_status(
    api_url: &str,
    batch_number: u64,
    status: &str,
    tx_hash: &str,
    contract_address: &str,
    chain_id: u64,
    block_number: u64,
    gas_used: u64,
    receipt: &Value,
) -> Result<()> {
    let api_url = api_url.trim_end_matches('/');
    let url = format!("{api_url}/rollup/batches/{batch_number}/settlement");
    let response = reqwest::Client::new()
        .post(&url)
        .json(&SettlementStatusUpdate {
            status,
            tx_hash,
            contract_address,
            chain_id,
            block_number,
            gas_used,
            receipt,
        })
        .send()
        .await
        .with_context(|| format!("failed to persist settlement status to {url}"))?;
    let response_status = response.status();
    if !response_status.is_success() {
        let body = response.text().await.unwrap_or_default();
        bail!("settlement status persistence failed with {response_status}: {body}");
    }

    println!("persisted {status} settlement status for batch {batch_number}: {tx_hash}");
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

fn extract_tx_hash(output: &str) -> Option<String> {
    output
        .split(|character: char| {
            character.is_whitespace() || matches!(character, ':' | ',' | '"' | '\'')
        })
        .find_map(|part| {
            let hex = part.strip_prefix("0x")?;
            (hex.len() == 64 && hex.bytes().all(|byte| byte.is_ascii_hexdigit()))
                .then(|| format!("0x{}", hex.to_ascii_lowercase()))
        })
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

fn parse_hex_u64(value: &str, label: &str) -> Result<u64> {
    let Some(hex) = value.strip_prefix("0x") else {
        bail!("{label} must start with 0x");
    };
    u64::from_str_radix(hex, 16).with_context(|| format!("failed to parse {label}: {value}"))
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

    #[test]
    fn extracts_transaction_hash_from_cast_output() {
        let hash = format!("0x{}", "A".repeat(64));
        let output = format!("transactionHash {hash}\nblockNumber 1\n");

        assert_eq!(
            extract_tx_hash(&output),
            Some(format!("0x{}", "a".repeat(64)))
        );
    }
}
