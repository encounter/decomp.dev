use axum::{
    extract::{Query, State},
    http::{HeaderMap, StatusCode, header::REFERER},
    response::{IntoResponse, Redirect, Response},
};
use decomp_dev_auth::{CURRENT_USER, CurrentUser, GITHUB_OAUTH_STATE, RETURN_TO, generate_nonce};
use decomp_dev_core::{AppError, config::Config, util::UrlExt};
use decomp_dev_github::graphql::CurrentUserResponse;
use maud::{DOCTYPE, html};
use tower_sessions::Session;

use crate::handlers::common::{Load, TemplateContext};

#[derive(serde::Deserialize)]
pub struct LoginQuery {
    pub return_to: Option<String>,
}

pub async fn login(
    session: Session,
    headers: HeaderMap,
    Query(query): Query<LoginQuery>,
    State(config): State<Config>,
    current_user: Option<CurrentUser>,
    mut ctx: TemplateContext,
) -> Result<Response, AppError> {
    if current_user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    let Some(config) = &config.github.oauth else {
        // Dev mode override
        if config.server.dev_mode {
            session
                .insert(CURRENT_USER, CurrentUser {
                    oauth: None,
                    data: CurrentUserResponse {
                        id: u64::MAX,
                        login: "devuser".to_string(),
                        url: String::new(),
                        repositories: vec![],
                    },
                    super_admin: true,
                })
                .await?;
            return Ok(Redirect::to("/").into_response());
        }
        tracing::warn!("No GitHub OAuth config found");
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "No GitHub OAuth config").into_response());
    };
    let oauth_state = generate_nonce();
    session.insert(GITHUB_OAUTH_STATE, oauth_state.clone()).await?;
    if let Some(return_to) = calc_return_to(&headers, query) {
        session.insert(RETURN_TO, return_to).await?;
    }
    let mut redirect_url = url::Url::parse("https://github.com/login/oauth/authorize")?;
    {
        let mut query = redirect_url.query_pairs_mut();
        query.append_pair("client_id", &config.client_id);
        query.append_pair("redirect_uri", &config.redirect_uri);
        query.append_pair("state", &oauth_state);
    }
    let rendered = html! {
        (DOCTYPE)
        html lang="en" {
            head {
                meta charset="utf-8";
                title { "Logging in... • decomp.dev" }
                meta http-equiv="refresh" content=(format!("0;URL={redirect_url}"));
                (ctx.header().await)
                (ctx.chunks("main", Load::Deferred).await)
            }
            body {
                .loading-container {
                    div aria-busy="true" { "Logging in..." }
                }
            }
        }
    };
    Ok((ctx, rendered).into_response())
}

pub async fn logout(
    session: Session,
    headers: HeaderMap,
    Query(query): Query<LoginQuery>,
) -> Result<Response, AppError> {
    session.flush().await?;
    if let Some(return_to) = calc_return_to(&headers, query) {
        return Ok(Redirect::to(&return_to).into_response());
    }
    Ok(Redirect::to("/").into_response())
}

fn calc_return_to(headers: &HeaderMap, LoginQuery { return_to }: LoginQuery) -> Option<String> {
    let mut return_to = return_to.or_else(|| {
        if headers.get("sec-fetch-site").and_then(|h| h.to_str().ok()) == Some("same-origin") {
            headers
                .get(REFERER)
                .and_then(|h| h.to_str().ok())
                .and_then(|s| url::Url::parse(s).ok())
                .map(|u| u.path_and_query().to_string())
        } else {
            None
        }
    });
    return_to.take_if(|s| s.starts_with('/'))
}
