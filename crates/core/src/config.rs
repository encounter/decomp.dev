use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct Config {
    pub server: ServerConfig,
    pub db: DbConfig,
    pub github: GitHubConfig,
    pub openai: Option<OpenAiConfig>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct ServerConfig {
    pub port: u16,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct DbConfig {
    pub url: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubConfig {
    pub token: String,
    pub app: Option<GitHubAppConfig>,
    pub oauth: Option<GitHubOAuthConfig>,
    #[serde(default)]
    pub super_admin_ids: Vec<u64>,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubAppConfig {
    pub id: u64,
    pub webhook_secret: String,
    pub private_key: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct GitHubOAuthConfig {
    pub client_id: String,
    pub client_secret: String,
    pub redirect_uri: String,
}

#[derive(Debug, Clone, Deserialize, Serialize)]
pub struct OpenAiConfig {
    pub api_key: String,
}
