use std::path::PathBuf;

use axum::{
    body::Body,
    http::{Request, StatusCode},
};
use chain_core::{
    add_pending_transaction, finalize_genesis, new_chain, produce_block, sign_transfer,
    transaction_hash, Account,
};
use explorer_api::{app, app_with_pool};
use http_body_util::BodyExt;
use serde_json::{json, Value};
use tempfile::TempDir;
use tower::ServiceExt;
use wallet_core::{address_from_public_key, create_keypair};

#[tokio::test]
async fn postgres_router_exposes_static_routes_without_database_queries() {
    let pool = sqlx::postgres::PgPoolOptions::new()
        .connect_lazy("postgresql://postgres:postgres@localhost/zk_rollup")
        .unwrap();
    let app = app_with_pool(pool);

    let index = get_json(app.clone(), "/").await;
    assert_eq!(index["name"], json!("zk-rollup explorer api"));

    let health = get_json(app, "/health").await;
    assert_eq!(health["status"], json!("ok"));
}

#[tokio::test]
async fn returns_api_index() {
    let fixture = ChainFixture::new();
    let app = app(fixture.chain_file.clone());

    let index = get_json(app, "/").await;
    assert_eq!(index["name"], json!("zk-rollup explorer api"));
    assert!(index["endpoints"]
        .as_array()
        .unwrap()
        .contains(&json!("GET /health")));

    let _temp_dir_kept_alive = fixture.temp_dir;
}

#[tokio::test]
async fn returns_chain_summary_and_latest_blocks() {
    let fixture = ChainFixture::new();
    let app = app(fixture.chain_file.clone());

    let summary = get_json(app.clone(), "/chain/summary").await;
    assert_eq!(summary["height"], json!(1));
    assert_eq!(summary["block_count"], json!(2));
    assert_eq!(summary["transaction_count"], json!(1));
    assert_eq!(summary["account_count"], json!(2));
    assert_eq!(summary["genesis_finalized"], json!(true));
    assert_eq!(summary["valid"], json!(true));

    let blocks = get_json(app, "/blocks").await;
    assert_eq!(blocks.as_array().unwrap().len(), 2);
    assert_eq!(blocks[0]["height"], json!(1));
    assert_eq!(blocks[0]["transaction_count"], json!(1));
    assert_eq!(blocks[1]["height"], json!(0));

    let _temp_dir_kept_alive = fixture.temp_dir;
}

#[tokio::test]
async fn fetches_blocks_accounts_and_transactions() {
    let fixture = ChainFixture::new();
    let app = app(fixture.chain_file.clone());

    let block = get_json(
        app.clone(),
        &format!("/blocks/hash/{}", fixture.latest_block_hash),
    )
    .await;
    assert_eq!(block["height"], json!(1));
    assert_eq!(block["block_hash"], json!(fixture.latest_block_hash));

    let account = get_json(
        app.clone(),
        &format!("/accounts/{}", path_segment(&fixture.alice_address)),
    )
    .await;
    assert_eq!(account["address"], json!(fixture.alice_address));
    assert_eq!(account["balance"], json!(60));
    assert_eq!(account["nonce"], json!(1));

    let transaction = get_json(
        app.clone(),
        &format!("/transactions/{}", fixture.transaction_hash),
    )
    .await;
    assert_eq!(transaction["hash"], json!(fixture.transaction_hash));
    assert_eq!(transaction["block_height"], json!(1));
    assert_eq!(transaction["transaction_index"], json!(0));
    assert_eq!(transaction["transfer"]["amount"], json!(40));

    let _temp_dir_kept_alive = fixture.temp_dir;
}

#[tokio::test]
async fn searches_by_height_hash_address_and_transaction() {
    let fixture = ChainFixture::new();
    let app = app(fixture.chain_file.clone());

    let by_height = get_json(app.clone(), "/search/1").await;
    assert_eq!(by_height["kind"], json!("Block"));
    assert_eq!(by_height["data"]["height"], json!(1));

    let by_hash = get_json(
        app.clone(),
        &format!("/search/{}", fixture.latest_block_hash),
    )
    .await;
    assert_eq!(by_hash["kind"], json!("Block"));
    assert_eq!(
        by_hash["data"]["block_hash"],
        json!(fixture.latest_block_hash)
    );

    let by_address = get_json(
        app.clone(),
        &format!("/search/{}", path_segment(&fixture.bob_address)),
    )
    .await;
    assert_eq!(by_address["kind"], json!("Account"));
    assert_eq!(by_address["data"]["balance"], json!(40));

    let by_transaction = get_json(
        app.clone(),
        &format!("/search/{}", fixture.transaction_hash),
    )
    .await;
    assert_eq!(by_transaction["kind"], json!("Transaction"));
    assert_eq!(by_transaction["data"]["block_height"], json!(1));

    let missing = app
        .oneshot(
            Request::builder()
                .uri("/search/not-found")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(missing.status(), StatusCode::NOT_FOUND);

    let _temp_dir_kept_alive = fixture.temp_dir;
}

struct ChainFixture {
    temp_dir: TempDir,
    chain_file: PathBuf,
    alice_address: String,
    bob_address: String,
    latest_block_hash: String,
    transaction_hash: String,
}

impl ChainFixture {
    fn new() -> Self {
        let temp_dir = TempDir::new().unwrap();
        let chain_file = temp_dir.path().join("chain.json");

        let alice = create_keypair();
        let bob = create_keypair();
        let alice_address = address_from_public_key(&alice.public_key_base64).unwrap();
        let bob_address = address_from_public_key(&bob.public_key_base64).unwrap();
        let mut chain = new_chain(vec![Account {
            address: alice_address.clone(),
            balance: 100,
            nonce: 0,
        }])
        .unwrap();

        finalize_genesis(&mut chain).unwrap();
        let transfer = sign_transfer(&alice.secret_key_base64, bob_address.clone(), 40, 0).unwrap();
        let hash = transaction_hash(&transfer);
        add_pending_transaction(&mut chain, transfer).unwrap();
        let block = produce_block(&mut chain).unwrap();

        std::fs::write(&chain_file, serde_json::to_string_pretty(&chain).unwrap()).unwrap();

        Self {
            temp_dir,
            chain_file,
            alice_address,
            bob_address,
            latest_block_hash: block.block_hash,
            transaction_hash: hash,
        }
    }
}

async fn get_json(app: axum::Router, uri: &str) -> Value {
    let response = app
        .oneshot(Request::builder().uri(uri).body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(response.status(), StatusCode::OK);

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
