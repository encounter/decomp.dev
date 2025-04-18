use anyhow::{anyhow, bail, Context};
use axum::{
    extract::{FromRef, FromRequestParts, OptionalFromRequestParts, Query, State},
    http::{header::ACCEPT, request::Parts, StatusCode},
    response::{IntoResponse, Redirect, Response},
};
use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use octocrab::{models::Author, Octocrab};
use rand::{rngs::OsRng, TryRngCore};
use time::{Duration, UtcDateTime};
use tower_sessions::Session;

use crate::{config::GitHubConfig, handlers::AppError, AppState};

const GITHUB_OAUTH_STATE: &str = "github_oauth_state";
const CURRENT_USER: &str = "current_user";

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct StoredOAuth {
    pub access_token: String,
    pub token_type: String,
    pub expires_at: Option<UtcDateTime>,
    pub refresh_token: Option<String>,
    pub refresh_token_expires_at: Option<UtcDateTime>,
}

impl From<StoredOAuth> for octocrab::auth::OAuth {
    fn from(value: StoredOAuth) -> Self {
        octocrab::auth::OAuth {
            access_token: value.access_token.into(),
            token_type: value.token_type,
            scope: Vec::new(),
            expires_in: None,
            refresh_token: None,
            refresh_token_expires_in: None,
        }
    }
}

#[derive(Clone, serde::Serialize, serde::Deserialize)]
pub struct CurrentUser {
    pub oauth: StoredOAuth,
    pub profile: Author,
}

pub async fn login(
    session: Session,
    State(state): State<AppState>,
    current_user: Option<CurrentUser>,
) -> Result<Response, AppError> {
    if current_user.is_some() {
        return Ok(Redirect::to("/").into_response());
    }
    let Some(config) = &state.config.github.oauth else {
        tracing::warn!("No GitHub OAuth config found");
        return Ok((StatusCode::INTERNAL_SERVER_ERROR, "No GitHub OAuth config").into_response());
    };
    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes)?;
    let nonce = URL_SAFE_NO_PAD.encode(bytes);
    session.insert(GITHUB_OAUTH_STATE, nonce.clone()).await?;
    let mut redirect_url = url::Url::parse("https://github.com/login/oauth/authorize")?;
    let mut query = redirect_url.query_pairs_mut();
    query.append_pair("client_id", &config.client_id);
    query.append_pair("redirect_uri", &config.redirect_uri);
    query.append_pair("state", &nonce);
    drop(query);
    Ok(Redirect::to(redirect_url.as_str()).into_response())
}

pub async fn logout(session: Session) -> Result<Response, AppError> {
    session.remove_value(CURRENT_USER).await?;
    session.remove_value(GITHUB_OAUTH_STATE).await?;
    Ok(Redirect::to("/").into_response())
}

#[derive(serde::Deserialize)]
pub struct OAuthQuery {
    pub code: String,
    pub state: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct OAuthResponse {
    pub access_token: String,
    pub token_type: String,
    pub expires_in: Option<i64>,
    pub refresh_token: Option<String>,
    pub refresh_token_expires_in: Option<i64>,
}

impl From<OAuthResponse> for octocrab::auth::OAuth {
    fn from(value: OAuthResponse) -> Self {
        octocrab::auth::OAuth {
            access_token: value.access_token.into(),
            token_type: value.token_type,
            scope: Vec::new(),
            expires_in: None,
            refresh_token: None,
            refresh_token_expires_in: None,
        }
    }
}

impl From<OAuthResponse> for StoredOAuth {
    fn from(value: OAuthResponse) -> Self {
        StoredOAuth {
            access_token: value.access_token,
            token_type: value.token_type,
            expires_at: value.expires_in.map(|s| UtcDateTime::now() + Duration::seconds(s)),
            refresh_token: value.refresh_token,
            refresh_token_expires_at: value
                .refresh_token_expires_in
                .map(|s| UtcDateTime::now() + Duration::seconds(s)),
        }
    }
}

#[derive(serde::Serialize)]
struct FetchAccessToken<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    code: &'a str,
}

#[derive(serde::Serialize)]
struct RefreshAccessToken<'a> {
    client_id: &'a str,
    client_secret: &'a str,
    grant_type: &'a str,
    refresh_token: &'a str,
}

pub async fn oauth(
    session: Session,
    Query(OAuthQuery { code, state: oauth_state }): Query<OAuthQuery>,
    State(state): State<AppState>,
) -> Result<Response, AppError> {
    let existing_state = session.get::<String>(GITHUB_OAUTH_STATE).await?;
    let Some(existing_state) = existing_state else {
        tracing::warn!("No state found in session");
        return Ok((StatusCode::BAD_REQUEST, "No state found").into_response());
    };
    if existing_state != oauth_state {
        tracing::warn!("State mismatch: expected {}, got {}", existing_state, oauth_state);
        return Ok((StatusCode::BAD_REQUEST, "State mismatch").into_response());
    }
    session.remove_value(GITHUB_OAUTH_STATE).await?;

    let current_user = fetch_access_token(&state.config.github, &code).await?;
    session.insert(CURRENT_USER, current_user).await?;

    Ok(Redirect::to("/").into_response())
}

fn oauth_client() -> Octocrab {
    Octocrab::builder()
        .base_uri("https://github.com")
        .expect("Failed to create base URI")
        .add_header(ACCEPT, "application/json".to_string())
        .build()
        .expect("Failed to create Octocrab client")
}

async fn fetch_access_token(config: &GitHubConfig, code: &str) -> Result<CurrentUser, AppError> {
    let Some(oauth_config) = &config.oauth else {
        tracing::warn!("No GitHub OAuth config found");
        return Err(AppError::Internal(anyhow!("No GitHub OAuth config")));
    };
    let base_client = oauth_client();
    let oauth: OAuthResponse = base_client
        .post(
            "/login/oauth/access_token",
            Some(&FetchAccessToken {
                client_id: &oauth_config.client_id,
                client_secret: &oauth_config.client_secret,
                code,
            }),
        )
        .await?;
    let oauth = StoredOAuth::from(oauth);
    let client = Octocrab::builder().oauth(oauth.clone().into()).build()?;
    let profile = client.current().user().await.context("Failed to fetch current user")?;
    tracing::info!("Logged in as @{}", profile.login);
    Ok(CurrentUser { oauth, profile })
}

async fn refresh_access_token(
    config: &GitHubConfig,
    refresh_token: &str,
) -> Result<CurrentUser, anyhow::Error> {
    let Some(oauth_config) = &config.oauth else {
        tracing::warn!("No GitHub OAuth config found");
        bail!("No GitHub OAuth config found");
    };
    let base_client = oauth_client();
    let oauth: OAuthResponse = base_client
        .post(
            "/login/oauth/access_token",
            Some(&RefreshAccessToken {
                client_id: &oauth_config.client_id,
                client_secret: &oauth_config.client_secret,
                grant_type: "refresh_token",
                refresh_token,
            }),
        )
        .await?;
    let oauth = StoredOAuth::from(oauth);
    let client = Octocrab::builder().oauth(oauth.clone().into()).build()?;
    let profile = client.current().user().await.context("Failed to fetch current user")?;
    tracing::info!("Refreshed token for @{}", profile.login);
    Ok(CurrentUser { oauth, profile })
}

impl<S> FromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(parts: &mut Parts, state: &S) -> Result<Self, Self::Rejection> {
        <CurrentUser as OptionalFromRequestParts<S>>::from_request_parts(parts, state)
            .await?
            .ok_or((StatusCode::UNAUTHORIZED, "Unauthorized"))
    }
}

impl<S> OptionalFromRequestParts<S> for CurrentUser
where
    AppState: FromRef<S>,
    S: Send + Sync,
{
    type Rejection = (StatusCode, &'static str);

    async fn from_request_parts(
        parts: &mut Parts,
        state: &S,
    ) -> Result<Option<Self>, Self::Rejection> {
        let session = Session::from_request_parts(parts, state).await?;
        let app_state = AppState::from_ref(state);
        let Some(user) = session.get::<CurrentUser>(CURRENT_USER).await.ok().flatten() else {
            return Ok(None);
        };
        if let Some(expires_at) = user.oauth.expires_at {
            if (UtcDateTime::now() + Duration::seconds(30)) > expires_at {
                // Access token expired, attempt to refresh
                if let Err(e) = session.remove_value(CURRENT_USER).await {
                    tracing::error!("Failed to remove user from session: {}", e);
                };
                if let Some(refresh_token) = &user.oauth.refresh_token {
                    if let Some(refresh_token_expires_at) = user.oauth.refresh_token_expires_at {
                        if UtcDateTime::now() >= refresh_token_expires_at {
                            // Refresh token expired
                            return Ok(None);
                        }
                    }
                    let current_user =
                        match refresh_access_token(&app_state.config.github, refresh_token).await {
                            Ok(current_user) => current_user,
                            Err(e) => {
                                tracing::error!("Failed to refresh access token: {:?}", e);
                                return Ok(None);
                            }
                        };
                    if let Err(e) = session.insert(CURRENT_USER, current_user.clone()).await {
                        tracing::error!("Failed to insert user into session: {}", e);
                    }
                    return Ok(Some(current_user));
                }
                return Ok(None);
            }
        }
        Ok(Some(user))
    }
}
