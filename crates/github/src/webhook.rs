use std::{fmt::Display, sync::Arc};

use anyhow::Result;
use axum::{
    body::Bytes,
    extract::{FromRef, FromRequest, Request},
    http::StatusCode,
    response::{IntoResponse, Response},
};
use decomp_dev_core::config::Config;
use hmac::{Hmac, Mac};
use octocrab::models::webhook_events::WebhookEvent;
use sha2::Sha256;

/// Verify and extract GitHub Event Payload.
#[derive(Clone)]
#[must_use]
pub struct GitHubEvent {
    pub event: WebhookEvent,
}

impl<S> FromRequest<S> for GitHubEvent
where
    Arc<Config>: FromRef<S>,
    S: Send + Sync + Clone,
{
    type Rejection = Response;

    async fn from_request(req: Request, state: &S) -> Result<Self, Self::Rejection> {
        fn err(m: impl Display) -> Response {
            tracing::error!("{m}");
            (StatusCode::BAD_REQUEST, m.to_string()).into_response()
        }
        let event = req
            .headers()
            .get("X-GitHub-Event")
            .and_then(|v| v.to_str().ok())
            .ok_or_else(|| err("X-GitHub-Event header missing"))?
            .to_string();
        let config = <Arc<Config>>::from_ref(state);
        let body = if let Some(app_config) = &config.github.app {
            let signature_sha256 = req
                .headers()
                .get("X-Hub-Signature-256")
                .and_then(|v| v.to_str().ok())
                .ok_or_else(|| err("X-Hub-Signature-256 missing"))?
                .strip_prefix("sha256=")
                .ok_or_else(|| err("X-Hub-Signature-256 sha256= prefix missing"))?;
            let signature =
                hex::decode(signature_sha256).map_err(|_| err("X-Hub-Signature-256 malformed"))?;
            let body =
                Bytes::from_request(req, state).await.map_err(|_| err("error reading body"))?;
            let mut mac = Hmac::<Sha256>::new_from_slice(app_config.webhook_secret.as_bytes())
                .expect("HMAC can take key of any size");
            mac.update(&body);
            if mac.verify_slice(&signature).is_err() {
                return Err(err("signature mismatch"));
            }
            body
        } else {
            Bytes::from_request(req, state).await.map_err(|_| err("error reading body"))?
        };
        let value = WebhookEvent::try_from_header_and_body(&event, &body)
            .map_err(|_| err("error parsing body"))?;
        Ok(GitHubEvent { event: value })
    }
}
