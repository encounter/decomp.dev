#![allow(clippy::too_many_arguments)]
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
    http::{Method, Request, StatusCode, header},
    middleware,
};
use decomp_dev_core::config::Config;
use decomp_dev_db::Database;
use decomp_dev_github::GitHub;
use decomp_dev_jobs::{JobContext, JobStorage, create_monitor};
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

use crate::handlers::{build_router, csp::csp_middleware};

#[derive(Clone, FromRef)]
pub struct AppState {
    config: Arc<Config>,
    db: Arc<Database>,
    github: Arc<GitHub>,
    jobs: Arc<JobStorage>,
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

    let config: Arc<Config> = {
        let file = BufReader::new(File::open("config.yml").expect("Failed to open config file"));
        serde_yaml::from_reader(file).expect("Failed to parse config file")
    };
    let db = Database::new(&config.db).await.expect("Failed to open database");
    let github = GitHub::new(&config.github).await.expect("Failed to create GitHub client");
    let jobs = JobStorage::setup(&config.db).await.expect("Failed to set up job storage");

    let job_context = JobContext { config: config.clone(), db: db.clone(), github: github.clone() };
    let state = AppState { config: config.clone(), db: db.clone(), github, jobs };

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

    // Create the job monitor
    let monitor = create_monitor(state.jobs.clone(), job_context, &config.worker);

    // Build the router
    let port = state.config.server.port;
    let router = app(state, session_store).into_make_service_with_connect_info::<SocketAddr>();

    // Create the listener
    #[allow(unused_mut)]
    let mut listener = None;
    #[cfg(target_os = "linux")]
    {
        use std::os::fd::{FromRawFd, IntoRawFd};
        let fds = libsystemd::activation::receive_descriptors_with_names(false)
            .expect("Failed to receive fds");
        if let Some((fd, name)) = fds.into_iter().next() {
            tracing::info!("Listening on {}", name);
            let std_listener = unsafe { std::net::TcpListener::from_raw_fd(fd.into_raw_fd()) };
            std_listener.set_nonblocking(true).expect("Failed to set non-blocking");
            listener =
                Some(TcpListener::from_std(std_listener).expect("Failed to create listener"));
        }
    }
    let listener = match listener {
        Some(listener) => listener,
        None => {
            let addr = SocketAddr::from((Ipv4Addr::UNSPECIFIED, port));
            tracing::info!("Listening on {}", addr);
            TcpListener::bind(addr).await.expect("bind error")
        }
    };

    #[cfg(target_os = "linux")]
    {
        libsystemd::daemon::notify(false, &[libsystemd::daemon::NotifyState::Ready])
            .expect("Failed to notify");
    }

    // Run both the web server and job monitor concurrently, with graceful shutdown
    let web_server = async {
        axum::serve(listener, router)
            .with_graceful_shutdown(shutdown_signal())
            .await
            .map_err(|e| anyhow::anyhow!("Web server error: {e}"))
    };
    let job_monitor = async {
        monitor
            .run_with_signal(shutdown_signal_io())
            .await
            .map_err(|e| anyhow::anyhow!("Job monitor error: {e}"))
    };

    // Wait for both to complete gracefully (early return on error)
    if let Err(e) = tokio::try_join!(web_server, job_monitor) {
        tracing::error!("{e}");
    }

    #[cfg(target_os = "linux")]
    {
        libsystemd::daemon::notify(false, &[libsystemd::daemon::NotifyState::Stopping])
            .expect("Failed to notify");
    }

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
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            Duration::from_secs(120),
        ))
        .layer(CorsLayer::new().allow_methods([Method::GET]).allow_origin(cors::Any))
        .layer(
            SessionManagerLayer::new(session_store)
                .with_secure(false)
                .with_same_site(SameSite::Lax)
                .with_expiry(Expiry::OnInactivity(time::Duration::days(30))),
        )
        .layer(middleware::from_fn(csp_middleware))
        .compression();
    let router = build_router();
    #[cfg(debug_assertions)]
    let router = router.layer(tower_livereload::LiveReloadLayer::new());
    router.layer(middleware).with_state(state)
}

async fn shutdown_signal() { shutdown_signal_io().await.ok(); }

/// Shutdown signal that returns io::Result for apalis compatibility.
async fn shutdown_signal_io() -> std::io::Result<()> {
    #[cfg(unix)]
    {
        let mut sigterm = signal::unix::signal(signal::unix::SignalKind::terminate())?;
        tokio::select! {
            result = signal::ctrl_c() => result,
            _ = sigterm.recv() => Ok(()),
        }
    }
    #[cfg(not(unix))]
    {
        signal::ctrl_c().await
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
