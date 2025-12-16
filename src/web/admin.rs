//! Admin panel routes and handlers
//!
//! Provides a web-based admin interface for:
//! - Viewing and editing season configurations
//! - Managing global settings
//! - Viewing live logs

use axum::{
    extract::{Path, Query, State},
    http::{header::SET_COOKIE, HeaderMap, StatusCode},
    response::{
        sse::{Event, Sse},
        Html, IntoResponse, Redirect, Response,
    },
    routing::get,
    Form, Router,
};
use serde::Deserialize;
use poise::serenity_prelude::{self as serenity, GuildId, UserId};
use std::convert::Infallible;
use std::sync::Arc;
use std::time::Duration;
use tokio_stream::wrappers::BroadcastStream;
use tokio_stream::StreamExt;
use tracing::{error, info, warn};

use super::auth::{
    access_denied_page, admin_oauth_url, check_admin_permissions_with_config, create_logout_cookie,
    create_session_cookie, get_session_token, login_page, AdminCallbackParams, AdminSession,
    SharedSessionStore,
};
use super::oauth::OAuthState;
use crate::logging::SharedLogBuffer;
use crate::managers::{SharedConfigManager, SharedChannelManager, SharedRoleManager};

/// Extended app state for admin panel
#[derive(Clone)]
pub struct AdminState {
    pub oauth: OAuthState,
    pub config_manager: SharedConfigManager,
    pub channel_manager: SharedChannelManager,
    pub role_manager: SharedRoleManager,
    pub session_store: SharedSessionStore,
    pub log_buffer: SharedLogBuffer,
    pub serenity_http: Arc<serenity::Http>,
    pub guild_id: GuildId,
}

/// Create admin router
pub fn admin_router(state: AdminState) -> Router {
    Router::new()
        .route("/", get(dashboard))
        .route("/login", get(login))
        .route("/logout", get(logout))
        .route("/callback", get(oauth_callback))
        .route("/seasons", get(seasons_list))
        .route("/season/:id", get(season_detail))
        .route("/edit/global", get(edit_global).post(save_global))
        .route("/edit/season/:id/:file", get(edit_season_file).post(save_season_file))
        .route("/new-season", get(new_season_form).post(create_season))
        .route("/logs", get(logs_page))
        .route("/logs/stream", get(logs_stream))
        .route("/restart", axum::routing::post(restart_bot))
        .route("/sync/roles", axum::routing::post(sync_roles))
        .route("/sync/assignments", axum::routing::post(sync_assignments))
        .route("/sync/season/:id", axum::routing::post(sync_season))
        .with_state(state)
}

/// Check authentication and return session or redirect
async fn require_auth(
    headers: &HeaderMap,
    state: &AdminState,
) -> Result<AdminSession, Response> {
    let token = get_session_token(headers).ok_or_else(|| {
        Redirect::to("/admin/login").into_response()
    })?;

    state
        .session_store
        .get_session(&token)
        .await
        .ok_or_else(|| Redirect::to("/admin/login").into_response())
}

/// GET /admin/login - Show login page
async fn login(State(state): State<AdminState>) -> Html<String> {
    let oauth_url = admin_oauth_url(&state.oauth);
    Html(login_page(&oauth_url))
}

/// GET /admin/logout - Clear session and redirect to login
async fn logout(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    if let Some(token) = get_session_token(&headers) {
        state.session_store.remove_session(&token).await;
    }

    (
        [(SET_COOKIE, create_logout_cookie())],
        Redirect::to("/admin/login"),
    )
}

/// GET /admin/callback - Handle OAuth callback
async fn oauth_callback(
    Query(params): Query<AdminCallbackParams>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    // Verify state parameter
    if params.state != "admin_login" {
        return (
            StatusCode::BAD_REQUEST,
            Html("Invalid OAuth state".to_string()),
        )
            .into_response();
    }

    // Exchange code for token
    let redirect_uri = format!("{}/admin/callback", state.oauth.base_url);

    let token_response = match state
        .oauth
        .http_client
        .post("https://discord.com/api/oauth2/token")
        .form(&[
            ("client_id", state.oauth.client_id.as_str()),
            ("client_secret", state.oauth.client_secret.as_str()),
            ("grant_type", "authorization_code"),
            ("code", &params.code),
            ("redirect_uri", &redirect_uri),
        ])
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to parse token response: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html("Authentication failed".to_string()),
                )
                    .into_response();
            }
        },
        Err(e) => {
            error!("Failed to exchange OAuth code: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Authentication failed".to_string()),
            )
                .into_response();
        }
    };

    let access_token = match token_response.get("access_token").and_then(|v| v.as_str()) {
        Some(t) => t,
        None => {
            error!("No access token in response: {:?}", token_response);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Authentication failed".to_string()),
            )
                .into_response();
        }
    };

    // Get user info
    let user_response = match state
        .oauth
        .http_client
        .get("https://discord.com/api/users/@me")
        .header("Authorization", format!("Bearer {}", access_token))
        .send()
        .await
    {
        Ok(resp) => match resp.json::<serde_json::Value>().await {
            Ok(json) => json,
            Err(e) => {
                error!("Failed to parse user response: {}", e);
                return (
                    StatusCode::INTERNAL_SERVER_ERROR,
                    Html("Authentication failed".to_string()),
                )
                    .into_response();
            }
        },
        Err(e) => {
            error!("Failed to get user info: {}", e);
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Authentication failed".to_string()),
            )
                .into_response();
        }
    };

    let discord_id = match user_response.get("id").and_then(|v| v.as_str()) {
        Some(id) => id.to_string(),
        None => {
            return (
                StatusCode::INTERNAL_SERVER_ERROR,
                Html("Failed to get user ID".to_string()),
            )
                .into_response();
        }
    };

    let username = user_response
        .get("username")
        .and_then(|v| v.as_str())
        .unwrap_or("Unknown")
        .to_string();

    let avatar_url = user_response.get("avatar").and_then(|v| v.as_str()).map(|hash| {
        format!(
            "https://cdn.discordapp.com/avatars/{}/{}.png",
            discord_id, hash
        )
    });

    // Check if user has admin permissions (including special roles from assignments.json)
    let user_id = UserId::new(discord_id.parse().unwrap_or(0));
    let is_admin = check_admin_permissions_with_config(
        &state.serenity_http,
        state.guild_id,
        user_id,
        Some(&state.config_manager),
    ).await;

    if !is_admin {
        warn!(
            "User {} ({}) denied admin access - no admin permissions",
            username, discord_id
        );
        return Html(access_denied_page()).into_response();
    }

    // Create session
    let session = AdminSession::new(discord_id.clone(), username.clone(), avatar_url);
    let session_token = state.session_store.create_session(session).await;

    info!("Admin {} ({}) logged in successfully", username, discord_id);

    (
        [(SET_COOKIE, create_session_cookie(&session_token))],
        Redirect::to("/admin"),
    )
        .into_response()
}

/// GET /admin - Dashboard
async fn dashboard(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    // Get season and global config info
    let config = state.config_manager.read().await;
    let seasons: Vec<_> = config
        .get_seasons()
        .iter()
        .map(|(id, season)| {
            format!(
                "<tr><td><a href=\"/admin/season/{}\">{}</a></td><td>{}</td><td>{}</td></tr>",
                id,
                id,
                if season.name().is_empty() { id } else { season.name() },
                season.user_count()
            )
        })
        .collect();

    // Get global config status
    let roles_count = config.get_global_roles().map(|r| r.roles.len()).unwrap_or(0);
    let has_permissions = config.get_global_permissions().is_some();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Admin Dashboard - Eventy</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
        }}
        .navbar {{
            background: rgba(255,255,255,0.05);
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }}
        .navbar h1 {{ font-size: 1.25rem; }}
        .navbar .user {{
            display: flex;
            align-items: center;
            gap: 1rem;
        }}
        .navbar .user img {{
            width: 32px;
            height: 32px;
            border-radius: 50%;
        }}
        .navbar a {{ color: #5865F2; text-decoration: none; }}
        .navbar a:hover {{ text-decoration: underline; }}
        .container {{ max-width: 1200px; margin: 0 auto; padding: 2rem; }}
        .cards {{
            display: grid;
            grid-template-columns: repeat(auto-fit, minmax(300px, 1fr));
            gap: 1.5rem;
            margin-bottom: 2rem;
        }}
        .card {{
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            padding: 1.5rem;
            border: 1px solid rgba(255,255,255,0.1);
        }}
        .card h2 {{
            font-size: 1rem;
            color: #a0a0a0;
            margin-bottom: 0.5rem;
        }}
        .card .value {{
            font-size: 2rem;
            font-weight: bold;
        }}
        table {{
            width: 100%;
            border-collapse: collapse;
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            overflow: hidden;
        }}
        th, td {{
            padding: 1rem;
            text-align: left;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }}
        th {{ background: rgba(255,255,255,0.05); color: #a0a0a0; font-weight: 500; }}
        tr:last-child td {{ border-bottom: none; }}
        a {{ color: #5865F2; text-decoration: none; }}
        a:hover {{ text-decoration: underline; }}
        .nav-links {{
            display: flex;
            gap: 2rem;
            margin-bottom: 2rem;
        }}
        .nav-links a {{
            padding: 0.5rem 1rem;
            background: rgba(255,255,255,0.05);
            border-radius: 8px;
        }}
        .nav-links a:hover {{
            background: rgba(255,255,255,0.1);
            text-decoration: none;
        }}
    </style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <div class="user">
            <span>Welcome, {}</span>
            <a href="/admin/logout">Logout</a>
        </div>
    </nav>
    <div class="container">
        <div class="nav-links">
            <a href="/admin">Dashboard</a>
            <a href="/admin/new-season">New Season</a>
            <a href="/admin/logs">Logs</a>
            <form method="POST" action="/admin/restart" style="display:inline;" onsubmit="return confirm('Are you sure you want to restart the bot?');">
                <button type="submit" style="background:#e74c3c;color:#fff;padding:0.5rem 1rem;border-radius:8px;border:none;cursor:pointer;">Restart Bot</button>
            </form>
        </div>
        <div class="cards">
            <div class="card">
                <h2>Seasons</h2>
                <div class="value">{}</div>
            </div>
            <div class="card">
                <h2>Global Roles</h2>
                <div class="value">{} defined</div>
                <div style="margin-top:1rem;display:flex;gap:0.5rem;flex-wrap:wrap;">
                    <a href="/admin/edit/global?tab=roles" style="background:rgba(255,255,255,0.1);color:#fff;padding:0.4rem 0.8rem;border-radius:6px;text-decoration:none;font-size:0.85rem;">Edit Roles</a>
                </div>
            </div>
            <div class="card">
                <h2>Permissions</h2>
                <div class="value">{}</div>
                <div style="margin-top:1rem;">
                    <a href="/admin/edit/global?tab=permissions" style="background:rgba(255,255,255,0.1);color:#fff;padding:0.4rem 0.8rem;border-radius:6px;text-decoration:none;font-size:0.85rem;">Edit</a>
                </div>
            </div>
        </div>
        <h2 style="margin-bottom: 1rem;">Seasons</h2>
        <table>
            <thead>
                <tr>
                    <th>ID</th>
                    <th>Name</th>
                    <th>Users</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
</body>
</html>"#,
        session.username,
        config.get_seasons().len(),
        roles_count,
        if has_permissions { "Loaded" } else { "Not loaded" },
        seasons.join("\n")
    );

    Html(html).into_response()
}

/// GET /admin/seasons - List all seasons
async fn seasons_list(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    // Redirect to dashboard for now (seasons are shown there)
    Redirect::to("/admin").into_response()
}

/// GET /admin/season/:id - Season detail view
async fn season_detail(
    headers: HeaderMap,
    Path(season_id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let config = state.config_manager.read().await;

    let season = match config.get_seasons().get(&season_id) {
        Some(s) => s,
        None => {
            return Html(format!(
                r#"<!DOCTYPE html><html><head><title>Not Found</title></head>
                <body style="background:#1a1a2e;color:#fff;font-family:sans-serif;padding:2rem;">
                <h1>Season not found: {}</h1>
                <a href="/admin" style="color:#5865F2;">Back to dashboard</a>
                </body></html>"#,
                season_id
            ))
            .into_response();
        }
    };

    let users_html: Vec<_> = season
        .users
        .iter()
        .enumerate()
        .map(|(i, user)| {
            format!(
                "<tr><td>{}</td><td>{}</td><td>{}</td></tr>",
                i + 1,
                user.name,
                &user.id[..8.min(user.id.len())]
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Season {} - Eventy Admin</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
        }}
        .navbar {{
            background: rgba(255,255,255,0.05);
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }}
        .navbar h1 {{ font-size: 1.25rem; }}
        .navbar a {{ color: #5865F2; text-decoration: none; }}
        .container {{ max-width: 1200px; margin: 0 auto; padding: 2rem; }}
        .back {{ margin-bottom: 1rem; }}
        .back a {{ color: #a0a0a0; text-decoration: none; }}
        .back a:hover {{ color: #fff; }}
        h2 {{ margin-bottom: 1rem; }}
        .info {{
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            padding: 1.5rem;
            margin-bottom: 2rem;
        }}
        .info p {{ margin-bottom: 0.5rem; color: #a0a0a0; }}
        .info strong {{ color: #fff; }}
        table {{
            width: 100%;
            border-collapse: collapse;
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            overflow: hidden;
        }}
        th, td {{
            padding: 0.75rem 1rem;
            text-align: left;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }}
        th {{ background: rgba(255,255,255,0.05); color: #a0a0a0; font-weight: 500; }}
        tr:last-child td {{ border-bottom: none; }}
        .search {{
            margin-bottom: 1rem;
        }}
        .search input {{
            padding: 0.75rem 1rem;
            border-radius: 8px;
            border: 1px solid rgba(255,255,255,0.2);
            background: rgba(255,255,255,0.05);
            color: #fff;
            width: 100%;
            max-width: 300px;
        }}
    </style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin">← Back to Dashboard</a></div>
        <h2>Season: {}</h2>
        <div class="info">
            <p>Name: <strong>{}</strong></p>
            <p>Users: <strong>{}</strong></p>
            <p>Active: <strong>{}</strong></p>
        </div>
        <div style="margin-bottom: 1.5rem;">
            <a href="/admin/edit/season/{}/users" class="btn btn-primary" style="background:#5865F2;color:#fff;padding:0.5rem 1rem;border-radius:6px;text-decoration:none;margin-right:0.5rem;">Edit users.json</a>
            <a href="/admin/edit/season/{}/season" class="btn btn-secondary" style="background:rgba(255,255,255,0.1);color:#fff;padding:0.5rem 1rem;border-radius:6px;text-decoration:none;margin-right:0.5rem;">Edit season.json</a>
            <form method="POST" action="/admin/sync/season/{}" style="display:inline;">
                <button type="submit" style="background:#2ecc71;color:#fff;padding:0.5rem 1rem;border-radius:6px;border:none;cursor:pointer;">Sync Category to Discord</button>
            </form>
        </div>
        <h3 style="margin-bottom: 1rem;">Users</h3>
        <div class="search">
            <input type="text" id="search" placeholder="Search users..." onkeyup="filterTable()">
        </div>
        <table id="users-table">
            <thead>
                <tr>
                    <th>#</th>
                    <th>Name</th>
                    <th>ID (partial)</th>
                </tr>
            </thead>
            <tbody>
                {}
            </tbody>
        </table>
    </div>
    <script>
        function filterTable() {{
            const search = document.getElementById('search').value.toLowerCase();
            const rows = document.querySelectorAll('#users-table tbody tr');
            rows.forEach(row => {{
                const text = row.textContent.toLowerCase();
                row.style.display = text.includes(search) ? '' : 'none';
            }});
        }}
    </script>
</body>
</html>"#,
        season_id,
        season_id,
        if season.name().is_empty() { &season_id } else { season.name() },
        season.user_count(),
        season.is_active(),
        season_id, // edit users link
        season_id, // edit season link
        season_id, // sync category form
        users_html.join("\n")
    );

    Html(html).into_response()
}

/// GET /admin/logs - Log viewer page
async fn logs_page(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    // Get recent logs
    let recent_logs = state.log_buffer.get_recent(100);
    let logs_html: Vec<_> = recent_logs
        .iter()
        .map(|entry| {
            let level_class = match entry.level.as_str() {
                "ERROR" => "error",
                "WARN" => "warn",
                "INFO" => "info",
                "DEBUG" => "debug",
                _ => "trace",
            };
            format!(
                r#"<div class="log-entry {}"><span class="time">{}</span> <span class="level">{}</span> <span class="target">[{}]</span> {}</div>"#,
                level_class,
                entry.timestamp.format("%H:%M:%S%.3f"),
                entry.level,
                entry.target,
                html_escape(&entry.message)
            )
        })
        .collect();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Logs - Eventy Admin</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
        }}
        .navbar {{
            background: rgba(255,255,255,0.05);
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }}
        .navbar h1 {{ font-size: 1.25rem; }}
        .navbar a {{ color: #5865F2; text-decoration: none; }}
        .container {{ max-width: 1400px; margin: 0 auto; padding: 2rem; }}
        .back {{ margin-bottom: 1rem; }}
        .back a {{ color: #a0a0a0; text-decoration: none; }}
        .back a:hover {{ color: #fff; }}
        .controls {{
            display: flex;
            gap: 1rem;
            margin-bottom: 1rem;
            align-items: center;
        }}
        .controls label {{
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        .log-container {{
            background: #0d0d1a;
            border-radius: 8px;
            padding: 1rem;
            font-family: 'SF Mono', 'Fira Code', monospace;
            font-size: 0.85rem;
            height: calc(100vh - 250px);
            overflow-y: auto;
            border: 1px solid rgba(255,255,255,0.1);
        }}
        .log-entry {{
            padding: 0.25rem 0;
            border-bottom: 1px solid rgba(255,255,255,0.05);
            white-space: pre-wrap;
            word-break: break-all;
        }}
        .log-entry .time {{ color: #666; }}
        .log-entry .level {{ font-weight: bold; }}
        .log-entry .target {{ color: #888; }}
        .log-entry.error {{ color: #e74c3c; }}
        .log-entry.error .level {{ color: #e74c3c; }}
        .log-entry.warn {{ color: #f39c12; }}
        .log-entry.warn .level {{ color: #f39c12; }}
        .log-entry.info .level {{ color: #3498db; }}
        .log-entry.debug {{ color: #888; }}
        .log-entry.debug .level {{ color: #888; }}
        #live-indicator {{
            display: inline-block;
            width: 8px;
            height: 8px;
            background: #2ecc71;
            border-radius: 50%;
            margin-right: 0.5rem;
            animation: pulse 2s infinite;
        }}
        #live-indicator.disconnected {{
            background: #e74c3c;
            animation: none;
        }}
        @keyframes pulse {{
            0%, 100% {{ opacity: 1; }}
            50% {{ opacity: 0.5; }}
        }}
    </style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin">← Back to Dashboard</a></div>
        <h2 style="margin-bottom: 1rem;">Live Logs</h2>
        <div class="controls">
            <span id="live-indicator"></span>
            <span id="status">Connecting...</span>
            <label>
                <input type="checkbox" id="autoscroll" checked>
                Auto-scroll
            </label>
        </div>
        <div class="log-container" id="logs">
            {}
        </div>
    </div>
    <script>
        const logsContainer = document.getElementById('logs');
        const autoscrollCheckbox = document.getElementById('autoscroll');
        const indicator = document.getElementById('live-indicator');
        const status = document.getElementById('status');

        function scrollToBottom() {{
            if (autoscrollCheckbox.checked) {{
                logsContainer.scrollTop = logsContainer.scrollHeight;
            }}
        }}

        function addLogEntry(entry) {{
            const levelClass = entry.level.toLowerCase();
            const time = new Date(entry.timestamp).toLocaleTimeString('en-US', {{
                hour12: false,
                hour: '2-digit',
                minute: '2-digit',
                second: '2-digit',
                fractionalSecondDigits: 3
            }});

            const div = document.createElement('div');
            div.className = 'log-entry ' + levelClass;
            div.innerHTML = '<span class="time">' + time + '</span> ' +
                '<span class="level">' + entry.level + '</span> ' +
                '<span class="target">[' + entry.target + ']</span> ' +
                escapeHtml(entry.message);

            logsContainer.appendChild(div);

            // Keep only last 500 entries in DOM
            while (logsContainer.children.length > 500) {{
                logsContainer.removeChild(logsContainer.firstChild);
            }}

            scrollToBottom();
        }}

        function escapeHtml(text) {{
            const div = document.createElement('div');
            div.textContent = text;
            return div.innerHTML;
        }}

        // Connect to SSE stream
        const eventSource = new EventSource('/admin/logs/stream');

        eventSource.onopen = function() {{
            indicator.classList.remove('disconnected');
            status.textContent = 'Connected';
        }};

        eventSource.onmessage = function(event) {{
            try {{
                const entry = JSON.parse(event.data);
                addLogEntry(entry);
            }} catch (e) {{
                console.error('Failed to parse log entry:', e);
            }}
        }};

        eventSource.onerror = function() {{
            indicator.classList.add('disconnected');
            status.textContent = 'Disconnected - Reconnecting...';
        }};

        // Initial scroll to bottom
        scrollToBottom();
    </script>
</body>
</html>"#,
        logs_html.join("\n")
    );

    Html(html).into_response()
}

/// GET /admin/logs/stream - SSE endpoint for live logs
async fn logs_stream(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    // Check auth via query param or cookie
    let token = get_session_token(&headers);

    if token.is_none() {
        return (StatusCode::UNAUTHORIZED, "Unauthorized").into_response();
    }

    let session = state.session_store.get_session(token.as_ref().unwrap()).await;
    if session.is_none() {
        return (StatusCode::UNAUTHORIZED, "Session expired").into_response();
    }

    // Create SSE stream
    let rx = state.log_buffer.subscribe();
    let stream = BroadcastStream::new(rx).filter_map(|result| {
        match result {
            Ok(entry) => Some(Ok::<_, Infallible>(Event::default().data(entry.to_json()))),
            Err(_) => None, // Skip lagged messages
        }
    });

    Sse::new(stream)
        .keep_alive(
            axum::response::sse::KeepAlive::new()
                .interval(Duration::from_secs(15))
                .text("ping"),
        )
        .into_response()
}

/// Escape HTML special characters
fn html_escape(s: &str) -> String {
    s.replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&#39;")
}

/// Form data for JSON editor
#[derive(Deserialize)]
struct JsonEditorForm {
    content: String,
}

/// Form data for creating a new season
#[derive(Deserialize)]
struct NewSeasonForm {
    season_id: String,
    name: String,
}

/// Common CSS for editor pages
fn editor_css() -> &'static str {
    r#"
        * { box-sizing: border-box; margin: 0; padding: 0; }
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
        }
        .navbar {
            background: rgba(255,255,255,0.05);
            padding: 1rem 2rem;
            display: flex;
            justify-content: space-between;
            align-items: center;
            border-bottom: 1px solid rgba(255,255,255,0.1);
        }
        .navbar h1 { font-size: 1.25rem; }
        .navbar a { color: #5865F2; text-decoration: none; }
        .container { max-width: 1200px; margin: 0 auto; padding: 2rem; }
        .back { margin-bottom: 1rem; }
        .back a { color: #a0a0a0; text-decoration: none; }
        .back a:hover { color: #fff; }
        h2 { margin-bottom: 1rem; }
        .editor-container {
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            padding: 1.5rem;
            margin-bottom: 1rem;
        }
        textarea {
            width: 100%;
            min-height: 500px;
            background: #0d0d1a;
            border: 1px solid rgba(255,255,255,0.2);
            border-radius: 8px;
            color: #fff;
            font-family: 'SF Mono', 'Fira Code', 'Consolas', monospace;
            font-size: 14px;
            padding: 1rem;
            resize: vertical;
            line-height: 1.5;
        }
        textarea:focus {
            outline: none;
            border-color: #5865F2;
        }
        .btn {
            display: inline-block;
            padding: 0.75rem 1.5rem;
            border-radius: 8px;
            border: none;
            cursor: pointer;
            font-size: 1rem;
            text-decoration: none;
            margin-right: 0.5rem;
        }
        .btn-primary {
            background: #5865F2;
            color: white;
        }
        .btn-primary:hover {
            background: #4752c4;
        }
        .btn-secondary {
            background: rgba(255,255,255,0.1);
            color: white;
        }
        .btn-secondary:hover {
            background: rgba(255,255,255,0.2);
        }
        .message {
            padding: 1rem;
            border-radius: 8px;
            margin-bottom: 1rem;
        }
        .message.success {
            background: rgba(46, 204, 113, 0.2);
            border: 1px solid #2ecc71;
        }
        .message.error {
            background: rgba(231, 76, 60, 0.2);
            border: 1px solid #e74c3c;
        }
        .form-group {
            margin-bottom: 1rem;
        }
        .form-group label {
            display: block;
            margin-bottom: 0.5rem;
            color: #a0a0a0;
        }
        .form-group input {
            width: 100%;
            max-width: 400px;
            padding: 0.75rem 1rem;
            border-radius: 8px;
            border: 1px solid rgba(255,255,255,0.2);
            background: rgba(255,255,255,0.05);
            color: #fff;
            font-size: 1rem;
        }
        .form-group input:focus {
            outline: none;
            border-color: #5865F2;
        }
        .hint {
            font-size: 0.85rem;
            color: #888;
            margin-top: 0.5rem;
        }
    "#
}

/// GET /admin/edit/global - Edit global configs
async fn edit_global(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();
    drop(config);

    // Load all three global config files
    let roles_content = tokio::fs::read_to_string(format!("{}/global/roles.json", data_path))
        .await
        .unwrap_or_else(|_| r#"{"roles": []}"#.to_string());
    let assignments_content = tokio::fs::read_to_string(format!("{}/global/assignments.json", data_path))
        .await
        .unwrap_or_else(|_| r#"{"discord_usernames_by_role": {}}"#.to_string());
    let permissions_content = tokio::fs::read_to_string(format!("{}/global/permissions.json", data_path))
        .await
        .unwrap_or_else(|_| r#"{"definitions": {}}"#.to_string());

    // Get which tab to show
    let active_tab = params.get("tab").map(|s| s.as_str()).unwrap_or("roles");

    let message = params.get("msg").map(|m| {
        let (class, text) = if m == "saved" {
            ("success", "Configuration saved successfully!")
        } else if m.starts_with("error:") {
            ("error", m.strip_prefix("error:").unwrap_or("Unknown error"))
        } else {
            ("", m.as_str())
        };
        format!(r#"<div class="message {}">{}</div>"#, class, html_escape(text))
    }).unwrap_or_default();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Edit Global Config - Eventy Admin</title>
    <style>
        {editor_css}
        .tabs {{
            display: flex;
            gap: 0;
            margin-bottom: 1rem;
            border-bottom: 1px solid rgba(255,255,255,0.2);
        }}
        .tab {{
            padding: 0.75rem 1.5rem;
            background: transparent;
            border: none;
            color: #a0a0a0;
            cursor: pointer;
            font-size: 0.9rem;
            border-bottom: 2px solid transparent;
            margin-bottom: -1px;
        }}
        .tab:hover {{ color: #fff; }}
        .tab.active {{
            color: #5865F2;
            border-bottom-color: #5865F2;
        }}
        .tab-content {{ display: none; }}
        .tab-content.active {{ display: block; }}
        .file-hint {{
            color: #a0a0a0;
            font-size: 0.85rem;
            margin-bottom: 1rem;
        }}
    </style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin">← Back to Dashboard</a></div>
        <h2>Edit Global Configuration</h2>
        {message}

        <div class="tabs">
            <button class="tab {roles_active}" onclick="showTab('roles')">Roles</button>
            <button class="tab {assignments_active}" onclick="showTab('assignments')">Assignments</button>
            <button class="tab {permissions_active}" onclick="showTab('permissions')">Permissions</button>
        </div>

        <div id="roles-tab" class="tab-content {roles_active}">
            <p class="file-hint">File: global/roles.json - Defines Discord roles to create</p>
            <form method="POST" action="/admin/edit/global?file=roles">
                <div class="editor-container">
                    <textarea name="content" class="editor">{roles_content}</textarea>
                </div>
                <button type="submit" class="btn btn-primary">Save Roles</button>
            </form>
            <div style="margin-top:1.5rem;padding-top:1.5rem;border-top:1px solid rgba(255,255,255,0.1);">
                <h3 style="margin-bottom:0.75rem;font-size:1rem;color:#a0a0a0;">Sync to Discord</h3>
                <p style="margin-bottom:1rem;font-size:0.9rem;color:#808080;">Create or update roles in Discord to match this configuration.</p>
                <form method="POST" action="/admin/sync/roles" style="display:inline;">
                    <button type="submit" class="btn" style="background:#2ecc71;color:#fff;">Sync Roles to Discord</button>
                </form>
            </div>
        </div>

        <div id="assignments-tab" class="tab-content {assignments_active}">
            <p class="file-hint">File: global/assignments.json - Assigns special roles to Discord users</p>
            <form method="POST" action="/admin/edit/global?file=assignments">
                <div class="editor-container">
                    <textarea name="content" class="editor">{assignments_content}</textarea>
                </div>
                <button type="submit" class="btn btn-primary">Save Assignments</button>
            </form>
            <div style="margin-top:1.5rem;padding-top:1.5rem;border-top:1px solid rgba(255,255,255,0.1);">
                <h3 style="margin-bottom:0.75rem;font-size:1rem;color:#a0a0a0;">Sync to Discord</h3>
                <p style="margin-bottom:1rem;font-size:0.9rem;color:#808080;">Assign special roles to all verified users in the server based on this configuration.</p>
                <form method="POST" action="/admin/sync/assignments" style="display:inline;">
                    <button type="submit" class="btn" style="background:#2ecc71;color:#fff;">Sync Assignments to Discord</button>
                </form>
            </div>
        </div>

        <div id="permissions-tab" class="tab-content {permissions_active}">
            <p class="file-hint">File: global/permissions.json - Defines permission levels (read, readwrite, admin)</p>
            <form method="POST" action="/admin/edit/global?file=permissions">
                <div class="editor-container">
                    <textarea name="content" class="editor">{permissions_content}</textarea>
                </div>
                <button type="submit" class="btn btn-primary">Save Permissions</button>
            </form>
        </div>

        <a href="/admin" class="btn btn-secondary" style="margin-top: 1rem;">Back to Dashboard</a>
    </div>
    <script>
        function showTab(name) {{
            document.querySelectorAll('.tab').forEach(t => t.classList.remove('active'));
            document.querySelectorAll('.tab-content').forEach(t => t.classList.remove('active'));
            document.querySelector(`[onclick="showTab('${{name}}')"]`).classList.add('active');
            document.getElementById(name + '-tab').classList.add('active');
        }}

        // JSON validation
        document.querySelectorAll('.editor').forEach(editor => {{
            editor.addEventListener('input', function() {{
                try {{
                    JSON.parse(this.value);
                    this.style.borderColor = 'rgba(255,255,255,0.2)';
                }} catch (e) {{
                    this.style.borderColor = '#e74c3c';
                }}
            }});
        }});
    </script>
</body>
</html>"#,
        editor_css = editor_css(),
        message = message,
        roles_active = if active_tab == "roles" { "active" } else { "" },
        assignments_active = if active_tab == "assignments" { "active" } else { "" },
        permissions_active = if active_tab == "permissions" { "active" } else { "" },
        roles_content = html_escape(&roles_content),
        assignments_content = html_escape(&assignments_content),
        permissions_content = html_escape(&permissions_content),
    );

    Html(html).into_response()
}

/// Query params for save_global
#[derive(Deserialize)]
struct SaveGlobalParams {
    file: Option<String>,
}

/// POST /admin/edit/global - Save a global config file
async fn save_global(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(params): Query<SaveGlobalParams>,
    Form(form): Form<JsonEditorForm>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let file_type = params.file.as_deref().unwrap_or("roles");

    // Validate JSON
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&form.content) {
        let err_msg = format!("error:Invalid JSON: {}", e);
        return Redirect::to(&format!("/admin/edit/global?tab={}&msg={}", file_type, urlencoding::encode(&err_msg))).into_response();
    }

    // Save to file
    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();
    drop(config);

    let file_path = match file_type {
        "roles" => format!("{}/global/roles.json", data_path),
        "assignments" => format!("{}/global/assignments.json", data_path),
        "permissions" => format!("{}/global/permissions.json", data_path),
        _ => {
            let err_msg = "error:Unknown file type";
            return Redirect::to(&format!("/admin/edit/global?msg={}", urlencoding::encode(err_msg))).into_response();
        }
    };

    // Ensure global directory exists
    let global_dir = format!("{}/global", data_path);
    if let Err(e) = tokio::fs::create_dir_all(&global_dir).await {
        warn!("Failed to create global directory: {}", e);
    }

    // Save file with explicit sync to ensure data is flushed to disk
    match tokio::fs::File::create(&file_path).await {
        Ok(file) => {
            use tokio::io::AsyncWriteExt;
            let mut file = file;
            if let Err(e) = file.write_all(form.content.as_bytes()).await {
                let err_msg = format!("error:Failed to write: {}", e);
                return Redirect::to(&format!("/admin/edit/global?tab={}&msg={}", file_type, urlencoding::encode(&err_msg))).into_response();
            }
            // Sync to disk to prevent race condition with reload
            if let Err(e) = file.sync_all().await {
                warn!("Failed to sync file to disk: {}", e);
            }
        }
        Err(e) => {
            let err_msg = format!("error:Failed to save: {}", e);
            return Redirect::to(&format!("/admin/edit/global?tab={}&msg={}", file_type, urlencoding::encode(&err_msg))).into_response();
        }
    }

    // Reload config
    let mut config = state.config_manager.write().await;
    if let Err(e) = config.load_all().await {
        warn!("Failed to reload config after save: {}", e);
    }

    info!("Global config ({}) saved via admin panel", file_type);
    Redirect::to(&format!("/admin/edit/global?tab={}&msg=saved", file_type)).into_response()
}

/// Season file path parameters
#[derive(Deserialize)]
struct SeasonFileParams {
    id: String,
    file: String,
}

/// GET /admin/edit/season/:id/:file - Edit a season config file
async fn edit_season_file(
    headers: HeaderMap,
    Path(params): Path<SeasonFileParams>,
    State(state): State<AdminState>,
    Query(query): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();
    drop(config);

    // Determine which file to load
    let file_type = match get_season_file_type(&params.file) {
        Some(ft) => ft,
        None => {
            return Html(format!(
                r#"<!DOCTYPE html><html><head><title>Not Found</title></head>
                <body style="background:#1a1a2e;color:#fff;font-family:sans-serif;padding:2rem;">
                <h1>Unknown file type: {}</h1>
                <a href="/admin/season/{}" style="color:#5865F2;">Back to season</a>
                </body></html>"#,
                params.file, params.id
            )).into_response();
        }
    };

    let file_path = format!("{}/seasons/{}/{}", data_path, params.id, file_type.file_name);
    let config_type = file_type.name;
    let description = file_type.description;

    // Read file content or use template
    let content = tokio::fs::read_to_string(&file_path)
        .await
        .unwrap_or_else(|_| get_season_file_template(config_type, &params.id));

    let message = query.get("msg").map(|m| {
        let (class, text) = if m == "saved" {
            ("success", "Configuration saved successfully!")
        } else if m.starts_with("error:") {
            ("error", m.strip_prefix("error:").unwrap_or("Unknown error"))
        } else {
            ("", m.as_str())
        };
        format!(r#"<div class="message {}">{}</div>"#, class, html_escape(text))
    }).unwrap_or_default();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Edit {} - {} - Eventy Admin</title>
    <style>{}</style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin/season/{}">← Back to Season {}</a></div>
        <h2>Edit {}.json for season {}</h2>
        {}
        <form method="POST" action="/admin/edit/season/{}/{}">
            <div class="editor-container">
                <textarea name="content" id="editor">{}</textarea>
            </div>
            <button type="submit" class="btn btn-primary">Save Changes</button>
            <a href="/admin/season/{}" class="btn btn-secondary">Cancel</a>
        </form>
        <p class="hint">{}</p>
    </div>
    <script>
        const editor = document.getElementById('editor');
        editor.addEventListener('input', function() {{
            try {{
                JSON.parse(this.value);
                this.style.borderColor = 'rgba(255,255,255,0.2)';
            }} catch (e) {{
                this.style.borderColor = '#e74c3c';
            }}
        }});
    </script>
</body>
</html>"#,
        config_type, params.id,
        editor_css(),
        params.id, params.id,
        config_type, params.id,
        message,
        params.id, config_type,
        html_escape(&content),
        params.id,
        description
    );

    Html(html).into_response()
}

/// POST /admin/edit/season/:id/:file - Save a season config file
async fn save_season_file(
    headers: HeaderMap,
    Path(params): Path<SeasonFileParams>,
    State(state): State<AdminState>,
    Form(form): Form<JsonEditorForm>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let redirect_url = format!("/admin/edit/season/{}/{}", params.id, params.file);

    // Validate JSON
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&form.content) {
        let err_msg = format!("error:Invalid JSON: {}", e);
        return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode(&err_msg))).into_response();
    }

    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();
    drop(config);

    // Determine file path
    let file_type = match get_season_file_type(&params.file) {
        Some(ft) => ft,
        None => {
            return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode("error:Unknown file type"))).into_response();
        }
    };

    let dir_path = format!("{}/seasons/{}", data_path, params.id);
    let file_path = format!("{}/{}", dir_path, file_type.file_name);

    // Ensure directory exists
    if let Err(e) = tokio::fs::create_dir_all(&dir_path).await {
        let err_msg = format!("error:Failed to create directory: {}", e);
        return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode(&err_msg))).into_response();
    }

    // Save file with explicit sync to ensure data is flushed to disk
    match tokio::fs::File::create(&file_path).await {
        Ok(file) => {
            use tokio::io::AsyncWriteExt;
            let mut file = file;
            if let Err(e) = file.write_all(form.content.as_bytes()).await {
                let err_msg = format!("error:Failed to write: {}", e);
                return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode(&err_msg))).into_response();
            }
            // Sync to disk to prevent race condition with reload
            if let Err(e) = file.sync_all().await {
                warn!("Failed to sync file to disk: {}", e);
            }
        }
        Err(e) => {
            let err_msg = format!("error:Failed to save: {}", e);
            return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode(&err_msg))).into_response();
        }
    }

    // Reload config
    let mut config = state.config_manager.write().await;
    if let Err(e) = config.load_all().await {
        warn!("Failed to reload config after save: {}", e);
    }

    info!("Season {} {} saved via admin panel", params.id, file_type.file_name);
    Redirect::to(&format!("{}?msg=saved", redirect_url)).into_response()
}

/// GET /admin/new-season - Form to create a new season
async fn new_season_form(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Query(params): Query<std::collections::HashMap<String, String>>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    let message = params.get("msg").map(|m| {
        let (class, text) = if m == "created" {
            ("success", "Season created successfully!")
        } else if m.starts_with("error:") {
            ("error", m.strip_prefix("error:").unwrap_or("Unknown error"))
        } else {
            ("", m.as_str())
        };
        format!(r#"<div class="message {}">{}</div>"#, class, html_escape(text))
    }).unwrap_or_default();

    let html = format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>New Season - Eventy Admin</title>
    <style>{}</style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin">← Back to Dashboard</a></div>
        <h2>Create New Season</h2>
        {}
        <form method="POST" action="/admin/new-season">
            <div class="editor-container">
                <div class="form-group">
                    <label for="season_id">Season ID</label>
                    <input type="text" name="season_id" id="season_id" placeholder="e.g., 2025F" required pattern="[A-Za-z0-9_-]+">
                    <p class="hint">Alphanumeric, no spaces. This will be the folder name.</p>
                </div>
                <div class="form-group">
                    <label for="name">Display Name</label>
                    <input type="text" name="name" id="name" placeholder="e.g., Fall 2025 Season">
                    <p class="hint">Optional human-readable name.</p>
                </div>
            </div>
            <button type="submit" class="btn btn-primary">Create Season</button>
            <a href="/admin" class="btn btn-secondary">Cancel</a>
        </form>
    </div>
</body>
</html>"#,
        editor_css(),
        message
    );

    Html(html).into_response()
}

/// POST /admin/new-season - Create a new season by copying from template
async fn create_season(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Form(form): Form<NewSeasonForm>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    // Validate season ID
    let season_id = form.season_id.trim();
    if season_id.is_empty() {
        let msg = urlencoding::encode("error:Season ID is required");
        return Redirect::to(&format!("/admin/new-season?msg={}", msg)).into_response();
    }

    if !season_id.chars().all(|c| c.is_alphanumeric() || c == '_' || c == '-') {
        let msg = urlencoding::encode("error:Season ID must be alphanumeric (with _ or -)");
        return Redirect::to(&format!("/admin/new-season?msg={}", msg)).into_response();
    }

    // Don't allow creating a season called "template"
    if season_id.eq_ignore_ascii_case("template") {
        let msg = urlencoding::encode("error:Cannot create a season named 'template'");
        return Redirect::to(&format!("/admin/new-season?msg={}", msg)).into_response();
    }

    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();

    // Check if season already exists
    if config.get_seasons().contains_key(season_id) {
        return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode("error:Season already exists"))).into_response();
    }
    drop(config);

    let template_dir = format!("{}/seasons/template", data_path);
    let season_dir = format!("{}/seasons/{}", data_path, season_id);

    // Create season directory
    if let Err(e) = tokio::fs::create_dir_all(&season_dir).await {
        let err_msg = format!("error:Failed to create directory: {}", e);
        return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode(&err_msg))).into_response();
    }

    // Copy and modify template files
    let files_to_copy = ["users.json", "category.json", "roles.json"];
    for filename in files_to_copy {
        let template_path = format!("{}/{}", template_dir, filename);
        let dest_path = format!("{}/{}", season_dir, filename);

        // Read template file
        let content = match tokio::fs::read_to_string(&template_path).await {
            Ok(c) => c,
            Err(e) => {
                warn!("Template file {} not found, skipping: {}", filename, e);
                continue;
            }
        };

        // Parse and modify JSON to replace TEMPLATE with actual season ID
        let modified_content = match serde_json::from_str::<serde_json::Value>(&content) {
            Ok(mut json) => {
                // Replace season_id field if present
                if let Some(obj) = json.as_object_mut() {
                    if obj.contains_key("season_id") {
                        obj.insert("season_id".to_string(), serde_json::json!(season_id));
                    }
                    // For users.json: update name and set active to true, clear users
                    if filename == "users.json" {
                        obj.insert("name".to_string(), serde_json::json!(form.name.trim()));
                        obj.insert("active".to_string(), serde_json::json!(true));
                        obj.insert("users".to_string(), serde_json::json!([]));
                    }
                    // For category.json: update category_name
                    if filename == "category.json" {
                        if let Some(name) = form.name.trim().chars().next() {
                            // Only set if name is non-empty
                            if !form.name.trim().is_empty() {
                                obj.insert("category_name".to_string(), serde_json::json!(form.name.trim()));
                            } else {
                                obj.insert("category_name".to_string(), serde_json::json!(season_id));
                            }
                            let _ = name; // silence warning
                        }
                    }
                }
                serde_json::to_string_pretty(&json).unwrap_or(content)
            }
            Err(_) => content, // If JSON parse fails, copy as-is
        };

        // Write modified content
        match tokio::fs::File::create(&dest_path).await {
            Ok(file) => {
                use tokio::io::AsyncWriteExt;
                let mut file = file;
                if let Err(e) = file.write_all(modified_content.as_bytes()).await {
                    let err_msg = format!("error:Failed to write {}: {}", filename, e);
                    return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode(&err_msg))).into_response();
                }
                if let Err(e) = file.sync_all().await {
                    warn!("Failed to sync {} to disk: {}", filename, e);
                }
            }
            Err(e) => {
                let err_msg = format!("error:Failed to create {}: {}", filename, e);
                return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode(&err_msg))).into_response();
            }
        }
    }

    // Reload config
    let mut config = state.config_manager.write().await;
    if let Err(e) = config.load_all().await {
        warn!("Failed to reload config after creating season: {}", e);
    }

    info!("Season {} created from template via admin panel", season_id);
    Redirect::to(&format!("/admin/season/{}", season_id)).into_response()
}

/// POST /admin/restart - Restart the bot
async fn restart_bot(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    warn!(
        "Bot restart initiated by {} ({}) via admin panel",
        session.username, session.discord_id
    );

    // Return a page that shows restart message, then exit
    let html = r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>Restarting - Eventy Admin</title>
    <style>
        body {
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
            display: flex;
            justify-content: center;
            align-items: center;
        }
        .container {
            text-align: center;
            background: rgba(255,255,255,0.05);
            padding: 3rem;
            border-radius: 16px;
            border: 1px solid rgba(255,255,255,0.1);
        }
        h1 { margin-bottom: 1rem; }
        .spinner {
            width: 40px;
            height: 40px;
            border: 4px solid rgba(255,255,255,0.1);
            border-top-color: #5865F2;
            border-radius: 50%;
            animation: spin 1s linear infinite;
            margin: 1.5rem auto;
        }
        @keyframes spin {
            to { transform: rotate(360deg); }
        }
        p { color: #a0a0a0; }
    </style>
    <script>
        // Try to reconnect after a delay
        setTimeout(function() {
            window.location.href = '/admin';
        }, 5000);
    </script>
</head>
<body>
    <div class="container">
        <h1>Restarting Bot...</h1>
        <div class="spinner"></div>
        <p>The bot is restarting. You will be redirected automatically.</p>
        <p style="margin-top: 1rem; font-size: 0.85rem;">If not redirected, <a href="/admin" style="color:#5865F2;">click here</a>.</p>
    </div>
</body>
</html>"#;

    // Spawn a task to exit after response is sent
    tokio::spawn(async {
        tokio::time::sleep(std::time::Duration::from_millis(500)).await;
        std::process::exit(0);
    });

    Html(html).into_response()
}

/// Handle a role operation error and return a user-friendly message
fn handle_role_error(role_name: &str, error: &serenity::Error) -> String {
    let err_str = error.to_string();

    if err_str.contains("Missing Permissions") || err_str.contains("50013") {
        let msg = format!("{}: Role hierarchy issue - move bot's role above '{}' in Discord server settings", role_name, role_name);
        error!("{}", msg);
        msg
    } else {
        error!("Failed to sync role '{}': {}", role_name, error);
        format!("{}: {}", role_name, error)
    }
}

/// Season file type configuration
struct SeasonFileType {
    name: &'static str,        // e.g., "users"
    file_name: &'static str,   // e.g., "users.json"
    description: &'static str,
}

/// Get file configuration for a season file type
fn get_season_file_type(file_type: &str) -> Option<SeasonFileType> {
    match file_type {
        "users" | "users.json" => Some(SeasonFileType {
            name: "users",
            file_name: "users.json",
            description: "User list with verification IDs",
        }),
        "season" | "season.json" => Some(SeasonFileType {
            name: "season",
            file_name: "season.json",
            description: "Season configuration (name, active, channels)",
        }),
        "category" | "category.json" => Some(SeasonFileType {
            name: "category",
            file_name: "category.json",
            description: "Category and channel structure",
        }),
        "roles" | "roles.json" => Some(SeasonFileType {
            name: "roles",
            file_name: "roles.json",
            description: "Special role assignments",
        }),
        _ => None,
    }
}

/// Get default template content for a season file type
fn get_season_file_template(file_type: &str, season_id: &str) -> String {
    match file_type {
        "users" | "users.json" => "[]".to_string(),
        "season" | "season.json" => format!(r#"{{
  "name": "{}",
  "active": true,
  "channels": []
}}"#, season_id),
        "category" | "category.json" => r#"{
  "category_name": "Season Category",
  "channels": []
}"#.to_string(),
        "roles" | "roles.json" => r#"{
  "roles": {}
}"#.to_string(),
        _ => "{}".to_string(),
    }
}

/// POST /admin/sync/roles - Sync roles to Discord
async fn sync_roles(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    // Diagnostic logging - this should always appear
    eprintln!("[SYNC_ROLES] Handler called - starting role sync");
    info!("=== SYNC ROLES STARTED ===");

    let _session = match require_auth(&headers, &state).await {
        Ok(s) => {
            info!("sync_roles: authenticated as {}", s.username);
            s
        }
        Err(redirect) => {
            warn!("sync_roles: authentication failed");
            return redirect;
        }
    };

    info!("sync_roles: loading config...");
    let config = state.config_manager.read().await;
    let global_roles = match config.get_global_roles() {
        Some(r) => {
            info!("sync_roles: found {} roles in config", r.roles.len());
            r.clone()
        }
        None => {
            warn!("sync_roles: no global roles configured");
            return Html(sync_result_page(
                "Sync Roles",
                false,
                "No global roles configured. Add roles to `data/global/roles.json`.",
                "/admin",
            )).into_response();
        }
    };
    drop(config);

    let http = state.serenity_http.as_ref();
    let guild_id = state.guild_id;
    info!("sync_roles: using guild_id {}", guild_id);

    // Get existing roles
    info!("sync_roles: fetching existing roles from Discord...");
    let existing_roles = match guild_id.roles(http).await {
        Ok(r) => {
            info!("sync_roles: Discord has {} existing roles", r.len());
            r
        }
        Err(e) => {
            error!("sync_roles: failed to fetch roles from Discord: {}", e);
            return Html(sync_result_page(
                "Sync Roles",
                false,
                &format!("Failed to fetch roles from Discord: {}", e),
                "/admin",
            )).into_response();
        }
    };

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = Vec::new();
    let mut errors = Vec::new();

    for role_def in &global_roles.roles {
        if let Some((role_id, existing_role)) = existing_roles.iter().find(|(_, r)| r.name == role_def.name) {
            let target_color = role_def
                .color
                .as_ref()
                .and_then(|c| {
                    let hex = c.trim_start_matches('#');
                    u32::from_str_radix(hex, 16).ok()
                })
                .unwrap_or(0);

            let needs_update = existing_role.colour.0 != target_color
                || existing_role.hoist != role_def.hoist
                || existing_role.mentionable != role_def.mentionable;

            if needs_update {
                match guild_id
                    .edit_role(
                        http,
                        *role_id,
                        serenity::EditRole::new()
                            .colour(target_color as u64)
                            .hoist(role_def.hoist)
                            .mentionable(role_def.mentionable),
                    )
                    .await
                {
                    Ok(_) => {
                        info!("Updated role '{}' via admin panel", role_def.name);
                        updated.push(role_def.name.clone());
                    }
                    Err(e) => {
                        errors.push(handle_role_error(&role_def.name, &e));
                    }
                }
            } else {
                unchanged.push(role_def.name.clone());
            }
        } else {
            // Create new role
            let color = role_def
                .color
                .as_ref()
                .and_then(|c| {
                    let hex = c.trim_start_matches('#');
                    u32::from_str_radix(hex, 16).ok().map(serenity::Colour::new)
                })
                .unwrap_or(serenity::Colour::default());

            match guild_id
                .create_role(
                    http,
                    serenity::EditRole::new()
                        .name(&role_def.name)
                        .colour(color)
                        .hoist(role_def.hoist)
                        .mentionable(role_def.mentionable),
                )
                .await
            {
                Ok(role) => {
                    info!("Created role '{}' (ID: {}) via admin panel", role_def.name, role.id);
                    created.push(role_def.name.clone());
                }
                Err(e) => {
                    errors.push(handle_role_error(&role_def.name, &e));
                }
            }
        }
    }

    // Build result message
    let mut message = String::new();
    if !created.is_empty() {
        message.push_str(&format!("<p><strong>Created ({}):</strong> {}</p>", created.len(), created.join(", ")));
    }
    if !updated.is_empty() {
        message.push_str(&format!("<p><strong>Updated ({}):</strong> {}</p>", updated.len(), updated.join(", ")));
    }
    if !unchanged.is_empty() {
        message.push_str(&format!("<p><strong>Unchanged ({}):</strong> {}</p>", unchanged.len(), unchanged.join(", ")));
    }
    if !errors.is_empty() {
        message.push_str(&format!("<p style=\"color:#e74c3c;\"><strong>Errors ({}):</strong> {}</p>", errors.len(), errors.join(", ")));
    }
    if message.is_empty() {
        message = String::from("<p>No roles to sync.</p>");
    }

    Html(sync_result_page(
        "Sync Roles",
        errors.is_empty(),
        &message,
        "/admin",
    )).into_response()
}

/// POST /admin/sync/assignments - Sync role assignments to all verified Discord members
async fn sync_assignments(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    info!("=== SYNC ASSIGNMENTS STARTED ===");

    let _session = match require_auth(&headers, &state).await {
        Ok(s) => {
            info!("sync_assignments: authenticated as {}", s.username);
            s
        }
        Err(redirect) => {
            warn!("sync_assignments: authentication failed");
            return redirect;
        }
    };

    // Load assignments config
    let config = state.config_manager.read().await;
    let special_members = match config.get_special_members() {
        Some(sm) => sm.clone(),
        None => {
            warn!("sync_assignments: no special members configured");
            return Html(sync_result_page(
                "Sync Assignments",
                false,
                "No assignments configured. Add role assignments to `data/global/assignments.json`.",
                "/admin/edit/global?tab=assignments",
            )).into_response();
        }
    };
    drop(config);

    let http = state.serenity_http.as_ref();
    let guild_id = state.guild_id;

    // Get all members in the guild
    info!("sync_assignments: fetching guild members...");
    let members = match guild_id.members(http, None, None).await {
        Ok(m) => {
            info!("sync_assignments: fetched {} members", m.len());
            m
        }
        Err(e) => {
            error!("sync_assignments: failed to fetch guild members: {}", e);
            return Html(sync_result_page(
                "Sync Assignments",
                false,
                &format!("Failed to fetch guild members: {}", e),
                "/admin/edit/global?tab=assignments",
            )).into_response();
        }
    };

    // Get all managed role names (roles in assignments.json)
    let all_assignment_roles = special_members.get_all_role_names();
    info!("sync_assignments: managed roles: {:?}", all_assignment_roles);

    let role_manager = state.role_manager.read().await;

    // Pre-build a map of role_name -> role_id for all managed roles
    let mut managed_role_ids = std::collections::HashMap::new();
    for role_name in &all_assignment_roles {
        if let Ok(role_id) = role_manager.get_role_id(http, guild_id, role_name).await {
            managed_role_ids.insert(role_name.clone(), role_id);
        }
    }

    let mut total_added = Vec::new();
    let mut total_removed = Vec::new();
    let mut total_failed = Vec::new();
    let mut users_processed = 0;

    for member in &members {
        let discord_username = &member.user.name;
        let user_id = member.user.id;

        // Get special roles for this user (what they SHOULD have)
        let desired_roles = special_members.get_roles_for_user(discord_username);

        // Process ALL members who either have or should have assignment roles
        // This ensures we remove roles from users who were removed from assignments.json
        let has_any_managed_role = managed_role_ids.values().any(|role_id| {
            member.roles.contains(role_id)
        });

        if desired_roles.is_empty() && !has_any_managed_role {
            continue; // Skip users with no special roles and no managed roles
        }

        users_processed += 1;
        info!(
            "sync_assignments: processing user '{}' ({}) - desired: {:?}",
            discord_username,
            user_id,
            desired_roles
        );

        let (added, removed, failed) = role_manager
            .full_sync_assignments_for_user(
                http,
                guild_id,
                user_id,
                discord_username,
                &desired_roles,
                &all_assignment_roles,
            )
            .await;

        for role in added {
            total_added.push(format!("{} +{}", discord_username, role));
        }
        for role in removed {
            total_removed.push(format!("{} -{}", discord_username, role));
        }
        for err in failed {
            total_failed.push(format!("{}: {}", discord_username, err));
        }
    }

    drop(role_manager);

    // Build result message
    let mut message = format!("<p><strong>Users processed:</strong> {}</p>", users_processed);

    if !total_added.is_empty() {
        message.push_str(&format!(
            "<p><strong>Roles added ({}):</strong></p><ul>{}</ul>",
            total_added.len(),
            total_added.iter().map(|r| format!("<li>{}</li>", html_escape(r))).collect::<Vec<_>>().join("")
        ));
    }

    if !total_removed.is_empty() {
        message.push_str(&format!(
            "<p><strong>Roles removed ({}):</strong></p><ul>{}</ul>",
            total_removed.len(),
            total_removed.iter().map(|r| format!("<li>{}</li>", html_escape(r))).collect::<Vec<_>>().join("")
        ));
    }

    if !total_failed.is_empty() {
        message.push_str(&format!(
            "<p style=\"color:#e74c3c;\"><strong>Failed ({}):</strong></p><ul>{}</ul>",
            total_failed.len(),
            total_failed.iter().map(|r| format!("<li>{}</li>", html_escape(r))).collect::<Vec<_>>().join("")
        ));
    }

    if total_added.is_empty() && total_removed.is_empty() && total_failed.is_empty() && users_processed > 0 {
        message.push_str("<p>All users already have the correct roles.</p>");
    } else if users_processed == 0 {
        message.push_str("<p>No changes needed.</p>");
    }

    info!(
        "sync_assignments: completed - {} added, {} removed, {} failed",
        total_added.len(),
        total_removed.len(),
        total_failed.len()
    );

    Html(sync_result_page(
        "Sync Assignments",
        total_failed.is_empty(),
        &message,
        "/admin/edit/global?tab=assignments",
    )).into_response()
}

/// POST /admin/sync/season/:id - Sync season category to Discord
async fn sync_season(
    headers: HeaderMap,
    Path(season_id): Path<String>,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    info!("=== SYNC SEASON STARTED for '{}' ===", season_id);

    let _session = match require_auth(&headers, &state).await {
        Ok(s) => {
            info!("sync_season: authenticated as {}", s.username);
            s
        }
        Err(redirect) => {
            warn!("sync_season: authentication failed");
            return redirect;
        }
    };

    // Load season config
    let config = state.config_manager.read().await;
    let season = match config.get_season(&season_id) {
        Some(s) => {
            info!("sync_season: found season '{}' with {} channels", season_id, s.channels().len());
            s.clone()
        }
        None => {
            warn!("sync_season: season '{}' not found", season_id);
            return Html(sync_result_page(
                &format!("Sync Season {}", season_id),
                false,
                &format!("Season '{}' not found.", season_id),
                &format!("/admin/season/{}", season_id),
            )).into_response();
        }
    };
    drop(config);

    if season.channels().is_empty() {
        return Html(sync_result_page(
            &format!("Sync Season {}", season_id),
            false,
            &format!("Season '{}' has no channels defined in season.json.", season_id),
            &format!("/admin/season/{}", season_id),
        )).into_response();
    }

    let http = state.serenity_http.as_ref();
    let guild_id = state.guild_id;
    let category_name = season.name().to_string();
    let channels = season.channels().to_vec();

    // Use channel_manager to sync
    let channel_manager = state.channel_manager.read().await;
    let summary = match channel_manager.sync_season_channels(http, guild_id, &category_name, &channels).await {
        Ok(s) => s,
        Err(e) => {
            error!("Failed to sync season '{}': {}", season_id, e);
            return Html(sync_result_page(
                &format!("Sync Season {}", season_id),
                false,
                &format!("Failed to sync season: {}", e),
                &format!("/admin/season/{}", season_id),
            )).into_response();
        }
    };
    drop(channel_manager);

    // Build result message from summary
    let mut message = String::new();

    if let Some(cat) = &summary.category_created {
        message.push_str(&format!("<p><strong>Category created:</strong> {}</p>", cat));
    } else if let Some(cat) = &summary.category_existing {
        message.push_str(&format!("<p><strong>Category:</strong> {}</p>", cat));
    }

    if !summary.channels_created.is_empty() {
        message.push_str(&format!("<p><strong>Channels created ({}):</strong> {}</p>",
            summary.channels_created.len(),
            summary.channels_created.iter().map(|c| format!("#{}", c)).collect::<Vec<_>>().join(", ")
        ));
    }

    if !summary.channels_updated.is_empty() {
        message.push_str(&format!("<p><strong>Channels updated ({}):</strong> {}</p>",
            summary.channels_updated.len(),
            summary.channels_updated.iter().map(|c| format!("#{}", c)).collect::<Vec<_>>().join(", ")
        ));
    }

    let has_missing_roles = !summary.missing_roles.is_empty();
    if has_missing_roles {
        warn!("Missing roles for season '{}': {}", season_id, summary.missing_roles.join(", "));
        message.push_str(&format!(
            "<p style=\"color:#f39c12;\"><strong>Warning:</strong> The following roles were not found and permissions were not set: {}</p>",
            summary.missing_roles.join(", ")
        ));
        message.push_str("<p style=\"color:#a0a0a0;font-size:0.9rem;\">Make sure to sync roles first, or check that role names match exactly.</p>");
    }

    if !summary.warnings.is_empty() {
        for warning in &summary.warnings {
            message.push_str(&format!("<p style=\"color:#f39c12;\">{}</p>", warning));
        }
    }

    let success = summary.warnings.is_empty() && !has_missing_roles;
    Html(sync_result_page(
        &format!("Sync Season {}", season_id),
        success,
        &message,
        &format!("/admin/season/{}", season_id),
    )).into_response()
}

/// Generate a sync result page
fn sync_result_page(title: &str, success: bool, message: &str, back_url: &str) -> String {
    let status_color = if success { "#2ecc71" } else { "#e74c3c" };
    let status_text = if success { "Success" } else { "Error" };

    format!(
        r#"<!DOCTYPE html>
<html lang="en">
<head>
    <meta charset="UTF-8">
    <meta name="viewport" content="width=device-width, initial-scale=1.0">
    <title>{} - Eventy Admin</title>
    <style>
        * {{ box-sizing: border-box; margin: 0; padding: 0; }}
        body {{
            font-family: -apple-system, BlinkMacSystemFont, 'Segoe UI', Roboto, sans-serif;
            background: #1a1a2e;
            min-height: 100vh;
            color: #fff;
            display: flex;
            justify-content: center;
            align-items: center;
        }}
        .container {{
            background: rgba(255,255,255,0.05);
            border-radius: 12px;
            padding: 2rem;
            max-width: 600px;
            width: 90%;
            border: 1px solid rgba(255,255,255,0.1);
        }}
        h1 {{
            margin-bottom: 1rem;
            display: flex;
            align-items: center;
            gap: 0.5rem;
        }}
        .status {{
            display: inline-block;
            padding: 0.25rem 0.75rem;
            border-radius: 4px;
            font-size: 0.85rem;
            background: {};
        }}
        .message {{
            margin: 1.5rem 0;
            line-height: 1.6;
        }}
        .message p {{
            margin-bottom: 0.5rem;
        }}
        .back {{
            display: inline-block;
            padding: 0.75rem 1.5rem;
            background: #5865F2;
            color: #fff;
            text-decoration: none;
            border-radius: 8px;
        }}
        .back:hover {{
            background: #4752c4;
        }}
    </style>
</head>
<body>
    <div class="container">
        <h1>{} <span class="status">{}</span></h1>
        <div class="message">{}</div>
        <a href="{}" class="back">Back</a>
    </div>
</body>
</html>"#,
        title,
        status_color,
        title,
        status_text,
        message,
        back_url
    )
}
