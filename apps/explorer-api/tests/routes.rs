use std::{
    env,
    time::{SystemTime, UNIX_EPOCH},
};

use axum::{
    body::Body,
    http::{Request, Response, StatusCode},
};
use chain_core::{
    add_pending_transaction, finalize_genesis, new_chain, next_nonce, produce_block,
    rollup_batch_commitment, sign_transfer, transaction_hash, Account, Chain,
    RollupBatchCommitment,
};
use explorer_api::{app_with_pool, app_with_pool_and_tokens, initialize_genesis, GenesisConfig};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use sqlx::{postgres::PgPoolOptions, Executor, PgPool};
use tower::ServiceExt;
use wallet_core::create_keypair;

#[tokio::test]
async fn postgres_router_exposes_index_without_database_queries() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://postgres:postgres@localhost/zk_rollup")
        .unwrap();
    let app = app_with_pool(pool);

    let index = get_json(app, "/").await;
    assert_eq!(index["name"], json!("zk-rollup explorer api"));
}

#[tokio::test]
async fn readiness_reports_an_unavailable_database() {
    let pool = PgPoolOptions::new()
        .acquire_timeout(std::time::Duration::from_millis(100))
        .connect_lazy("postgresql://postgres:postgres@127.0.0.1:1/zk_rollup")
        .unwrap();
    let response = get_response(app_with_pool(pool), "/ready").await;

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
    let payload = response_json(response).await;
    assert_eq!(payload["status"], json!("not_ready"));
    assert_eq!(payload["database"], json!("unavailable"));
}

#[tokio::test]
async fn genesis_initialization_is_atomic_and_one_time() {
    let Some(database_url) = optional_test_database_url() else {
        eprintln!("skipping Postgres integration test; set TEST_DATABASE_URL or DATABASE_URL");
        return;
    };

    let alice = create_keypair();
    let bob = create_keypair();
    let accounts = vec![
        Account {
            address: alice.address.clone(),
            balance: 1_000,
            nonce: 0,
        },
        Account {
            address: bob.address.clone(),
            balance: 0,
            nonce: 0,
        },
    ];
    let mut expected_chain = new_chain(accounts.clone()).unwrap();
    let expected_genesis = finalize_genesis(&mut expected_chain).unwrap();
    let (pool, schema) = setup_postgres_test_database(&database_url).await;

    let initialized = initialize_genesis(
        &pool,
        GenesisConfig {
            accounts: accounts.clone(),
        },
    )
    .await
    .unwrap();

    assert_eq!(initialized.account_count, 2);
    assert_eq!(initialized.state_root, expected_genesis.state_root);
    assert_eq!(initialized.block_hash, expected_genesis.block_hash);
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM blocks")
            .fetch_one(&pool)
            .await
            .unwrap(),
        1
    );
    assert_eq!(
        sqlx::query_scalar::<_, i64>("SELECT COUNT(*) FROM accounts")
            .fetch_one(&pool)
            .await
            .unwrap(),
        2
    );

    let repeated = initialize_genesis(&pool, GenesisConfig { accounts })
        .await
        .unwrap_err();
    assert!(repeated.to_string().contains("requires empty"));

    teardown_postgres_test_database(&pool, &schema).await;
}

#[tokio::test]
async fn postgres_source_serves_indexed_chain_pages_and_details() {
    let Some(database_url) = optional_test_database_url() else {
        eprintln!("skipping Postgres integration test; set TEST_DATABASE_URL or DATABASE_URL");
        return;
    };

    let fixture = ChainFixture::with_transaction_blocks(3);
    let (pool, schema) = setup_postgres_test_database(&database_url).await;
    insert_indexed_chain(&pool, &fixture.chain).await;

    let app = app_with_pool(pool.clone());
    let summary = get_json(app.clone(), "/chain/summary").await;
    assert_eq!(summary["height"], json!(3));
    assert_eq!(summary["block_count"], json!(4));
    assert_eq!(summary["transaction_count"], json!(3));
    assert_eq!(summary["pending_transaction_count"], json!(0));
    assert_eq!(summary["account_count"], json!(2));
    assert_eq!(summary["valid"], json!(true));

    let blocks = get_json(app.clone(), "/blocks?limit=2&offset=1").await;
    assert_eq!(blocks["total"], json!(4));
    assert_eq!(blocks["limit"], json!(2));
    assert_eq!(blocks["offset"], json!(1));
    assert_eq!(blocks["next_offset"], json!(3));
    assert_eq!(blocks["items"].as_array().unwrap().len(), 2);
    assert_eq!(blocks["items"][0]["height"], json!(2));
    assert_eq!(blocks["items"][1]["height"], json!(1));

    let latest = get_json(
        app.clone(),
        &format!("/blocks/hash/{}", fixture.latest_block_hash),
    )
    .await;
    assert_eq!(latest["height"], json!(3));
    assert_eq!(latest["block_hash"], json!(fixture.latest_block_hash));

    let account = get_json(
        app.clone(),
        &format!("/accounts/{}", path_segment(&fixture.alice_address)),
    )
    .await;
    assert_eq!(account["address"], json!(fixture.alice_address));

    let transaction = get_json(
        app.clone(),
        &format!("/transactions/{}", fixture.transaction_hash),
    )
    .await;
    assert_eq!(transaction["hash"], json!(fixture.transaction_hash));
    assert_eq!(transaction["block_height"], json!(1));

    let batches = get_json(app.clone(), "/rollup/batches?limit=1").await;
    assert_eq!(batches["total"], json!(1));
    assert_eq!(batches["items"][0]["batch_number"], json!(0));
    assert_eq!(batches["items"][0]["end_block_height"], json!(3));
    assert_eq!(batches["items"][0]["transaction_count"], json!(3));

    let batch = get_json(app.clone(), "/rollup/batches/0").await;
    assert_eq!(batch["transactions"].as_array().unwrap().len(), 3);
    assert_eq!(batch["l1_submit_tx_hash"], Value::Null);

    let settlement = get_json(app.clone(), "/rollup/batches/0/settlement-calldata").await;
    assert_eq!(settlement["contract_name"], json!("RollupSettlement"));
    assert_eq!(settlement["function"], json!("submitBatch"));
    assert_eq!(
        settlement["signature"],
        json!("submitBatch(uint256,bytes32,bytes32,bytes32,bytes32,bytes32)")
    );
    assert_eq!(settlement["args"]["batchNumber"], json!(0));
    assert_eq!(settlement["ordered_args"].as_array().unwrap().len(), 6);
    assert_eq!(
        settlement["ordered_args"][1],
        settlement["args"]["prevStateRoot"]
    );
    assert_eq!(
        settlement["ordered_args"][2],
        settlement["args"]["newStateRoot"]
    );
    assert!(settlement["args"]["prevStateRoot"]
        .as_str()
        .expect("state root should be a string")
        .starts_with("0x"));

    let submit_hash = format!("0x{}", "a".repeat(64));
    let contract_address = format!("0x{}", "b".repeat(40));
    let finalize_before_submit = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "finalized",
            "tx_hash": format!("0x{}", "c".repeat(64)),
            "contract_address": contract_address,
            "chain_id": 31337,
            "block_number": 8,
            "gas_used": 30000,
            "receipt": {
                "transactionHash": format!("0x{}", "c".repeat(64)),
                "to": contract_address,
                "status": "0x1"
            }
        }),
    )
    .await;
    assert_eq!(finalize_before_submit.status(), StatusCode::CONFLICT);

    let receipt_mismatch = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "submitted",
            "tx_hash": submit_hash,
            "contract_address": contract_address,
            "chain_id": 31337,
            "block_number": 7,
            "gas_used": 50000,
            "receipt": {
                "transactionHash": format!("0x{}", "d".repeat(64)),
                "to": contract_address,
                "status": "0x1"
            }
        }),
    )
    .await;
    assert_eq!(receipt_mismatch.status(), StatusCode::BAD_REQUEST);

    let submit_receipt = json!({
        "transactionHash": submit_hash,
        "to": contract_address,
        "status": "0x1",
        "blockNumber": "0x7",
        "gasUsed": "0xc350"
    });
    let settlement_update = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "submitted",
            "tx_hash": submit_hash,
            "contract_address": contract_address,
            "chain_id": 31337,
            "block_number": 7,
            "gas_used": 50000,
            "receipt": submit_receipt
        }),
    )
    .await;
    assert_eq!(settlement_update.status(), StatusCode::OK);
    let settlement_update = response_json(settlement_update).await;
    assert_eq!(settlement_update["status"], json!("submitted"));
    assert_eq!(
        settlement_update["l1_submit_tx_hash"],
        json!(format!("0x{}", "a".repeat(64)))
    );
    assert_eq!(settlement_update["l1_chain_id"], json!(31337));
    assert_eq!(settlement_update["l1_submit_block_number"], json!(7));
    assert_eq!(settlement_update["l1_submit_gas_used"], json!(50000));
    assert_eq!(
        settlement_update["l1_submit_receipt"]["transactionHash"],
        json!(format!("0x{}", "a".repeat(64)))
    );

    let finalize_hash = format!("0x{}", "c".repeat(64));
    let wrong_chain_finalize = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "finalized",
            "tx_hash": finalize_hash,
            "contract_address": contract_address,
            "chain_id": 1,
            "block_number": 8,
            "gas_used": 30000,
            "receipt": {
                "transactionHash": finalize_hash,
                "to": contract_address,
                "status": "0x1"
            }
        }),
    )
    .await;
    assert_eq!(wrong_chain_finalize.status(), StatusCode::CONFLICT);

    let wrong_contract = format!("0x{}", "e".repeat(40));
    let wrong_contract_finalize = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "finalized",
            "tx_hash": finalize_hash,
            "contract_address": wrong_contract,
            "chain_id": 31337,
            "block_number": 8,
            "gas_used": 30000,
            "receipt": {
                "transactionHash": finalize_hash,
                "to": wrong_contract,
                "status": "0x1"
            }
        }),
    )
    .await;
    assert_eq!(wrong_contract_finalize.status(), StatusCode::CONFLICT);

    let finalize_receipt = json!({
        "transactionHash": finalize_hash,
        "to": contract_address,
        "status": "0x1",
        "blockNumber": "0x8",
        "gasUsed": "0x7530"
    });
    let finalized_update = post_json(
        app.clone(),
        "/rollup/batches/0/settlement",
        &json!({
            "status": "finalized",
            "tx_hash": finalize_hash,
            "contract_address": contract_address,
            "chain_id": 31337,
            "block_number": 8,
            "gas_used": 30000,
            "receipt": finalize_receipt
        }),
    )
    .await;
    assert_eq!(finalized_update.status(), StatusCode::OK);
    let finalized_update = response_json(finalized_update).await;
    assert_eq!(finalized_update["status"], json!("finalized"));
    assert_eq!(
        finalized_update["l1_finalize_tx_hash"],
        json!(format!("0x{}", "c".repeat(64)))
    );
    assert_eq!(finalized_update["l1_finalize_block_number"], json!(8));
    assert_eq!(finalized_update["l1_finalize_gas_used"], json!(30000));

    let new_transfer = sign_transfer(
        &fixture.alice_secret_key,
        fixture.bob_address.clone(),
        5,
        fixture.alice_nonce,
    )
    .unwrap();
    let submit = post_json(app.clone(), "/rollup/transactions", &new_transfer).await;
    assert_eq!(submit.status(), StatusCode::OK);
    let submitted = response_json(submit).await;
    assert_eq!(submitted["status"], json!("pending"));
    assert_eq!(submitted["hash"], json!(transaction_hash(&new_transfer)));

    let pool_page = get_json(app.clone(), "/rollup/transactions").await;
    assert_eq!(pool_page["total"], json!(1));
    assert_eq!(pool_page["items"][0]["status"], json!("pending"));

    let pending_summary = get_json(app.clone(), "/chain/summary").await;
    assert_eq!(pending_summary["pending_transaction_count"], json!(1));

    let sequenced = post_json(app.clone(), "/rollup/batches", &json!({ "limit": 10 })).await;
    assert_eq!(sequenced.status(), StatusCode::OK);
    let sequenced = response_json(sequenced).await;
    assert_eq!(sequenced["block"]["height"], json!(4));
    assert_eq!(
        sequenced["included_transactions"].as_array().unwrap().len(),
        1
    );
    assert_eq!(sequenced["rollup_batch"]["batch_number"], json!(1));
    assert_eq!(sequenced["rollup_batch"]["start_block_height"], json!(4));
    assert_eq!(sequenced["rollup_batch"]["end_block_height"], json!(4));
    assert_eq!(sequenced["rollup_batch"]["transaction_count"], json!(1));

    let witness = get_json(app.clone(), "/rollup/batches/1/validity-witness").await;
    assert_eq!(witness["batch_number"], json!(1));
    assert_eq!(witness["previous_block"]["height"], json!(3));
    assert_eq!(witness["blocks"].as_array().unwrap().len(), 1);
    assert_eq!(
        witness["accounts_before"][&fixture.alice_address]["nonce"],
        json!(fixture.alice_nonce)
    );

    let sequenced_summary = get_json(app.clone(), "/chain/summary").await;
    assert_eq!(sequenced_summary["height"], json!(4));
    assert_eq!(sequenced_summary["transaction_count"], json!(4));
    assert_eq!(sequenced_summary["pending_transaction_count"], json!(0));

    let l2_recipient = format!("0x{}", "1".repeat(40));
    let deposit = json!({
        "chain_id": 31337,
        "inbox_address": format!("0x{}", "2".repeat(40)),
        "deposit_id": 0,
        "l1_tx_hash": format!("0x{}", "3".repeat(64)),
        "l1_block_number": 12,
        "l1_block_hash": format!("0x{}", "4".repeat(64)),
        "l1_log_index": 0,
        "depositor_l1_address": format!("0x{}", "5".repeat(40)),
        "recipient_l2_address": l2_recipient,
        "amount": 25,
        "rolling_commitment": format!("0x{}", "6".repeat(64)),
        "confirmations": 2
    });
    let observed = post_json(app.clone(), "/l1/deposits", &deposit).await;
    assert_eq!(observed.status(), StatusCode::OK);
    let observed = response_json(observed).await;
    assert_eq!(observed["deposit_id"], json!(0));
    assert_eq!(observed["status"], json!("confirmed"));
    assert_eq!(observed["confirmations"], json!(2));

    let repeated = post_json(app.clone(), "/l1/deposits", &deposit).await;
    assert_eq!(repeated.status(), StatusCode::OK);
    let deposits = get_json(app.clone(), "/l1/deposits").await;
    assert_eq!(deposits["total"], json!(1));

    let reorged_deposit = json!({
        "chain_id": 31337,
        "inbox_address": format!("0x{}", "2".repeat(40)),
        "deposit_id": 0,
        "l1_tx_hash": format!("0x{}", "7".repeat(64)),
        "l1_block_number": 12,
        "l1_block_hash": format!("0x{}", "8".repeat(64)),
        "l1_log_index": 1,
        "depositor_l1_address": format!("0x{}", "5".repeat(40)),
        "recipient_l2_address": l2_recipient,
        "amount": 25,
        "rolling_commitment": format!("0x{}", "9".repeat(64)),
        "confirmations": 0
    });
    let reobserved = post_json(app.clone(), "/l1/deposits", &reorged_deposit).await;
    assert_eq!(reobserved.status(), StatusCode::OK);
    let deposits = get_json(app.clone(), "/l1/deposits").await;
    assert_eq!(deposits["total"], json!(2));
    assert_eq!(deposits["items"][0]["status"], json!("observed"));
    assert_eq!(deposits["items"][1]["status"], json!("orphaned"));

    let stale_repeat = post_json(app.clone(), "/l1/deposits", &deposit).await;
    assert_eq!(stale_repeat.status(), StatusCode::OK);
    let deposits = get_json(app.clone(), "/l1/deposits").await;
    assert_eq!(deposits["total"], json!(2));
    assert_eq!(deposits["items"][0]["status"], json!("observed"));
    assert_eq!(deposits["items"][1]["status"], json!("orphaned"));

    let recipient_deposits = get_json(
        app.clone(),
        &format!("/l1/deposits/recipient/{}", l2_recipient),
    )
    .await;
    assert_eq!(recipient_deposits["total"], json!(2));

    insert_error_event(&pool).await;
    let health = get_json(app.clone(), "/health").await;
    assert_eq!(health["status"], json!("degraded"));
    assert_eq!(health["latest_error"]["message"], json!("test API failure"));

    let errors = get_json(app, "/errors").await;
    assert_eq!(errors.as_array().unwrap().len(), 1);
    assert_eq!(errors[0]["severity"], json!("error"));
    teardown_postgres_test_database(&pool, &schema).await;
}

#[tokio::test]
async fn confirmed_deposit_sequences_into_l2_state() {
    let Some(database_url) = optional_test_database_url() else {
        eprintln!("skipping Postgres integration test; set TEST_DATABASE_URL or DATABASE_URL");
        return;
    };

    let recipient = format!("0x{}", "1".repeat(40));
    let (pool, schema) = setup_postgres_test_database(&database_url).await;
    initialize_genesis(&pool, GenesisConfig { accounts: vec![] })
        .await
        .unwrap();
    let app = app_with_pool(pool.clone());
    let deposit = json!({
        "chain_id": 31337,
        "inbox_address": format!("0x{}", "2".repeat(40)),
        "deposit_id": 0,
        "l1_tx_hash": format!("0x{}", "3".repeat(64)),
        "l1_block_number": 12,
        "l1_block_hash": format!("0x{}", "4".repeat(64)),
        "l1_log_index": 0,
        "depositor_l1_address": format!("0x{}", "5".repeat(40)),
        "recipient_l2_address": recipient,
        "amount": 25,
        "rolling_commitment": format!("0x{}", "6".repeat(64)),
        "confirmations": 2
    });

    let observed = post_json(app.clone(), "/l1/deposits", &deposit).await;
    assert_eq!(observed.status(), StatusCode::OK);
    let sequenced = post_json(app.clone(), "/rollup/batches", &json!({ "limit": 10 })).await;
    assert_eq!(sequenced.status(), StatusCode::OK);
    let sequenced = response_json(sequenced).await;

    assert_eq!(sequenced["block"]["deposits"].as_array().unwrap().len(), 1);
    assert_eq!(sequenced["included_deposits"].as_array().unwrap().len(), 1);
    assert_eq!(
        sequenced["included_deposits"][0]["status"],
        json!("executed")
    );
    assert_eq!(sequenced["rollup_batch"]["deposit_count"], json!(1));
    assert_eq!(sequenced["rollup_batch"]["first_deposit_id"], json!(0));
    assert_eq!(sequenced["rollup_batch"]["last_deposit_id"], json!(0));
    assert_eq!(sequenced["rollup_batch"]["transaction_count"], json!(0));

    let account = get_json(
        app.clone(),
        &format!(
            "/accounts/{}",
            path_segment(deposit["recipient_l2_address"].as_str().unwrap())
        ),
    )
    .await;
    assert_eq!(account["balance"], json!(25));
    assert_eq!(account["nonce"], json!(0));

    let witness = get_json(app.clone(), "/rollup/batches/0/validity-witness").await;
    assert_eq!(
        witness["blocks"][0]["deposits"].as_array().unwrap().len(),
        1
    );

    let repeat_sequence = post_json(app, "/rollup/batches", &json!({ "limit": 10 })).await;
    assert_eq!(repeat_sequence.status(), StatusCode::BAD_REQUEST);

    teardown_postgres_test_database(&pool, &schema).await;
}

#[tokio::test]
async fn settlement_update_token_does_not_block_local_deposit_ingest() {
    let Some(database_url) = optional_test_database_url() else {
        eprintln!("skipping Postgres integration test; set TEST_DATABASE_URL or DATABASE_URL");
        return;
    };

    let (pool, schema) = setup_postgres_test_database(&database_url).await;
    let app = app_with_pool_and_tokens(
        pool.clone(),
        Some("__settlement_updates_disabled__".to_string()),
        None,
    );

    let observed = post_json(app, "/l1/deposits", &deposit_payload()).await;

    assert_eq!(observed.status(), StatusCode::OK);
    let observed = response_json(observed).await;
    assert_eq!(observed["status"], json!("confirmed"));
    teardown_postgres_test_database(&pool, &schema).await;
}

struct ChainFixture {
    chain: Chain,
    alice_address: String,
    alice_secret_key: String,
    alice_nonce: u64,
    bob_address: String,
    latest_block_hash: String,
    transaction_hash: String,
}

impl ChainFixture {
    fn with_transaction_blocks(count: usize) -> Self {
        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = alice.address.clone();
        let bob_address = bob.address.clone();
        let mut chain = new_chain(vec![
            Account {
                address: alice_address.clone(),
                balance: 100,
                nonce: 0,
            },
            Account {
                address: bob_address.clone(),
                balance: 0,
                nonce: 0,
            },
        ])
        .unwrap();

        finalize_genesis(&mut chain).unwrap();
        let mut first_hash = None;
        let mut latest_block_hash = chain.blocks.last().unwrap().block_hash.clone();
        for index in 0..count {
            let alice_sends = index % 2 == 0;
            let (secret_key, from, to) = if alice_sends {
                (&alice.secret_key_hex, &alice_address, bob_address.clone())
            } else {
                (&bob.secret_key_hex, &bob_address, alice_address.clone())
            };
            let nonce = next_nonce(&chain, from).unwrap();
            let transfer = sign_transfer(secret_key, to, 40, nonce).unwrap();
            first_hash.get_or_insert_with(|| transaction_hash(&transfer));
            add_pending_transaction(&mut chain, transfer).unwrap();
            let block = produce_block(&mut chain).unwrap();
            latest_block_hash = block.block_hash;
        }
        let alice_nonce = chain.accounts.get(&alice_address).unwrap().nonce;

        Self {
            chain,
            alice_address,
            alice_secret_key: alice.secret_key_hex,
            alice_nonce,
            bob_address,
            latest_block_hash,
            transaction_hash: first_hash.unwrap_or_default(),
        }
    }
}

fn deposit_payload() -> Value {
    json!({
        "chain_id": 31337,
        "inbox_address": format!("0x{}", "2".repeat(40)),
        "deposit_id": 0,
        "l1_tx_hash": format!("0x{}", "3".repeat(64)),
        "l1_block_number": 12,
        "l1_block_hash": format!("0x{}", "4".repeat(64)),
        "l1_log_index": 0,
        "depositor_l1_address": format!("0x{}", "5".repeat(40)),
        "recipient_l2_address": format!("0x{}", "1".repeat(40)),
        "amount": 25,
        "rolling_commitment": format!("0x{}", "6".repeat(64)),
        "confirmations": 2
    })
}

const INIT_SQL: &[&str] = &[
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

fn optional_test_database_url() -> Option<String> {
    env::var("TEST_DATABASE_URL")
        .ok()
        .or_else(|| env::var("DATABASE_URL").ok())
        .filter(|value| !value.trim().is_empty())
}

async fn setup_postgres_test_database(database_url: &str) -> (PgPool, String) {
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .connect(database_url)
        .await
        .expect("connect to Postgres test database");
    let schema = format!("explorer_api_test_{}", unique_suffix());

    sqlx::query(&format!(r#"CREATE SCHEMA "{schema}""#))
        .execute(&pool)
        .await
        .expect("create isolated test schema");
    sqlx::query(&format!(r#"SET search_path TO "{schema}""#))
        .execute(&pool)
        .await
        .expect("set isolated test schema search path");

    for migration in INIT_SQL {
        for statement in migration
            .split(';')
            .map(str::trim)
            .filter(|statement| !statement.is_empty())
        {
            pool.execute(statement)
                .await
                .expect("apply test database migration");
        }
    }

    (pool, schema)
}

async fn teardown_postgres_test_database(pool: &PgPool, schema: &str) {
    let _ = sqlx::query(&format!(r#"DROP SCHEMA IF EXISTS "{schema}" CASCADE"#))
        .execute(pool)
        .await;
}

fn unique_suffix() -> u128 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .expect("system clock should be after Unix epoch")
        .as_nanos()
}

async fn insert_indexed_chain(pool: &PgPool, chain: &Chain) {
    for block in &chain.blocks {
        sqlx::query(
            r#"
            INSERT INTO blocks (
                height, parent_hash, state_root, block_hash, transaction_count, raw_block
            )
            VALUES ($1, $2, $3, $4, $5, $6)
            "#,
        )
        .bind(block.height as i64)
        .bind(&block.parent_hash)
        .bind(&block.state_root)
        .bind(&block.block_hash)
        .bind(block.transactions.len() as i32)
        .bind(serde_json::to_value(block).unwrap())
        .execute(pool)
        .await
        .expect("insert indexed block");

        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO transactions (
                    hash,
                    block_height,
                    transaction_index,
                    from_address,
                    to_address,
                    amount,
                    nonce,
                    signature,
                    raw_transfer
                )
                VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9)
                "#,
            )
            .bind(transaction_hash(transaction))
            .bind(block.height as i64)
            .bind(transaction_index as i32)
            .bind(&transaction.from)
            .bind(&transaction.to)
            .bind(transaction.amount as i64)
            .bind(transaction.nonce as i64)
            .bind(&transaction.signature)
            .bind(serde_json::to_value(transaction).unwrap())
            .execute(pool)
            .await
            .expect("insert indexed transaction");
        }
    }

    for account in chain.accounts.values() {
        sqlx::query(
            r#"
            INSERT INTO accounts (address, balance, nonce)
            VALUES ($1, $2, $3)
            "#,
        )
        .bind(&account.address)
        .bind(account.balance as i64)
        .bind(account.nonce as i64)
        .execute(pool)
        .await
        .expect("insert indexed account");
    }

    if let Some(latest) = chain.blocks.last() {
        sqlx::query(
            r#"
            INSERT INTO indexer_checkpoints (id, last_indexed_height)
            VALUES ('chain', $1)
            "#,
        )
        .bind(latest.height as i64)
        .execute(pool)
        .await
        .expect("insert indexer checkpoint");
    }

    if chain.blocks.last().is_some_and(|block| block.height >= 1) {
        let commitment = rollup_batch_commitment(
            chain,
            0,
            1,
            chain.blocks.last().expect("checked above").height,
        )
        .expect("build rollup batch commitment");
        insert_rollup_batch(pool, chain, &commitment).await;
    }
}

async fn insert_rollup_batch(pool: &PgPool, chain: &Chain, commitment: &RollupBatchCommitment) {
    sqlx::query(
        r#"
        INSERT INTO rollup_batches (
            batch_number,
            start_block_height,
            end_block_height,
            prev_state_root,
            new_state_root,
            transaction_root,
            block_hash_root,
            data_hash,
            transaction_count,
            status,
            raw_commitment
        )
        VALUES ($1, $2, $3, $4, $5, $6, $7, $8, $9, 'built', $10)
        "#,
    )
    .bind(commitment.batch_number as i64)
    .bind(commitment.start_block_height as i64)
    .bind(commitment.end_block_height as i64)
    .bind(&commitment.prev_state_root)
    .bind(&commitment.new_state_root)
    .bind(&commitment.transaction_root)
    .bind(&commitment.block_hash_root)
    .bind(&commitment.data_hash)
    .bind(commitment.transaction_count as i32)
    .bind(serde_json::to_value(commitment).unwrap())
    .execute(pool)
    .await
    .expect("insert rollup batch");

    for block in chain.blocks.iter().filter(|block| {
        block.height >= commitment.start_block_height && block.height <= commitment.end_block_height
    }) {
        for (transaction_index, transaction) in block.transactions.iter().enumerate() {
            sqlx::query(
                r#"
                INSERT INTO rollup_batch_transactions (
                    batch_number, transaction_hash, block_height, transaction_index
                )
                VALUES ($1, $2, $3, $4)
                "#,
            )
            .bind(commitment.batch_number as i64)
            .bind(transaction_hash(transaction))
            .bind(block.height as i64)
            .bind(transaction_index as i32)
            .execute(pool)
            .await
            .expect("insert rollup batch transaction");
        }
    }
}

async fn insert_error_event(pool: &PgPool) {
    sqlx::query(
        r#"
        INSERT INTO error_events (service, severity, message, context)
        VALUES ('explorer-api', 'error', 'test API failure', '{"test": true}'::jsonb)
        "#,
    )
    .execute(pool)
    .await
    .expect("insert error event");
}

async fn get_json(app: axum::Router, uri: &str) -> Value {
    let response = get_response(app, uri).await;
    assert_eq!(response.status(), StatusCode::OK);

    response_json(response).await
}

async fn get_response(app: axum::Router, uri: &str) -> Response<Body> {
    app.oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap()
}

async fn post_json<T: serde::Serialize>(
    app: axum::Router,
    uri: &str,
    payload: &T,
) -> Response<Body> {
    app.oneshot(
        Request::builder()
            .method("POST")
            .uri(uri)
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_string(payload).unwrap()))
            .unwrap(),
    )
    .await
    .unwrap()
}

async fn response_json(response: Response<Body>) -> Value {
    let bytes = response.into_body().collect().await.unwrap().to_bytes();
    serde_json::from_slice(&bytes).unwrap()
}

fn path_segment(value: &str) -> String {
    value
        .bytes()
        .map(|byte| match byte {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                (byte as char).to_string()
            }
            _ => format!("%{byte:02X}"),
        })
        .collect()
}
