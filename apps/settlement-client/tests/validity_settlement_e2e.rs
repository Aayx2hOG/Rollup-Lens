//! Validity-rollup settlement integration tests.
//!
//! Both tiers require Postgres, Anvil, and Forge. The fast tier uses a
//! deterministic verifier mock. The real tier additionally generates and
//! verifies an SP1 Groth16 proof.

use std::{
    env, fs,
    future::IntoFuture,
    net::TcpListener,
    path::{Path, PathBuf},
    process::{Child, Command, Output, Stdio},
    time::{SystemTime, UNIX_EPOCH},
};

use chain_core::{sign_transfer, Account};
use explorer_api::{app_with_pool_and_tokens_and_deposit_inbox, initialize_genesis, GenesisConfig};
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use wallet_core::create_keypair;

const ANVIL_PRIVATE_KEY: &str =
    "0xac0974bec39a17e36ba4a6b4d238ff944bacb478cbed5efcae784d7bf4f2ff80";
const ANVIL_ADDRESS: &str = "0xf39fd6e51aad88f6f4ce6ab8827279cfffb92266";
const MOCK_PROGRAM_VKEY: &str =
    "0x0000000000000000000000000000000000000000000000000000000000000007";
const MOCK_PROOF: &str = "0x70726f6f66";
const L1_DEPOSIT_INBOX: &str = "0x2222222222222222222222222222222222222222";

const MIGRATIONS: &[&str] = &[
    include_str!("../../../prisma/migrations/0001_init/migration.sql"),
    include_str!("../../../prisma/migrations/0002_error_events/migration.sql"),
    include_str!("../../../prisma/migrations/0003_rollup_batches/migration.sql"),
    include_str!("../../../prisma/migrations/0004_transaction_pool/migration.sql"),
    include_str!("../../../prisma/migrations/0005_l1_settlement_status/migration.sql"),
    include_str!("../../../prisma/migrations/0006_l1_receipt_metadata/migration.sql"),
    include_str!("../../../prisma/migrations/0007_validity_witness/migration.sql"),
    include_str!("../../../prisma/migrations/0008_operator_and_l1_indexer/migration.sql"),
    include_str!("../../../prisma/migrations/0009_eip712_ethereum_accounts/migration.sql"),
    include_str!("../../../prisma/migrations/0010_l1_deposits/migration.sql"),
    include_str!("../../../prisma/migrations/0011_deterministic_deposit_execution/migration.sql"),
];

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TEST_DATABASE_URL, Anvil, and Forge"]
async fn sequences_and_finalizes_a_batch_with_mock_verifier() {
    run_scenario(VerifierMode::Mock).await;
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
#[ignore = "requires TEST_DATABASE_URL, Anvil, Forge, and an SP1 Groth16 prover"]
async fn proves_sequences_and_finalizes_a_batch_on_anvil() {
    run_scenario(VerifierMode::RealSp1).await;
}

#[derive(Clone, Copy)]
enum VerifierMode {
    Mock,
    RealSp1,
}

async fn run_scenario(verifier_mode: VerifierMode) {
    let database_url = env::var("TEST_DATABASE_URL")
        .expect("set TEST_DATABASE_URL to a disposable Postgres database");
    let prover = match verifier_mode {
        VerifierMode::Mock => None,
        VerifierMode::RealSp1 => {
            let path = env::var_os("VALIDITY_PROVER_BIN")
                .map(PathBuf::from)
                .expect("set VALIDITY_PROVER_BIN to a release build of validity-prover");
            assert!(
                path.is_file(),
                "VALIDITY_PROVER_BIN does not point to a file: {}",
                path.display()
            );
            Some(path)
        }
    };
    require_tool("anvil");
    require_tool("forge");
    require_tool("cast");

    let workspace = workspace_root();
    let temp = tempfile::tempdir().expect("create test directory");
    let (pool, schema) = isolated_database(&database_url).await;

    let alice = create_keypair();
    let bob = create_keypair();
    let alice_address = alice.address.clone();
    let bob_address = bob.address.clone();
    let genesis = initialize_genesis(
        &pool,
        GenesisConfig {
            accounts: vec![
                Account {
                    address: alice_address,
                    balance: 1_000,
                    nonce: 0,
                },
                Account {
                    address: bob_address.clone(),
                    balance: 0,
                    nonce: 0,
                },
            ],
        },
    )
    .await
    .expect("initialize genesis");

    let api_listener = tokio::net::TcpListener::bind("127.0.0.1:0")
        .await
        .expect("bind API");
    let api_url = format!("http://{}", api_listener.local_addr().unwrap());
    let api_task = tokio::spawn(
        axum::serve(
            api_listener,
            app_with_pool_and_tokens_and_deposit_inbox(
                pool.clone(),
                None,
                None,
                L1_DEPOSIT_INBOX.to_string(),
            ),
        )
        .into_future(),
    );

    let client = reqwest::Client::new();
    let transfer = sign_transfer(&alice.secret_key_hex, bob_address, 25, 0).unwrap();
    post_ok(
        &client,
        &format!("{api_url}/rollup/transactions"),
        &json!(transfer),
    )
    .await;
    post_ok(&client, &format!("{api_url}/rollup/batches"), &json!({})).await;

    let witness_path = temp.path().join("witness.json");
    let witness = get_ok(
        &client,
        &format!("{api_url}/rollup/batches/0/validity-witness"),
    )
    .await;
    fs::write(&witness_path, serde_json::to_vec_pretty(&witness).unwrap()).unwrap();

    let artifact_path = temp.path().join("proof.json");
    match prover {
        Some(prover) => {
            run(
                Command::new(&prover)
                    .arg("groth16")
                    .arg(&witness_path)
                    .arg("--evm-artifact-out")
                    .arg(&artifact_path),
                "generate SP1 Groth16 proof",
            );
        }
        None => {
            let calldata = get_ok(
                &client,
                &format!("{api_url}/rollup/batches/0/settlement-calldata"),
            )
            .await;
            let batch = get_ok(&client, &format!("{api_url}/rollup/batches/0")).await;
            let artifact = mock_proof_artifact(&calldata, &batch);
            fs::write(
                &artifact_path,
                serde_json::to_vec_pretty(&artifact).unwrap(),
            )
            .unwrap();
        }
    }
    let artifact: Value =
        serde_json::from_slice(&fs::read(&artifact_path).unwrap()).expect("parse proof artifact");
    let decoded = &artifact["decoded"];
    assert_eq!(decoded["batch_number"], json!(0));
    assert_eq!(
        strip_hex(decoded["prev_state_root"].as_str().unwrap()),
        genesis.state_root
    );

    let anvil_port = unused_port();
    let rpc_url = format!("http://127.0.0.1:{anvil_port}");
    let anvil = ManagedChild::spawn(
        Command::new("anvil")
            .arg("--port")
            .arg(anvil_port.to_string())
            .arg("--silent"),
        "start Anvil",
    );
    wait_for_rpc(&client, &rpc_url).await;

    let program_vkey = artifact["program_vkey"].as_str().unwrap();
    let verifier = match verifier_mode {
        VerifierMode::Mock => forge_create(
            &workspace,
            &rpc_url,
            "contracts/mocks/ValidityVerifierMock.sol:ValidityVerifierMock",
            &[program_vkey],
        ),
        VerifierMode::RealSp1 => forge_create(
            &workspace,
            &rpc_url,
            "lib/sp1-contracts/contracts/src/v6.1.0/SP1VerifierGroth16.sol:SP1Verifier",
            &[],
        ),
    };
    let genesis_root = format!("0x{}", genesis.state_root);
    let settlement = forge_create(
        &workspace,
        &rpc_url,
        "contracts/ValidityRollupSettlement.sol:ValidityRollupSettlement",
        &[&genesis_root, &verifier, program_vkey, L1_DEPOSIT_INBOX],
    );

    settlement_client(
        &api_url,
        &rpc_url,
        &settlement,
        &["set-sequencer", "--sequencer", ANVIL_ADDRESS],
    );
    settlement_client(
        &api_url,
        &rpc_url,
        &settlement,
        &[
            "submit-batch",
            "--batch-number",
            "0",
            "--proof",
            artifact_path.to_str().unwrap(),
        ],
    );

    let finalized_root = cast_call(&rpc_url, &settlement, "finalizedStateRoot()(bytes32)");
    assert_eq!(
        finalized_root.to_ascii_lowercase(),
        decoded["new_state_root"]
            .as_str()
            .unwrap()
            .to_ascii_lowercase()
    );
    assert_eq!(
        cast_call(&rpc_url, &settlement, "nextBatchNumber()(uint256)"),
        "1"
    );

    let batch = get_ok(&client, &format!("{api_url}/rollup/batches/0")).await;
    assert_eq!(batch["status"], json!("finalized"));
    assert!(batch["l1_submit_tx_hash"]
        .as_str()
        .unwrap()
        .starts_with("0x"));
    assert!(batch["l1_submit_receipt"].is_object());
    assert_eq!(batch["l1_chain_id"], json!(31337));

    api_task.abort();
    let _ = api_task.await;
    drop(anvil);
    drop_schema(&pool, &schema).await;
}

fn mock_proof_artifact(calldata: &Value, batch: &Value) -> Value {
    let args = calldata["ordered_args"]
        .as_array()
        .expect("settlement calldata args");
    assert_eq!(args.len(), 6);

    let mut public_values = String::from("0x");
    public_values.push_str(&abi_u256(args[0].as_u64().expect("batch number")));
    for root in &args[1..] {
        let root = root.as_str().expect("bytes32 root");
        let root = strip_hex(root);
        assert_eq!(root.len(), 64, "root must be bytes32");
        public_values.push_str(root);
    }
    public_values.push_str(strip_hex(
        batch["deposit_root"].as_str().expect("deposit root"),
    ));
    public_values.push_str(&abi_u256(batch["first_deposit_id"].as_u64().unwrap_or(0)));
    public_values.push_str(&abi_u256(batch["last_deposit_id"].as_u64().unwrap_or(0)));
    public_values.push_str(&abi_u256(
        batch["deposit_count"].as_u64().expect("deposit count"),
    ));
    public_values.push_str(&abi_u256(
        batch["transaction_count"]
            .as_u64()
            .expect("transaction count"),
    ));
    public_values.push_str(&abi_address(L1_DEPOSIT_INBOX));

    json!({
        "program_vkey": MOCK_PROGRAM_VKEY,
        "public_values": public_values,
        "proof": MOCK_PROOF,
        "decoded": {
            "batch_number": args[0],
            "prev_state_root": args[1],
            "new_state_root": args[2],
            "transaction_root": args[3],
            "block_hash_root": args[4],
            "data_hash": args[5],
            "deposit_root": batch["deposit_root"],
            "first_deposit_id": batch["first_deposit_id"],
            "last_deposit_id": batch["last_deposit_id"],
            "deposit_count": batch["deposit_count"],
            "transaction_count": batch["transaction_count"],
            "l1_deposit_inbox": L1_DEPOSIT_INBOX
        }
    })
}

fn abi_address(value: &str) -> String {
    let address = strip_hex(value);
    assert_eq!(address.len(), 40, "address must be 20 bytes");
    format!("{}{}", "0".repeat(24), address)
}

fn abi_u256(value: u64) -> String {
    format!("{value:064x}")
}

fn settlement_client(api_url: &str, rpc_url: &str, contract: &str, args: &[&str]) {
    run(
        Command::new(env!("CARGO_BIN_EXE_settlement-client"))
            .arg("--api-url")
            .arg(api_url)
            .arg("--rpc-url")
            .arg(rpc_url)
            .arg("--contract")
            .arg(contract)
            .arg("--private-key")
            .arg(ANVIL_PRIVATE_KEY)
            .args(args),
        "run settlement client",
    );
}

fn forge_create(
    workspace: &Path,
    rpc_url: &str,
    contract: &str,
    constructor_args: &[&str],
) -> String {
    let mut command = Command::new("forge");
    command
        .current_dir(workspace)
        .args([
            "create",
            contract,
            "--broadcast",
            "--json",
            "--rpc-url",
            rpc_url,
        ])
        .args(["--private-key", ANVIL_PRIVATE_KEY]);
    if !constructor_args.is_empty() {
        command.arg("--constructor-args").args(constructor_args);
    }
    let output = run(&mut command, "deploy contract");
    let value: Value = serde_json::from_slice(&output.stdout).expect("parse forge create JSON");
    value
        .get("deployedTo")
        .or_else(|| value.get("deployed_to"))
        .and_then(Value::as_str)
        .expect("forge output should contain deployed contract address")
        .to_owned()
}

fn cast_call(rpc_url: &str, contract: &str, signature: &str) -> String {
    let output = run(
        Command::new("cast").args(["call", contract, signature, "--rpc-url", rpc_url]),
        "read settlement contract",
    );
    String::from_utf8(output.stdout).unwrap().trim().to_owned()
}

async fn post_ok(client: &reqwest::Client, url: &str, body: &Value) -> Value {
    let response = client
        .post(url)
        .json(body)
        .send()
        .await
        .expect("POST request");
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(
        status.is_success(),
        "POST {url} failed ({status}): {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).expect("POST response JSON")
}

async fn get_ok(client: &reqwest::Client, url: &str) -> Value {
    let response = client.get(url).send().await.expect("GET request");
    let status = response.status();
    let bytes = response.bytes().await.unwrap();
    assert!(
        status.is_success(),
        "GET {url} failed ({status}): {}",
        String::from_utf8_lossy(&bytes)
    );
    serde_json::from_slice(&bytes).expect("GET response JSON")
}

async fn isolated_database(database_url: &str) -> (PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .expect("connect to Postgres");
    let schema = format!(
        "validity_settlement_e2e_{}",
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos()
    );
    pool.execute(format!(r#"CREATE SCHEMA "{schema}""#).as_str())
        .await
        .expect("create isolated schema");
    pool.execute(format!(r#"SET search_path TO "{schema}""#).as_str())
        .await
        .expect("select isolated schema");
    for migration in MIGRATIONS {
        for statement in migration
            .split(';')
            .map(str::trim)
            .filter(|s| !s.is_empty())
        {
            pool.execute(statement).await.expect("apply migration");
        }
    }
    (pool, schema)
}

async fn drop_schema(pool: &PgPool, schema: &str) {
    let _ = pool
        .execute(format!(r#"DROP SCHEMA IF EXISTS "{schema}" CASCADE"#).as_str())
        .await;
}

async fn wait_for_rpc(client: &reqwest::Client, rpc_url: &str) {
    for _ in 0..100 {
        if client
            .post(rpc_url)
            .json(&json!({"jsonrpc":"2.0","id":1,"method":"eth_chainId","params":[]}))
            .send()
            .await
            .is_ok_and(|response| response.status().is_success())
        {
            return;
        }
        tokio::time::sleep(std::time::Duration::from_millis(50)).await;
    }
    panic!("Anvil did not become ready at {rpc_url}");
}

fn require_tool(tool: &str) {
    let status = Command::new(tool)
        .arg("--version")
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .unwrap_or_else(|error| panic!("{tool} is required: {error}"));
    assert!(status.success(), "{tool} --version failed");
}

fn run(command: &mut Command, action: &str) -> Output {
    let output = command
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .output()
        .unwrap_or_else(|error| panic!("failed to {action}: {error}"));
    assert!(
        output.status.success(),
        "failed to {action} ({})\nstdout:\n{}\nstderr:\n{}",
        output.status,
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    output
}

fn workspace_root() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .ancestors()
        .nth(2)
        .unwrap()
        .to_path_buf()
}

fn unused_port() -> u16 {
    TcpListener::bind("127.0.0.1:0")
        .unwrap()
        .local_addr()
        .unwrap()
        .port()
}

fn strip_hex(value: &str) -> &str {
    value.strip_prefix("0x").unwrap_or(value)
}

struct ManagedChild(Option<Child>);

impl ManagedChild {
    fn spawn(command: &mut Command, action: &str) -> Self {
        let child = command
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap_or_else(|error| panic!("failed to {action}: {error}"));
        Self(Some(child))
    }
}

impl Drop for ManagedChild {
    fn drop(&mut self) {
        if let Some(child) = &mut self.0 {
            let _ = child.kill();
            let _ = child.wait();
        }
    }
}
