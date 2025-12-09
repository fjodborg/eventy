//! OAuth state and Discord API interactions

use serde::Deserialize;

/// OAuth configuration
#[derive(Clone)]
pub struct OAuthState {
    pub client_id: String,
    pub client_secret: String,
    pub bot_token: String,
    pub base_url: String,
    pub http_client: reqwest::Client,
}

impl OAuthState {
    pub fn from_env() -> Option<Self> {
        let client_id = std::env::var("DISCORD_CLIENT_ID").ok()?;
        let client_secret = std::env::var("DISCORD_CLIENT_SECRET").ok()?;
        let bot_token = std::env::var("DISCORD_BOT_TOKEN")
            .or_else(|_| std::env::var("DISCORD_TOKEN"))
            .ok()?;
        let base_url = std::env::var("WEB_BASE_URL")
            .unwrap_or_else(|_| "http://localhost:3000".to_string());

        Some(Self {
            client_id,
            client_secret,
            bot_token,
            base_url,
            http_client: reqwest::Client::new(),
        })
    }

    pub fn redirect_uri(&self) -> String {
        format!("{}/callback", self.base_url)
    }
}

/// Discord OAuth token response
#[derive(Deserialize)]
pub struct TokenResponse {
    pub access_token: String,
    pub token_type: String,
}

/// Discord user info from /users/@me
#[derive(Deserialize, Debug)]
pub struct DiscordUser {
    pub id: String,
    pub username: String,
    pub global_name: Option<String>,
    pub discriminator: String,
}
