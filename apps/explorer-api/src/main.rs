use std::{env, fs, net::SocketAddr, path::PathBuf, time::Duration};

use anyhow::{Context, Result};
use clap::Parser;
use explorer_api::{
    app_with_config, dev_cors, initialize_genesis, new_event_queue, restricted_cors,
    sequence_pending_transactions_with_events, AppConfig, EventQueue, GenesisConfig,
};
use sqlx::postgres::PgPoolOptions;

const PRISMA_ENV_PATH: &str = "prisma/.env";

#[derive(Debug, Parser)]
#[command(name = "explorer-api")]
#[command(about = "Postgres-backed rollup explorer and sequencer API")]
struct Cli {
    #[arg(long)]
    database_url: Option<String>,

    #[arg(long, default_value = "0.0.0.0:3000")]
    bind: SocketAddr,

    #[arg(long)]
    auto_sequence: bool,

    #[arg(long, default_value_t = 10)]
    auto_sequence_interval_seconds: u64,

    #[arg(
        long,
        value_name = "GENESIS_JSON",
        conflicts_with = "auto_sequence",
        help = "Initialize genesis atomically from an account allocation file, then exit"
    )]
    initialize_genesis: Option<PathBuf>,
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();
    let config = RuntimeConfig::from_env()?;
    let database_url = database_url(cli.database_url)?;
    validate_database_url(&database_url)?;
    let pool = PgPoolOptions::new()
        .max_connections(5)
        .connect_lazy(&database_url)
        .context("DATABASE_URL is not a valid Postgres connection string")?;

    if let Some(path) = &cli.initialize_genesis {
        let config: GenesisConfig = serde_json::from_slice(
            &fs::read(path)
                .with_context(|| format!("failed to read genesis file {}", path.display()))?,
        )
        .with_context(|| format!("failed to parse genesis file {}", path.display()))?;
        let initialized = initialize_genesis(&pool, config)
            .await
            .context("genesis initialization failed")?;
        println!("{}", serde_json::to_string_pretty(&initialized)?);
        return Ok(());
    }

    let listener = tokio::net::TcpListener::bind(cli.bind)
        .await
        .with_context(|| format!("failed to bind explorer API to {}", cli.bind))?;
    let l1_deposit_inbox = env::var("L1_DEPOSIT_INBOX")
        .ok()
        .filter(|value| !value.is_empty())
        .unwrap_or_else(|| "0x0000000000000000000000000000000000000000".to_string())
        .to_ascii_lowercase();
    let settlement_token = env::var("SETTLEMENT_UPDATE_TOKEN")
        .ok()
        .filter(|value| !value.is_empty());
    let deposit_token = env::var("L1_DEPOSIT_INGEST_TOKEN")
        .ok()
        .filter(|value| !value.is_empty());
    let events = new_event_queue();
    if cli.auto_sequence {
        spawn_auto_sequencer(
            pool.clone(),
            cli.auto_sequence_interval_seconds,
            l1_deposit_inbox.clone(),
            events.clone(),
        );
    }
    let cors = if config.cors_origins.is_empty() {
        dev_cors()
    } else {
        restricted_cors(&config.cors_origins).map_err(anyhow::Error::msg)?
    };
    let router = app_with_config(
        pool,
        AppConfig {
            operator_token: config.operator_token,
            settlement_token,
            deposit_token,
            deposit_inbox: l1_deposit_inbox,
            events,
            cors,
        },
    );

    println!("serving Postgres-backed rollup API");
    println!("listening on http://{}", cli.bind);
    println!("readiness probe available at /ready");
    axum::serve(listener, router).await?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct RuntimeConfig {
    operator_token: Option<String>,
    cors_origins: Vec<String>,
}

impl RuntimeConfig {
    fn from_env() -> Result<Self> {
        Self::from_values(
            env::var("APP_ENV").ok(),
            env::var("OPERATOR_API_TOKEN").ok(),
            env::var("CORS_ALLOWED_ORIGINS").ok(),
        )
    }

    fn from_values(
        app_env: Option<String>,
        operator_token: Option<String>,
        cors_origins: Option<String>,
    ) -> Result<Self> {
        let production = app_env
            .as_deref()
            .map(str::trim)
            .is_some_and(|value| value.eq_ignore_ascii_case("production"));
        let operator_token = non_empty_env(operator_token);
        let cors_origins = parse_csv_env(cors_origins);

        if production && operator_token.is_none() {
            anyhow::bail!("OPERATOR_API_TOKEN must be set when APP_ENV=production");
        }
        if production && cors_origins.is_empty() {
            anyhow::bail!("CORS_ALLOWED_ORIGINS must be set when APP_ENV=production");
        }

        Ok(Self {
            operator_token,
            cors_origins,
        })
    }
}

fn non_empty_env(value: Option<String>) -> Option<String> {
    value
        .map(|value| value.trim().to_string())
        .filter(|value| !value.is_empty())
}

fn parse_csv_env(value: Option<String>) -> Vec<String> {
    value
        .into_iter()
        .flat_map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|origin| !origin.is_empty())
                .map(str::to_string)
                .collect::<Vec<_>>()
        })
        .collect()
}

fn spawn_auto_sequencer(
    pool: sqlx::PgPool,
    interval_seconds: u64,
    l1_deposit_inbox: String,
    events: EventQueue,
) {
    let interval = Duration::from_secs(interval_seconds.max(1));
    tokio::spawn(async move {
        println!(
            "automatic sequencer enabled; checking pending transactions every {} seconds",
            interval.as_secs()
        );
        let mut ticker = tokio::time::interval(interval);
        loop {
            ticker.tick().await;
            match sequence_pending_transactions_with_events(
                pool.clone(),
                100,
                l1_deposit_inbox.clone(),
                events.clone(),
            )
            .await
            {
                Ok(true) => println!("automatic sequencer processed pending transactions"),
                Ok(false) => {}
                Err(error) => eprintln!("automatic sequencing failed: {error}"),
            }
        }
    });
}

fn database_url(cli_value: Option<String>) -> Result<String> {
    cli_value
        .or_else(|| env::var("DATABASE_URL").ok())
        .or_else(database_url_from_prisma_env)
        .filter(|value| !value.trim().is_empty())
        .context("provide --database-url, set DATABASE_URL, or add DATABASE_URL to prisma/.env")
}

fn validate_database_url(database_url: &str) -> Result<()> {
    if !database_url.starts_with("postgres://") && !database_url.starts_with("postgresql://") {
        anyhow::bail!("DATABASE_URL must use the postgres:// or postgresql:// scheme");
    }
    if !database_url.contains('@') {
        anyhow::bail!("DATABASE_URL must include database credentials and a host");
    }
    Ok(())
}

fn database_url_from_prisma_env() -> Option<String> {
    let entries = dotenvy::from_path_iter(PRISMA_ENV_PATH).ok()?;
    for entry in entries.flatten() {
        let (key, value) = entry;
        if key == "DATABASE_URL" {
            return Some(value);
        }
    }
    None
}

#[cfg(test)]
mod tests {
    use super::RuntimeConfig;

    #[test]
    fn production_requires_operator_token() {
        let error = RuntimeConfig::from_values(
            Some("production".to_string()),
            None,
            Some("https://rolluplens.example.com".to_string()),
        )
        .unwrap_err();

        assert!(error.to_string().contains("OPERATOR_API_TOKEN"));
    }

    #[test]
    fn production_requires_cors_origins() {
        let error = RuntimeConfig::from_values(
            Some("production".to_string()),
            Some("secret".to_string()),
            None,
        )
        .unwrap_err();

        assert!(error.to_string().contains("CORS_ALLOWED_ORIGINS"));
    }

    #[test]
    fn non_production_allows_missing_security_env() {
        let config = RuntimeConfig::from_values(Some("development".to_string()), None, None)
            .expect("development config should allow local defaults");

        assert_eq!(config.operator_token, None);
        assert!(config.cors_origins.is_empty());
    }

    #[test]
    fn production_parses_security_env() {
        let config = RuntimeConfig::from_values(
            Some("production".to_string()),
            Some(" secret ".to_string()),
            Some(" https://ui.example.com,https://admin.example.com ".to_string()),
        )
        .expect("production config should accept required security settings");

        assert_eq!(config.operator_token, Some("secret".to_string()));
        assert_eq!(
            config.cors_origins,
            vec![
                "https://ui.example.com".to_string(),
                "https://admin.example.com".to_string()
            ]
        );
    }
}
