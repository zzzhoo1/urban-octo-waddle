mod account;
mod admin;
mod auth;
mod circuit;
mod config;
mod crypto;
mod error;
mod gateway;
mod logs;
mod openai;
mod router;
mod routes;
mod session;
mod sse_tap;
mod statics;
mod store;
mod upstream;

use std::sync::Arc;

use axum::{response::Redirect, routing::get, Router};
use clap::Parser;
use tracing_subscriber::EnvFilter;

use crate::account::Pool;
use crate::auth::KeyAuth;
use crate::config::GatewayConfig;
use crate::crypto::Encryptor;
use crate::gateway::Gateway;
use crate::logs::RequestLog;
use crate::router::ModelRouter;
use crate::store::JsonFileStore;
use crate::upstream::{CodeArtsClient, CODEARTS_GATEWAY_PROD};

/// CodeArts OpenAI-compatible multi-account gateway (Rust rewrite).
#[derive(Parser, Debug)]
#[command(name = "gateway", version, about)]
pub struct Cli {
    /// Listen address, e.g. 127.0.0.1:16211 or :8080
    #[arg(long = "listen", default_value = ":8080")]
    pub listen: String,

    /// Encryption secret key (AES-256-GCM, SHA256 derived)
    #[arg(long = "secret")]
    pub secret: String,

    /// Account store path (JSON file)
    #[arg(long = "store", default_value = "")]
    pub store: String,

    /// Upstream base url (bearer mode, kept for CLI compatibility)
    #[arg(long = "upstream", default_value = "")]
    pub upstream: String,

    /// Upstream bearer token (kept for CLI compatibility)
    #[arg(long = "upstream-token", default_value = "")]
    pub upstream_token: String,

    /// CodeArts access key id
    #[arg(long = "codearts-ak", default_value = "")]
    pub codearts_ak: String,

    /// CodeArts secret access key
    #[arg(long = "codearts-sk", default_value = "")]
    pub codearts_sk: String,

    /// CodeArts STS security token (deprecated, accepted for compatibility)
    #[arg(long = "codearts-token", default_value = "")]
    pub codearts_token: String,

    /// Auto STS (deprecated, accepted for compatibility)
    #[arg(long = "codearts-sts-auto", default_value_t = false)]
    pub codearts_sts_auto: bool,

    /// CodeArts gateway url
    #[arg(long = "codearts-gateway", default_value = CODEARTS_GATEWAY_PROD)]
    pub codearts_gateway: String,

    /// Gateway api key (admin + data plane); acts as the master key when
    /// keys.json is empty.
    #[arg(long = "gw-key", default_value = "")]
    pub gw_key: String,
}

/// Shared application state.
pub struct AppState {
    pub gateway: Arc<Gateway>,
    pub keys: Arc<KeyAuth>,
    pub pool: Arc<Pool>,
    pub store: Arc<JsonFileStore>,
    pub key_store: Arc<JsonFileStore>,
    pub encryptor: Arc<Encryptor>,
    pub logs: Arc<RequestLog>,
    pub cfg: GatewayConfig,
    pub listen: String,
    pub router: Arc<ModelRouter>,
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::try_from_default_env().unwrap_or_else(|_| EnvFilter::new("info")))
        .init();

    let cli = Cli::parse();

    if cli.secret.is_empty() {
        anyhow::bail!("--secret is required");
    }
    if cli.codearts_ak.is_empty() || cli.codearts_sk.is_empty() {
        anyhow::bail!("--codearts-ak and --codearts-sk are required");
    }
    if cli.gw_key.is_empty() {
        anyhow::bail!("--gw-key is required");
    }

    let cfg = GatewayConfig::default();
    let encryptor = Arc::new(Encryptor::new(&cli.secret));
    let store_path = if cli.store.is_empty() {
        "accounts.json".to_string()
    } else {
        cli.store.clone()
    };
    let store = Arc::new(JsonFileStore::new(store_path));

    let pool = Arc::new(Pool::new());
    for acc in store.load()? {
        pool.add_data(acc);
    }

    // API keys: load persisted keys; fall back to the master --gw-key so
    // existing single-key deployments keep working without migration.
    let persisted = store.load_keys().unwrap_or_default();
    let keys = if persisted.is_empty() {
        Arc::new(KeyAuth::new(vec![cli.gw_key.clone()]))
    } else {
        let ka = KeyAuth::from_entries(persisted);
        // Ensure the master key still works even if not in the file.
        if !ka.authenticate(&cli.gw_key) {
            ka.upsert(crate::auth::ApiKey {
                name: "master".to_string(),
                key_hash: crate::auth::hash_key(&cli.gw_key),
                hint: crate::auth::mask_key(&cli.gw_key),
                created_at: Some(chrono::Utc::now()),
                expires_at: None,
                token_quota: 0,
                tokens_used: 0,
                model_allowlist: Vec::new(),
                enabled: true,
            });
        }
        Arc::new(ka)
    };

    let router = Arc::new(ModelRouter::new(
        store.load_router().unwrap_or_default(),
    ));

    let upstream = Arc::new(CodeArtsClient::new(
        cli.codearts_ak.clone(),
        cli.codearts_sk.clone(),
        cli.codearts_gateway.clone(),
    ));

    let gateway = Arc::new(Gateway::new(
        pool.clone(),
        upstream,
        keys.clone(),
        cfg.clone(),
        router.clone(),
    ));

    let state = Arc::new(AppState {
        gateway,
        keys,
        pool,
        store: store.clone(),
        key_store: store,
        encryptor,
        logs: Arc::new(RequestLog::new(500)),
        cfg,
        listen: cli.listen.clone(),
        router,
    });

    let app = Router::new()
        .merge(routes::v1_router(state.clone()))
        .merge(admin::admin_router(state.clone()))
        .route("/", get(|| async { Redirect::temporary("/admin/") }))
        .route("/admin", get(|| async { Redirect::temporary("/admin/") }))
        .route("/admin/", get(statics::admin_index))
        .route("/admin/{*path}", get(statics::admin_static))
        .layer(tower_http::trace::TraceLayer::new_for_http());

    let bind = normalize_listen(&cli.listen);
    tracing::info!("listening on {}", bind);
    let listener = tokio::net::TcpListener::bind(&bind).await?;
    axum::serve(
        listener,
        app.into_make_service_with_connect_info::<std::net::SocketAddr>(),
    )
    .await?;
    Ok(())
}

/// Convert Go-style ':8080' into '0.0.0.0:8080'.
fn normalize_listen(addr: &str) -> String {
    if let Some(port) = addr.strip_prefix(':') {
        format!("0.0.0.0:{}", port)
    } else {
        addr.to_string()
    }
}
