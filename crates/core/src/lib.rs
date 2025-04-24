pub mod config;
pub mod models;
pub mod util;

use std::{convert::Infallible, net::SocketAddr};

use axum::{
    Extension,
    extract::{ConnectInfo, FromRequestParts, OriginalUri},
    http::{StatusCode, Uri, header, request::Parts},
    response::{IntoResponse, Response},
};

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
