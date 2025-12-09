//! OAuth Test Binary
//!
//! A minimal standalone web server to test Discord OAuth verification flow.
//!
//! Run with: cargo run --bin oauth_test
//!
//! Required environment variables:
//! - DISCORD_CLIENT_ID: Your bot's client ID
//! - DISCORD_CLIENT_SECRET: Your bot's client secret
//! - DISCORD_BOT_TOKEN: Your bot token (for adding users to guild)
//! - DISCORD_GUILD_ID: The guild to add users to
//! - WEB_BASE_URL: Base URL for OAuth redirect (default: http://localhost:3000)

use axum::{
    extract::{Path, Query, State},
    response::Html,
    routing::get,
    Router,
};
use serde::Deserialize;
use std::sync::Arc;
use tokio::net::TcpListener;
use tracing::{error, info};

/// Application state shared across handlers
#[derive(Clone)]
struct AppState {
    client_id: String,
    client_secret: String,
    bot_token: String,
    guild_id: String,
    base_url: String,
    http_client: reqwest::Client,
}

/// Query parameters from Discord OAuth callback
#[derive(Deserialize)]
struct CallbackParams {
    code: String,
    state: String, // This contains the UUID
}

/// Discord OAuth token response
#[derive(Deserialize)]
struct TokenResponse {
    access_token: String,
    token_type: String,
}

/// Discord user info from /users/@me
#[derive(Deserialize, Debug)]
struct DiscordUser {
    id: String,
    username: String,
    global_name: Option<String>,
    #[allow(dead_code)]
    discriminator: String,
}

/// Health check endpoint
async fn health() -> &'static str {
    "OAuth Test Server Running"
}

/// GET /verify/{uuid} - Show verification page
async fn verify_page(State(state): State<Arc<AppState>>, Path(uuid): Path<String>) -> Html<String> {
    info!("Verification page requested for UUID: {}", uuid);

    // Build redirect URI - must match EXACTLY what's registered in Discord Developer Portal
    let redirect_uri = format!("{}/callback", state.base_url);

    let oauth_url = format!(
        "https://discord.com/oauth2/authorize\
        ?client_id={}\
        &redirect_uri={}\
        &response_type=code\
        &scope=identify%20guilds.join\
        &state={}",
        state.client_id,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&uuid)
    );

    info!("Generated OAuth URL: {}", oauth_url);
    info!("Redirect URI (before encoding): {}", redirect_uri);

    Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Discord Verification</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #667eea 0%, #764ba2 100%);
        }}
        .container {{
            background: white;
            padding: 40px;
            border-radius: 16px;
            box-shadow: 0 10px 40px rgba(0,0,0,0.2);
            text-align: center;
            max-width: 400px;
        }}
        h1 {{
            color: #333;
            margin-bottom: 10px;
        }}
        p {{
            color: #666;
            margin-bottom: 30px;
        }}
        .uuid {{
            background: #f5f5f5;
            padding: 10px;
            border-radius: 8px;
            font-family: monospace;
            font-size: 12px;
            word-break: break-all;
            margin-bottom: 20px;
        }}
        .discord-btn {{
            display: inline-block;
            background: #5865F2;
            color: white;
            padding: 15px 30px;
            border-radius: 8px;
            text-decoration: none;
            font-weight: 600;
            font-size: 16px;
            transition: background 0.2s;
        }}
        .discord-btn:hover {{
            background: #4752C4;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Verify Your Account</h1>
        <p>Click the button below to verify with Discord and join the server.</p>
        <div class="uuid">Verification ID: {uuid}</div>
        <a href="{oauth_url}" class="discord-btn">
            Login with Discord
        </a>
    </div>
</body>
</html>"#,
        uuid = uuid,
        oauth_url = oauth_url
    ))
}

/// GET /callback - OAuth callback handler
async fn oauth_callback(
    State(state): State<Arc<AppState>>,
    Query(params): Query<CallbackParams>,
) -> Result<Html<String>, Html<String>> {
    info!("OAuth callback received for UUID: {}", params.state);

    // Exchange code for access token
    let token_response = state
        .http_client
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", state.client_id.as_str()),
            ("client_secret", state.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", params.code.as_str()),
            ("redirect_uri", &format!("{}/callback", state.base_url)),
        ])
        .send()
        .await
        .map_err(|e| {
            error!("Failed to exchange code: {}", e);
            Html(error_page("Failed to exchange authorization code"))
        })?;

    if !token_response.status().is_success() {
        let error_text = token_response.text().await.unwrap_or_default();
        error!("Token exchange failed: {}", error_text);
        return Err(Html(error_page("Token exchange failed")));
    }

    let token: TokenResponse = token_response.json().await.map_err(|e| {
        error!("Failed to parse token response: {}", e);
        Html(error_page("Failed to parse token response"))
    })?;

    info!("Got access token, fetching user info...");

    // Get user info
    let user_response = state
        .http_client
        .get("https://discord.com/api/users/@me")
        .header(
            "Authorization",
            format!("{} {}", token.token_type, token.access_token),
        )
        .send()
        .await
        .map_err(|e| {
            error!("Failed to get user info: {}", e);
            Html(error_page("Failed to get user info"))
        })?;

    if !user_response.status().is_success() {
        let error_text = user_response.text().await.unwrap_or_default();
        error!("User info request failed: {}", error_text);
        return Err(Html(error_page("Failed to get user info")));
    }

    let user: DiscordUser = user_response.json().await.map_err(|e| {
        error!("Failed to parse user info: {}", e);
        Html(error_page("Failed to parse user info"))
    })?;

    info!("User authenticated: {} ({})", user.username, user.id);

    // Try to add user to guild using bot token
    let add_result = state
        .http_client
        .put(&format!(
            "https://discord.com/api/guilds/{}/members/{}",
            state.guild_id, user.id
        ))
        .header("Authorization", format!("Bot {}", state.bot_token))
        .json(&serde_json::json!({
            "access_token": token.access_token
        }))
        .send()
        .await;

    let guild_status = match add_result {
        Ok(response) => {
            if response.status().is_success() || response.status().as_u16() == 204 {
                "Added to server!"
            } else if response.status().as_u16() == 201 {
                "Added to server!"
            } else {
                let status = response.status();
                let text = response.text().await.unwrap_or_default();
                info!("Guild add response: {} - {}", status, text);
                "Already in server or couldn't add"
            }
        }
        Err(e) => {
            error!("Failed to add to guild: {}", e);
            "Couldn't add to server"
        }
    };

    let display_name = user.global_name.as_deref().unwrap_or(&user.username);

    Ok(Html(format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Verification Success</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #11998e 0%, #38ef7d 100%);
        }}
        .container {{
            background: white;
            padding: 40px;
            border-radius: 16px;
            box-shadow: 0 10px 40px rgba(0,0,0,0.2);
            text-align: center;
            max-width: 400px;
        }}
        h1 {{
            color: #11998e;
            margin-bottom: 10px;
        }}
        .success-icon {{
            font-size: 60px;
            margin-bottom: 20px;
        }}
        .info {{
            background: #f5f5f5;
            padding: 15px;
            border-radius: 8px;
            margin: 20px 0;
            text-align: left;
        }}
        .info-row {{
            display: flex;
            justify-content: space-between;
            padding: 5px 0;
            border-bottom: 1px solid #eee;
        }}
        .info-row:last-child {{
            border-bottom: none;
        }}
        .label {{
            color: #888;
        }}
        .value {{
            color: #333;
            font-weight: 500;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="success-icon">✓</div>
        <h1>Verification Successful!</h1>
        <p>You've been verified and added to the server.</p>

        <div class="info">
            <div class="info-row">
                <span class="label">Discord User:</span>
                <span class="value">{display_name}</span>
            </div>
            <div class="info-row">
                <span class="label">Discord ID:</span>
                <span class="value">{discord_id}</span>
            </div>
            <div class="info-row">
                <span class="label">Verification UUID:</span>
                <span class="value" style="font-size: 11px;">{uuid}</span>
            </div>
            <div class="info-row">
                <span class="label">Server Status:</span>
                <span class="value">{guild_status}</span>
            </div>
        </div>

        <p style="color: #888; font-size: 14px;">You can now close this window.</p>
    </div>
</body>
</html>"#,
        display_name = display_name,
        discord_id = user.id,
        uuid = params.state,
        guild_status = guild_status
    )))
}

fn error_page(message: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Verification Error</title>
    <style>
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            display: flex;
            justify-content: center;
            align-items: center;
            min-height: 100vh;
            margin: 0;
            background: linear-gradient(135deg, #f093fb 0%, #f5576c 100%);
        }}
        .container {{
            background: white;
            padding: 40px;
            border-radius: 16px;
            box-shadow: 0 10px 40px rgba(0,0,0,0.2);
            text-align: center;
            max-width: 400px;
        }}
        h1 {{
            color: #f5576c;
        }}
        .error-icon {{
            font-size: 60px;
            margin-bottom: 20px;
        }}
        .message {{
            background: #fff5f5;
            padding: 15px;
            border-radius: 8px;
            color: #c53030;
            margin: 20px 0;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="error-icon">✕</div>
        <h1>Verification Failed</h1>
        <div class="message">{message}</div>
        <p style="color: #888; font-size: 14px;">Please try again or contact an administrator.</p>
    </div>
</body>
</html>"#,
        message = message
    )
}

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    // Initialize logging with DEBUG level
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    // Load environment
    dotenv::dotenv().ok();

    let client_id = std::env::var("DISCORD_CLIENT_ID").expect("DISCORD_CLIENT_ID must be set");
    let client_secret =
        std::env::var("DISCORD_CLIENT_SECRET").expect("DISCORD_CLIENT_SECRET must be set");
    let bot_token = std::env::var("DISCORD_BOT_TOKEN")
        .or_else(|_| std::env::var("DISCORD_TOKEN"))
        .expect("DISCORD_BOT_TOKEN or DISCORD_TOKEN must be set");
    let guild_id = std::env::var("DISCORD_GUILD_ID").expect("DISCORD_GUILD_ID must be set");
    let base_url =
        std::env::var("WEB_BASE_URL").unwrap_or_else(|_| "http://localhost:3000".to_string());

    info!("Starting OAuth test server...");
    info!("Client ID: {}", client_id);
    info!("Guild ID: {}", guild_id);
    info!("Base URL: {}", base_url);

    let state = Arc::new(AppState {
        client_id,
        client_secret,
        bot_token,
        guild_id,
        base_url: base_url.clone(),
        http_client: reqwest::Client::new(),
    });

    let app = Router::new()
        .route("/", get(health))
        .route("/verify/:uuid", get(verify_page))
        .route("/callback", get(oauth_callback))
        .with_state(state);

    let listener = TcpListener::bind("0.0.0.0:3000").await?;
    info!("Server listening on http://localhost:3000");
    info!("Test with: http://localhost:3000/verify/test-uuid-123");

    axum::serve(listener, app).await?;

    Ok(())
}
