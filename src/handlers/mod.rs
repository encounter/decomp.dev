use std::{convert::Infallible, net::SocketAddr, str::FromStr};

use axum::{
    extract::{ConnectInfo, FromRequestParts, OriginalUri},
    http::{header, request::Parts, HeaderMap, StatusCode, Uri},
    response::{IntoResponse, Response},
    routing::{get, post},
    Extension, Router,
};
use bytes::BytesMut;
use mime::Mime;
use prost::Message;

use crate::AppState;

mod assets;
mod auth;
mod badge;
mod css;
mod github;
mod js;
mod og;
mod project;
mod report;
mod treemap;

pub fn build_router() -> Router<AppState> {
    Router::new()
        .route("/api/github/webhook", post(github::webhook))
        .route("/api/github/oauth", get(auth::oauth))
        .route("/login", get(auth::login))
        .route("/logout", post(auth::logout))
        .route("/css/{*filename}", get(css::get_css))
        .route("/js/{*filename}", get(js::get_js))
        .route("/assets/{*filename}", get(assets::get_asset))
        .route("/og.png", get(og::get_og))
        .route("/", get(project::get_projects))
        .route("/{owner}/{repo}", get(report::get_report))
        .route("/{owner}/{repo}/{version}", get(report::get_report))
        .route("/{owner}/{repo}/{version}/{commit}", get(report::get_report))
}

pub enum AppError {
    Status(StatusCode),
    Internal(anyhow::Error),
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        match self {
            Self::Status(status) if status == StatusCode::NOT_FOUND => {
                (status, "Not found").into_response()
            }
            Self::Status(status) => status.into_response(),
            Self::Internal(err) => {
                tracing::error!("{:?}", err);
                (StatusCode::INTERNAL_SERVER_ERROR, format!("Something went wrong: {}", err))
                    .into_response()
            }
        }
    }
}

impl<E: Into<anyhow::Error>> From<E> for AppError {
    fn from(err: E) -> Self { Self::Internal(err.into()) }
}

pub fn parse_accept(headers: &HeaderMap, ext: Option<&str>) -> Vec<Mime> {
    // Explicit extension takes precedence
    if let Some(ext) = ext {
        return match ext.to_ascii_lowercase().as_str() {
            "json" => vec![mime::APPLICATION_JSON],
            "binpb" | "proto" => vec![Mime::from_str("application/x-protobuf").unwrap()],
            "svg" => vec![mime::IMAGE_SVG],
            _ => {
                if let Some(format) = image::ImageFormat::from_extension(ext) {
                    vec![Mime::from_str(format.to_mime_type()).unwrap()]
                } else {
                    // An unknown extension should be NOT_ACCEPTABLE, not */*.
                    vec![]
                }
            }
        };
    }
    // Otherwise, parse the Accept header
    let result = headers
        .get(header::ACCEPT)
        .and_then(|value| value.to_str().ok())
        .iter()
        .flat_map(|s| s.split(','))
        .map(|s| s.trim())
        .filter_map(|s| Mime::from_str(s).ok())
        .collect::<Vec<_>>();
    if result.is_empty() {
        // If no Accept header is present, use */*
        vec![mime::STAR_STAR]
    } else {
        result
    }
}

pub struct Protobuf<'a, T>(pub &'a T)
where T: Message;

pub const APPLICATION_PROTOBUF: &str = "application/x-protobuf";
pub const PROTOBUF: &str = "x-protobuf";

impl<T: Message> IntoResponse for Protobuf<'_, T> {
    fn into_response(self) -> Response {
        let mut bytes = BytesMut::with_capacity(self.0.encoded_len());
        self.0.encode(&mut bytes).unwrap();
        ([(header::CONTENT_TYPE, APPLICATION_PROTOBUF)], bytes.freeze()).into_response()
    }
}

/// Extractor for the full URI of the request, including the scheme and authority.
/// Uses the `x-forwarded-proto` and `x-forwarded-host` headers if present.
pub struct FullUri(pub Uri);

impl<S> FromRequestParts<S> for FullUri
where S: Send + Sync
{
    type Rejection = Infallible;

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        let uri = Extension::<OriginalUri>::from_request_parts(parts, state)
            .await
            .map_or_else(|_| parts.uri.clone(), |Extension(OriginalUri(uri))| uri);
        let mut builder = Uri::builder();
        if let Some(scheme) =
            parts.headers.get("x-forwarded-proto").and_then(|value| value.to_str().ok())
        {
            builder = builder.scheme(scheme);
        } else if let Some(scheme) = uri.scheme().cloned() {
            builder = builder.scheme(scheme);
        } else {
            // TODO: native https?
            builder = builder.scheme("http");
        }
        if let Some(host) =
            parts.headers.get("x-forwarded-host").and_then(|value| value.to_str().ok())
        {
            builder = builder.authority(host);
        } else if let Some(host) =
            parts.headers.get(header::HOST).and_then(|value| value.to_str().ok())
        {
            builder = builder.authority(host);
        } else if let Some(authority) = uri.authority().cloned() {
            builder = builder.authority(authority);
        } else if let Ok(ConnectInfo(socket_addr)) =
            ConnectInfo::<SocketAddr>::from_request_parts(parts, state).await
        {
            builder = builder.authority(socket_addr.to_string());
        }
        if let Some(path_and_query) = uri.path_and_query().cloned() {
            builder = builder.path_and_query(path_and_query);
        }
        Ok(FullUri(builder.build().unwrap_or(uri)))
    }
}
