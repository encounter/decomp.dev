mod cron;
mod frogress;
mod handlers;
mod proto;

use std::{
    fs::File,
    io::BufReader,
    net::{IpAddr, Ipv4Addr, SocketAddr},
    str::FromStr,
    sync::Arc,
    time::Duration,
};

use axum::{
    Router,
    extract::{ConnectInfo, FromRef},
    http::{Method, Request, header},
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
    trace::{DefaultOnResponse, MakeSpan, TraceLayer},
};
use tower_sessions::{Expiry, SessionManagerLayer, SessionStore, cookie::SameSite};
use tower_sessions_sqlx_store::SqliteStore;
use tracing::{Level, Span};
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
                .make_span_with(MyMakeSpan { level: Level::INFO })
                .on_response(DefaultOnResponse::new().level(Level::INFO)),
        )
        .layer(TimeoutLayer::new(Duration::from_secs(60)))
        .layer(CorsLayer::new().allow_methods([Method::GET]).allow_origin(cors::Any))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_same_site(SameSite::Lax)
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

#[derive(Debug, Clone)]
pub struct MyMakeSpan {
    level: Level,
}

impl<B> MakeSpan<B> for MyMakeSpan {
    fn make_span(&mut self, request: &Request<B>) -> Span {
        let cf_connecting_ip = request.headers().get("CF-Connecting-IP");
        let ip = if let Some(v) = cf_connecting_ip {
            str::from_utf8(v.as_bytes()).ok().and_then(|s| IpAddr::from_str(s).ok())
        } else if let Some(ConnectInfo(socket_addr)) =
            request.extensions().get::<ConnectInfo<SocketAddr>>()
        {
            Some(socket_addr.ip())
        } else {
            None
        };
        let ip = ip.unwrap_or(IpAddr::from([0, 0, 0, 0]));
        let user_agent = request
            .headers()
            .get(header::USER_AGENT)
            .and_then(|v| v.to_str().ok())
            .unwrap_or("[unknown]");
        macro_rules! make_span {
            ($level:expr) => {
                tracing::span!(
                    $level,
                    "request",
                    method = %request.method(),
                    uri = %request.uri(),
                    ip = %ip,
                    user_agent = %user_agent,
                )
            }
        }
        match self.level {
            Level::ERROR => make_span!(Level::ERROR),
            Level::WARN => make_span!(Level::WARN),
            Level::INFO => make_span!(Level::INFO),
            Level::DEBUG => make_span!(Level::DEBUG),
            Level::TRACE => make_span!(Level::TRACE),
        }
    }
}
