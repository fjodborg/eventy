//! Discord OAuth authentication for admin panel
//!
//! Admins authenticate via Discord OAuth and must have ADMINISTRATOR permission
//! in the configured guild to access the admin panel.

use axum::http::HeaderMap;
use poise::serenity_prelude::{self as serenity, GuildId, Permissions};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;
use tracing::{error, warn};

use super::oauth::OAuthState;

/// Session data for authenticated admin
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AdminSession {
    pub discord_id: String,
    pub username: String,
    pub avatar_url: Option<String>,
    pub created_at: u64,
    pub expires_at: u64,
}

impl AdminSession {
    /// Create a new session with 24-hour expiry
    pub fn new(discord_id: String, username: String, avatar_url: Option<String>) -> Self {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        Self {
            discord_id,
            username,
            avatar_url,
            created_at: now,
            expires_at: now + 86400, // 24 hours
        }
    }

    /// Check if session is expired
    pub fn is_expired(&self) -> bool {
        let now = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .unwrap_or_default()
            .as_secs();

        now >= self.expires_at
    }
}

/// Session store - maps session tokens to session data
pub struct SessionStore {
    sessions: RwLock<HashMap<String, AdminSession>>,
}

impl SessionStore {
    pub fn new() -> Self {
        Self {
            sessions: RwLock::new(HashMap::new()),
        }
    }

    /// Create a new session and return the token
    pub async fn create_session(&self, session: AdminSession) -> String {
        let token = uuid::Uuid::new_v4().to_string();
        self.sessions.write().await.insert(token.clone(), session);
        token
    }

    /// Get session by token (returns None if expired or not found)
    pub async fn get_session(&self, token: &str) -> Option<AdminSession> {
        let sessions = self.sessions.read().await;
        sessions.get(token).and_then(|s| {
            if s.is_expired() {
                None
            } else {
                Some(s.clone())
            }
        })
    }

    /// Remove a session
    pub async fn remove_session(&self, token: &str) {
        self.sessions.write().await.remove(token);
    }

    /// Clean up expired sessions
    pub async fn cleanup_expired(&self) {
        let mut sessions = self.sessions.write().await;
        sessions.retain(|_, s| !s.is_expired());
    }
}

pub type SharedSessionStore = Arc<SessionStore>;

pub fn create_session_store() -> SharedSessionStore {
    Arc::new(SessionStore::new())
}

/// OAuth callback parameters for admin login
#[derive(Deserialize)]
pub struct AdminCallbackParams {
    pub code: String,
    pub state: String, // Contains "admin_login"
}

/// Check if a user has admin permissions in the configured guild
pub async fn check_admin_permissions(
    http: &serenity::Http,
    guild_id: GuildId,
    user_id: serenity::UserId,
) -> bool {
    // Try to get the member
    match http.get_member(guild_id, user_id).await {
        Ok(member) => {
            // Get the guild to check permissions
            match http.get_guild(guild_id).await {
                Ok(guild) => {
                    // Get user's roles and check for ADMINISTRATOR
                    for role_id in &member.roles {
                        if let Some(role) = guild.roles.get(role_id) {
                            if role.permissions.contains(Permissions::ADMINISTRATOR) {
                                return true;
                            }
                        }
                    }

                    // Check if user is the guild owner
                    if guild.owner_id == user_id {
                        return true;
                    }

                    false
                }
                Err(e) => {
                    error!("Failed to get guild {}: {}", guild_id, e);
                    false
                }
            }
        }
        Err(e) => {
            warn!(
                "User {} is not a member of guild {}: {}",
                user_id, guild_id, e
            );
            false
        }
    }
}

/// Extract session token from cookies
pub fn get_session_token(headers: &HeaderMap) -> Option<String> {
    headers
        .get(axum::http::header::COOKIE)?
        .to_str()
        .ok()?
        .split(';')
        .find_map(|cookie| {
            let cookie = cookie.trim();
            if cookie.starts_with("admin_session=") {
                Some(cookie.trim_start_matches("admin_session=").to_string())
            } else {
                None
            }
        })
}

/// Create a session cookie
pub fn create_session_cookie(token: &str) -> String {
    format!(
        "admin_session={}; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=86400",
        token
    )
}

/// Create a logout cookie (clears the session)
pub fn create_logout_cookie() -> String {
    "admin_session=; Path=/admin; HttpOnly; SameSite=Lax; Max-Age=0".to_string()
}

/// Generate the OAuth URL for admin login
pub fn admin_oauth_url(oauth: &OAuthState) -> String {
    let redirect_uri = format!("{}/admin/callback", oauth.base_url);
    let redirect_uri_encoded = urlencoding::encode(&redirect_uri);

    format!(
        "https://discord.com/api/oauth2/authorize?client_id={}&redirect_uri={}&response_type=code&scope=identify&state=admin_login",
        oauth.client_id,
        redirect_uri_encoded
    )
}

/// Admin login page HTML
pub fn login_page(oauth_url: &str) -> String {
    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Login - Eventy</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #fff;
        }}
        .container {{
            background: rgba(255,255,255,0.05);
            border-radius: 16px;
            padding: 3rem;
            text-align: center;
            backdrop-filter: blur(10px);
            border: 1px solid rgba(255,255,255,0.1);
            max-width: 400px;
            width: 90%;
        }}
        h1 {{
            font-size: 1.8rem;
            margin-bottom: 0.5rem;
        }}
        p {{
            color: #a0a0a0;
            margin-bottom: 2rem;
        }}
        .discord-btn {{
            display: inline-flex;
            align-items: center;
            gap: 0.75rem;
            background: #5865F2;
            color: white;
            text-decoration: none;
            padding: 1rem 2rem;
            border-radius: 8px;
            font-weight: 600;
            font-size: 1rem;
            transition: background 0.2s;
        }}
        .discord-btn:hover {{
            background: #4752c4;
        }}
        .discord-btn svg {{
            width: 24px;
            height: 24px;
        }}
        .note {{
            margin-top: 2rem;
            font-size: 0.85rem;
            color: #808080;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>Admin Panel</h1>
        <p>Sign in with Discord to access the admin panel</p>
        <a href="{}" class="discord-btn">
            <svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 127.14 96.36" fill="currentColor">
                <path d="M107.7,8.07A105.15,105.15,0,0,0,81.47,0a72.06,72.06,0,0,0-3.36,6.83A97.68,97.68,0,0,0,49,6.83,72.37,72.37,0,0,0,45.64,0,105.89,105.89,0,0,0,19.39,8.09C2.79,32.65-1.71,56.6.54,80.21h0A105.73,105.73,0,0,0,32.71,96.36,77.7,77.7,0,0,0,39.6,85.25a68.42,68.42,0,0,1-10.85-5.18c.91-.66,1.8-1.34,2.66-2a75.57,75.57,0,0,0,64.32,0c.87.71,1.76,1.39,2.66,2a68.68,68.68,0,0,1-10.87,5.19,77,77,0,0,0,6.89,11.1A105.25,105.25,0,0,0,126.6,80.22h0C129.24,52.84,122.09,29.11,107.7,8.07ZM42.45,65.69C36.18,65.69,31,60,31,53s5-12.74,11.43-12.74S54,46,53.89,53,48.84,65.69,42.45,65.69Zm42.24,0C78.41,65.69,73.25,60,73.25,53s5-12.74,11.44-12.74S96.23,46,96.12,53,91.08,65.69,84.69,65.69Z"/>
            </svg>
            Login with Discord
        </a>
        <p class="note">You must have Administrator permissions in the server to access this panel.</p>
    </div>
</body>
</html>"#,
        oauth_url
    )
}

/// Access denied page HTML
pub fn access_denied_page() -> String {
    r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Access Denied - Eventy</title>
    <style>
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: linear-gradient(135deg, #1a1a2e 0%, #16213e 100%);
            min-height: 100vh;
            display: flex;
            align-items: center;
            justify-content: center;
            color: #fff;
        }
        .container {
            background: rgba(255,255,255,0.05);
            border-radius: 16px;
            padding: 3rem;
            text-align: center;
            backdrop-filter: blur(10px);
            border: 1px solid rgba(255,255,255,0.1);
            max-width: 500px;
            width: 90%;
        }
        h1 { color: #e74c3c; margin-bottom: 1rem; }
        p { color: #a0a0a0; margin-bottom: 1.5rem; }
        a {
            color: #5865F2;
            text-decoration: none;
        }
        a:hover { text-decoration: underline; }
    </style>
</head>
<body>
    <div class="container">
        <h1>Access Denied</h1>
        <p>You do not have Administrator permissions in this server.</p>
        <p>Please contact a server administrator if you believe this is an error.</p>
        <p><a href="/admin/login">Try logging in again</a></p>
    </div>
</body>
</html>"#
        .to_string()
}
