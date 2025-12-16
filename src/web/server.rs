//! Web server implementation for OAuth verification

use axum::{
    extract::{Host, Path, Query, State},
    handler::HandlerWithoutStateExt,
    http::{StatusCode, Uri},
    response::{Html, Redirect},
    routing::get,
    BoxError, Router,
};
use axum_server::tls_rustls::RustlsConfig;
use poise::serenity_prelude::{self as serenity, GuildId, UserId};
use serde::Deserialize;
use std::{net::SocketAddr, path::PathBuf, sync::Arc};
use tracing::{debug, error, info, warn};

use super::admin::{admin_router, AdminState};
use super::auth::SharedSessionStore;
use super::oauth::{DiscordUser, OAuthState, TokenResponse};
use crate::logging::SharedLogBuffer;
use crate::managers::{SharedChannelManager, SharedConfigManager, SharedRoleManager, SharedVerificationManager};
use crate::state::{SharedUserDatabase, TrackedUser};

/// Web server configuration
pub struct WebServerConfig {
    /// HTTPS port (main server)
    pub https_port: u16,
    /// HTTP port (redirects to HTTPS)
    pub http_port: u16,
    /// Path to certificate PEM file (cert + CA bundle)
    pub cert_path: PathBuf,
    /// Path to private key PEM file
    pub key_path: PathBuf,
}

impl Default for WebServerConfig {
    fn default() -> Self {
        Self {
            https_port: 443,
            http_port: 80,
            cert_path: PathBuf::from("certs/cert.pem"),
            key_path: PathBuf::from("certs/key.pem"),
        }
    }
}

impl WebServerConfig {
    /// Create config from environment variables
    pub fn from_env() -> Self {
        Self {
            https_port: std::env::var("HTTPS_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(443),
            http_port: std::env::var("HTTP_PORT")
                .ok()
                .and_then(|s| s.parse().ok())
                .unwrap_or(80),
            cert_path: std::env::var("TLS_CERT_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("certs/cert.pem")),
            key_path: std::env::var("TLS_KEY_PATH")
                .map(PathBuf::from)
                .unwrap_or_else(|_| PathBuf::from("certs/key.pem")),
        }
    }
}

/// Ports configuration for HTTP to HTTPS redirect
#[derive(Clone, Copy)]
struct Ports {
    http: u16,
    https: u16,
}

/// Shared state for web handlers
#[derive(Clone)]
pub struct AppState {
    pub oauth: OAuthState,
    pub config_manager: SharedConfigManager,
    pub role_manager: SharedRoleManager,
    pub verification_manager: SharedVerificationManager,
    pub serenity_http: Arc<serenity::Http>,
}

/// Query parameters from Discord OAuth callback
#[derive(Deserialize)]
pub struct CallbackParams {
    code: String,
    state: String, // This contains the UUID
}

/// Start the web server for OAuth verification and admin panel
pub async fn start_web_server(
    config: WebServerConfig,
    oauth: OAuthState,
    config_manager: SharedConfigManager,
    role_manager: SharedRoleManager,
    verification_manager: SharedVerificationManager,
    channel_manager: SharedChannelManager,
    user_database: SharedUserDatabase,
    serenity_http: Arc<serenity::Http>,
    session_store: SharedSessionStore,
    log_buffer: SharedLogBuffer,
    guild_id: GuildId,
) -> anyhow::Result<()> {
    let state = AppState {
        oauth: oauth.clone(),
        config_manager: config_manager.clone(),
        role_manager,
        verification_manager,
        serenity_http: serenity_http.clone(),
    };

    // Capture base_url before moving oauth into admin_state
    let base_url = oauth.base_url.clone();

    // Create admin state
    let admin_state = AdminState {
        oauth,
        config_manager,
        channel_manager,
        role_manager: state.role_manager.clone(),
        user_database,
        session_store,
        log_buffer,
        serenity_http,
        guild_id,
    };

    let app = Router::new()
        .route("/", get(health))
        .route("/verify/:uuid", get(verify_page))
        .route("/callback", get(oauth_callback))
        .with_state(state)
        .nest("/admin", admin_router(admin_state));

    let ports = Ports {
        http: config.http_port,
        https: config.https_port,
    };

    // Load TLS configuration
    let cert_path = config.cert_path.canonicalize().unwrap_or_else(|_| config.cert_path.clone());
    let key_path = config.key_path.canonicalize().unwrap_or_else(|_| config.key_path.clone());

    info!("Loading TLS certificates:");
    info!("  Certificate: {}", cert_path.display());
    info!("  Private key: {}", key_path.display());

    // Check if files exist
    if !config.cert_path.exists() {
        return Err(anyhow::anyhow!(
            "Certificate file not found: {}",
            cert_path.display()
        ));
    }
    if !config.key_path.exists() {
        return Err(anyhow::anyhow!(
            "Private key file not found: {}",
            key_path.display()
        ));
    }

    let tls_config = RustlsConfig::from_pem_file(&config.cert_path, &config.key_path)
        .await
        .map_err(|e| anyhow::anyhow!(
            "Failed to load TLS certificates: {}\n  Certificate: {}\n  Private key: {}\n\nHint: The private key must be in PKCS#8 PEM format. If you have an RSA key, convert it with:\n  openssl pkcs8 -topk8 -inform PEM -outform PEM -nocrypt -in private.key -out key.pem",
            e, cert_path.display(), key_path.display()
        ))?;

    // Spawn HTTP to HTTPS redirect server
    tokio::spawn(redirect_http_to_https(ports));

    let https_addr = SocketAddr::from(([0, 0, 0, 0], config.https_port));
    info!("Web server listening on https://0.0.0.0:{}", config.https_port);
    info!("HTTP redirect server on http://0.0.0.0:{}", config.http_port);
    info!("Admin panel available at https://<your-domain>/admin");
    info!("=== Discord OAuth Configuration ===");
    info!("Add these Redirect URIs in Discord Developer Portal:");
    info!("  1. {}/callback        (for user verification)", base_url);
    info!("  2. {}/admin/callback  (for admin login)", base_url);
    info!("Portal: https://discord.com/developers/applications -> OAuth2 -> Redirects");

    axum_server::bind_rustls(https_addr, tls_config)
        .serve(app.into_make_service())
        .await?;

    Ok(())
}

/// Redirect all HTTP requests to HTTPS
async fn redirect_http_to_https(ports: Ports) {
    fn make_https(host: &str, uri: Uri, https_port: u16) -> Result<Uri, BoxError> {
        let mut parts = uri.into_parts();

        parts.scheme = Some(axum::http::uri::Scheme::HTTPS);

        if parts.path_and_query.is_none() {
            parts.path_and_query = Some("/".parse().unwrap());
        }

        let authority: axum::http::uri::Authority = host.parse()?;
        let bare_host = match authority.port() {
            Some(port_struct) => authority
                .as_str()
                .strip_suffix(port_struct.as_str())
                .unwrap()
                .strip_suffix(':')
                .unwrap(),
            None => authority.as_str(),
        };

        // Only add port if it's not the default HTTPS port
        if https_port == 443 {
            parts.authority = Some(bare_host.parse()?);
        } else {
            parts.authority = Some(format!("{bare_host}:{https_port}").parse()?);
        }

        Ok(Uri::from_parts(parts)?)
    }

    let redirect = move |Host(host): Host, uri: Uri| async move {
        match make_https(&host, uri, ports.https) {
            Ok(uri) => Ok(Redirect::permanent(&uri.to_string())),
            Err(error) => {
                warn!(%error, "Failed to convert URI to HTTPS");
                Err(StatusCode::BAD_REQUEST)
            }
        }
    };

    let addr = SocketAddr::from(([0, 0, 0, 0], ports.http));
    let listener = match tokio::net::TcpListener::bind(addr).await {
        Ok(l) => l,
        Err(e) => {
            error!("Failed to bind HTTP redirect server on port {}: {}", ports.http, e);
            return;
        }
    };

    info!("HTTP redirect server listening on {}", listener.local_addr().unwrap());

    if let Err(e) = axum::serve(listener, redirect.into_make_service()).await {
        error!("HTTP redirect server error: {}", e);
    }
}

/// Health check endpoint
async fn health() -> &'static str {
    "OAuth Verification Server Running"
}

/// GET /verify/{uuid} - Show verification page
async fn verify_page(State(state): State<AppState>, Path(uuid): Path<String>) -> Html<String> {
    info!("Verification page requested for UUID: {}", uuid);

    // Check if the UUID exists in our config
    let user_exists = {
        let config = state.config_manager.read().await;
        config.find_user_by_verification_id(&uuid).is_some()
    };

    if !user_exists {
        return Html(error_page(
            "Invalid verification link. This ID was not found in our records.",
        ));
    }

    // Build redirect URI
    let redirect_uri = state.oauth.redirect_uri();

    let oauth_url = format!(
        "https://discord.com/oauth2/authorize\
        ?client_id={}\
        &redirect_uri={}\
        &response_type=code\
        &scope=identify%20guilds.join\
        &state={}",
        state.oauth.client_id,
        urlencoding::encode(&redirect_uri),
        urlencoding::encode(&uuid)
    );

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
    State(state): State<AppState>,
    Query(params): Query<CallbackParams>,
) -> Result<Html<String>, Html<String>> {
    info!("OAuth callback received for UUID: {}", params.state);

    // Exchange code for access token
    let token_response = state
        .oauth
        .http_client
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", state.oauth.client_id.as_str()),
            ("client_secret", state.oauth.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", params.code.as_str()),
            ("redirect_uri", &state.oauth.redirect_uri()),
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
        .oauth
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

    let discord_user: DiscordUser = user_response.json().await.map_err(|e| {
        error!("Failed to parse user info: {}", e);
        Html(error_page("Failed to parse user info"))
    })?;

    info!(
        "User authenticated: {} ({})",
        discord_user.username, discord_user.id
    );

    // Verify the user using our verification system
    let verification_id = &params.state;
    let discord_user_id: u64 = discord_user
        .id
        .parse()
        .map_err(|_| Html(error_page("Invalid Discord user ID")))?;
    let user_id = UserId::new(discord_user_id);

    // Look up the user in config FIRST to get season info
    let verification_result = {
        let config = state.config_manager.read().await;
        debug!("finding user...");
        match config.find_user_by_verification_id(verification_id) {
            Some((season, season_user)) => {
                // Special roles are looked up by Discord username
                info!(
                    "Looking up special roles for Discord username: '{}'",
                    discord_user.username
                );
                let special_roles = config.get_special_roles_for_user(&discord_user.username);

                // Use the season's member_role (e.g., Medlem2025E) instead of global default
                let member_role = season.member_role();
                info!(
                    "Using season member role '{}' for season '{}'",
                    member_role, season.season_id
                );

                let mut roles_to_assign = vec![member_role];
                roles_to_assign.extend(special_roles.clone());

                Some((
                    season_user.name.clone(),
                    season.season_id.clone(),
                    roles_to_assign,
                    special_roles,
                ))
            }
            None => None,
        }
    };

    let (display_name, season_id, roles_to_assign, special_roles) = match verification_result {
        Some(result) => result,
        None => {
            return Err(Html(error_page("Verification ID not found in our records")));
        }
    };

    // Check if already verified

    // Get existing user from DB
    let existing_user = state.verification_manager.get_verified_user(user_id).await;

    if let Some(ref user) = existing_user {
        // Check if verified for THIS season
        if user.verification_ids.contains_key(&season_id) {
            // User is verified for this season. Check if they are still in the guild.
            let guild_id_str = std::env::var("DISCORD_GUILD_ID").ok();
            if let Some(guild_id_str) = guild_id_str {
                if let Ok(guild_id) = guild_id_str.parse::<u64>() {
                    let guild_id = serenity::GuildId::new(guild_id);
                    let member = guild_id.member(&state.serenity_http, user_id).await;

                    if member.is_ok() {
                        // They are in the guild AND verified for this season.
                        return Ok(Html(already_verified_page(&discord_user.username)));
                    }
                    // If not in guild, we continue to re-verify (re-add roles, etc)
                    warn!(
                        "User {} is verified for season {} but not in guild. Re-verifying.",
                        user_id, season_id
                    );
                }
            }
        }
    }

    // Check if this verification ID was already used by someone else
    {
        let db = state.verification_manager.user_db().read().await;
        if let Some(existing) = db.find_by_verification_id(verification_id) {
            if existing.discord_id != discord_user.id {
                return Err(Html(error_page(
                    "This verification ID has already been used by another account.",
                )));
            }
        }
    }

    // Save/Update the verified user to database
    {
        let mut user_db = state.verification_manager.user_db().write().await;

        if let Some(existing) = user_db.find_by_discord_id(&discord_user.id) {
            let mut updated_user = existing.clone();
            updated_user.add_verification_id(&season_id, verification_id);
            // Merge special roles
            for role in &special_roles {
                if !updated_user.special_roles.contains(role) {
                    updated_user.special_roles.push(role.clone());
                }
            }
            // Update display name
            updated_user.display_name = display_name.clone();
            updated_user.update_last_seen();

            user_db.upsert_user(updated_user);
        } else {
            let tracked_user = TrackedUser::new(
                discord_user.id.clone(),
                verification_id.to_string(),
                season_id.clone(),
                display_name.clone(),
                special_roles,
            );
            user_db.upsert_user(tracked_user);
        }
    }

    // Save database to disk
    if let Err(e) = state
        .verification_manager
        .save_database("state/user_database.json")
        .await
    {
        error!("Failed to save user database: {}", e);
    }

    // Get all guilds and apply roles/nickname
    let mut guild_results = Vec::new();

    // Try to add user to guilds using OAuth access token
    let guild_id_str = std::env::var("DISCORD_GUILD_ID").ok();
    if let Some(guild_id_str) = guild_id_str {
        if let Ok(guild_id) = guild_id_str.parse::<u64>() {
            let add_result = state
                .oauth
                .http_client
                .put(&format!(
                    "https://discord.com/api/guilds/{}/members/{}",
                    guild_id, discord_user.id
                ))
                .header("Authorization", format!("Bot {}", state.oauth.bot_token))
                .json(&serde_json::json!({
                    "access_token": token.access_token
                }))
                .send()
                .await;

            match add_result {
                Ok(response) => {
                    if response.status().is_success()
                        || response.status().as_u16() == 204
                        || response.status().as_u16() == 201
                    {
                        guild_results.push(("Primary server", "Added/already member"));
                    } else {
                        let status = response.status();
                        let text = response.text().await.unwrap_or_default();
                        info!("Guild add response: {} - {}", status, text);
                        guild_results.push(("Primary server", "Already in server"));
                    }
                }
                Err(e) => {
                    error!("Failed to add to guild: {}", e);
                    guild_results.push(("Primary server", "Couldn't add"));
                }
            }

            // Now set nickname and assign roles using serenity HTTP
            let guild_id = serenity::GuildId::new(guild_id);

            // Set nickname
            if let Err(e) = guild_id
                .edit_member(
                    &state.serenity_http,
                    user_id,
                    serenity::EditMember::new().nickname(&display_name),
                )
                .await
            {
                error!(
                    "Failed to set nickname for {} in guild {}: {}. Bot requires 'Manage Nicknames' permission and must have a higher role than the target user.",
                    user_id, guild_id, e
                );
            } else {
                info!("Set nickname for {} to '{}'", user_id, display_name);
            }

            // Assign roles using sync_assignments_for_user for special roles
            info!(
                "Assigning {} roles to user {}: {:?}",
                roles_to_assign.len(),
                user_id,
                roles_to_assign
            );
            let role_manager = state.role_manager.read().await;

            // Use sync_assignments_for_user which handles checking existing roles and error messages
            let (added, failed) = role_manager
                .sync_assignments_for_user(
                    &state.serenity_http,
                    guild_id,
                    user_id,
                    &discord_user.username,
                    &roles_to_assign,
                )
                .await;

            if !added.is_empty() {
                info!("Assigned roles to {}: {:?}", user_id, added);
            }
            if !failed.is_empty() {
                warn!("Failed to assign some roles to {}: {:?}", user_id, failed);
            }
        }
    }

    info!(
        "User {} verified as '{}' via OAuth",
        discord_user.id, display_name
    );

    Ok(Html(success_page(
        &display_name,
        &discord_user.id,
        verification_id,
        &roles_to_assign,
    )))
}

fn success_page(display_name: &str, discord_id: &str, uuid: &str, roles: &[String]) -> String {
    let roles_html = roles.iter()
        .map(|r| format!("<span style=\"background: #5865F2; color: white; padding: 2px 8px; border-radius: 4px; margin: 2px; display: inline-block;\">{}</span>", r))
        .collect::<Vec<_>>()
        .join(" ");

    format!(
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
        .roles {{
            margin-top: 10px;
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
                <span class="label">Display Name:</span>
                <span class="value">{display_name}</span>
            </div>
            <div class="info-row">
                <span class="label">Discord ID:</span>
                <span class="value">{discord_id}</span>
            </div>
            <div class="info-row">
                <span class="label">Verification ID:</span>
                <span class="value" style="font-size: 11px;">{uuid}</span>
            </div>
            <div class="roles">
                <span class="label">Roles assigned:</span><br>
                {roles_html}
            </div>
        </div>

        <p style="color: #888; font-size: 14px;">You can now close this window and check Discord.</p>
    </div>
</body>
</html>"#,
        display_name = display_name,
        discord_id = discord_id,
        uuid = uuid,
        roles_html = roles_html
    )
}

fn already_verified_page(username: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Already Verified</title>
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
            color: #667eea;
        }}
        .icon {{
            font-size: 60px;
            margin-bottom: 20px;
        }}
    </style>
</head>
<body>
    <div class="container">
        <div class="icon">✓</div>
        <h1>Already Verified!</h1>
        <p>Hello <strong>{}</strong>, you are already verified.</p>
        <p style="color: #888; font-size: 14px;">You can close this window.</p>
    </div>
</body>
</html>"#,
        username
    )
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
