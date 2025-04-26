use axum::{
    extract::{Query, State},
    http::StatusCode,
    response::{IntoResponse, Redirect, Response},
};
use decomp_dev_auth::{CurrentUser, GITHUB_OAUTH_STATE, RETURN_TO, generate_nonce};
use decomp_dev_core::{AppError, config::GitHubConfig};
use maud::{DOCTYPE, html};
use tower_sessions::Session;

use crate::handlers::common::{chunks, header};

#[derive(serde::Deserialize)]
pub struct LoginQuery {
    pub return_to: Option<String>,
}

pub async fn login(
    session: Session,
    Query(LoginQuery { return_to }): Query<LoginQuery>,
    State(config): State<GitHubConfig>,
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    if current_user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    let Some(config) = &config.oauth else {
        tracing::warn!("No GitHub OAuth config found");
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "No GitHub OAuth config").into_response());
    };
    let nonce = generate_nonce();
    session.insert(GITHUB_OAUTH_STATE, nonce.clone()).await?;
    if let Some(return_to) = return_to {
        if return_to.starts_with('/') {
            session.insert(RETURN_TO, return_to).await?;
        }
    }
    let mut redirect_url = url::Url::parse("https://github.com/login/oauth/authorize")?;
    {
        let mut query = redirect_url.query_pairs_mut();
        query.append_pair("client_id", &config.client_id);
        query.append_pair("redirect_uri", &config.redirect_uri);
        query.append_pair("state", &nonce);
    }
    Ok(html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Logging in... • decomp.dev" }
                meta http-equiv="refresh" content=(format!("0;URL={redirect_url}"));
                (header())
                (chunks("main", true).await)
            }
            body {
                .loading-container {
                    div aria-busy="true" { "Logging in..." }
                }
            }
        }
    }
    .into_response())
}

pub async fn logout(session: Session) -> Result<Response, AppError> {
    session.flush().await?;
    Ok(Redirect::to("/").into_response())
}
