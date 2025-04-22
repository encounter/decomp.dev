mod cron;
mod frogress;
mod handlers;
mod proto;

use std::{
    fs::File,
    io::BufReader,
    net::{Ipv4Addr, SocketAddr},
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    extract::FromRef,
    http::{Method, header},
};
use decomp_dev_core::config::{Config, GitHubConfig};
use decomp_dev_db::Database;
use decomp_dev_github::{GitHub, webhook::WebhookState};
use tokio::{net::TcpListener, signal};
use tower::ServiceBuilder;
use tower_http::{
    ServiceBuilderExt, cors,
    cors::CorsLayer,
    timeout::TimeoutLayer,
    trace::{DefaultMakeSpan, DefaultOnResponse, TraceLayer},
};
use tower_sessions::{Expiry, SessionManagerLayer, SessionStore};
use tower_sessions_sqlx_store::SqliteStore;
use tracing_subscriber::{EnvFilter, filter::LevelFilter};

use crate::handlers::build_router;

#[derive(Clone, FromRef)]
struct AppState {
    config: Config,
    db: Database,
    github: GitHub,
}

impl FromRef<AppState> for WebhookState {
    fn from_ref(state: &AppState) -> Self {
        Self {
            config: state.config.github.clone(),
            db: state.db.clone(),
            github: state.github.clone(),
        }
    }
}

impl FromRef<AppState> for GitHubConfig {
    fn from_ref(state: &AppState) -> Self { state.config.github.clone() }
}

#[tokio::main]
async fn main() {
    tracing_subscriber::fmt()
        .with_env_filter(
            EnvFilter::builder()
                // Default to info level
                .with_default_directive(LevelFilter::INFO.into())
                .from_env_lossy(),
        )
        .init();

    let config: Config = {
        let file = BufReader::new(File::open("config.yml").expect("Failed to open config file"));
        serde_yaml::from_reader(file).expect("Failed to parse config file")
    };
    let db = Database::new(&config.db).await.expect("Failed to open database");
    let github = GitHub::new(&config.github).await.expect("Failed to create GitHub client");
    let state = AppState { config, db: db.clone(), github };

    // Create session store
    let session_store = SqliteStore::new(db.pool.clone());
    session_store.migrate().await.expect("Failed to migrate session store");

    // Refresh before starting the server
    // cron::refresh_projects(&mut state).await.expect("Failed to refresh projects");
    // frogress::migrate_data(&mut state).await.expect("Failed to migrate data");

    // Start the task scheduler
    let mut scheduler = cron::create(state.clone(), session_store.clone())
        .await
        .expect("Failed to create scheduler");

    // Run our service
    let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, state.config.server.port));
    tracing::info!("Listening on {}", addr);
    axum::serve(
        TcpListener::bind(addr).await.expect("bind error"),
        app(state, session_store).into_make_service_with_connect_info::<SocketAddr>(),
    )
    .with_graceful_shutdown(shutdown_signal())
    .await
    .expect("server error");

    scheduler.shutdown().await.expect("Failed to shut down scheduler");
    db.close().await;
    tracing::info!("Shut down gracefully");
}

fn app(state: AppState, session_store: impl SessionStore + Clone) -> Router {
    let sensitive_headers: Arc<[_]> = vec![header::AUTHORIZATION, header::COOKIE].into();
    let middleware = ServiceBuilder::new()
        .sensitive_request_headers(sensitive_headers.clone())
        .sensitive_response_headers(sensitive_headers)
        .layer(
            TraceLayer::new_for_http()
                .make_span_with(DefaultMakeSpan::new().level(tracing::Level::INFO))
                .on_response(DefaultOnResponse::new().level(tracing::Level::INFO)),
        )
        .layer(TimeoutLayer::new(Duration::from_secs(10)))
        .layer(CorsLayer::new().allow_methods([Method::GET]).allow_origin(cors::Any))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_expiry(Expiry::OnInactivity(time::Duration::days(1))),
        )
        .compression();
    let router = build_router();
    #[cfg(debug_assertions)]
    let router = router.layer(tower_livereload::LiveReloadLayer::new());
    router.layer(middleware).with_state(state)
}

async fn shutdown_signal() {
    let ctrl_c = async {
        signal::ctrl_c().await.expect("failed to install Ctrl+C handler");
    };

    #[cfg(unix)]
    let terminate = async {
        signal::unix::signal(signal::unix::SignalKind::terminate())
            .expect("failed to install signal handler")
            .recv()
            .await;
    };

    #[cfg(not(unix))]
    let terminate = std::future::pending::<()>();

    tokio::select! {
        _ = ctrl_c => {},
        _ = terminate => {},
    }
}
