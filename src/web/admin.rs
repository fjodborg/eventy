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
    routing::{get, post},
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
    access_denied_page, admin_oauth_url, check_admin_permissions, create_logout_cookie,
    create_session_cookie, get_session_token, login_page, AdminCallbackParams, AdminSession,
    SharedSessionStore,
};
use super::oauth::OAuthState;
use crate::logging::SharedLogBuffer;
use crate::managers::SharedConfigManager;

/// Extended app state for admin panel
#[derive(Clone)]
pub struct AdminState {
    pub oauth: OAuthState,
    pub config_manager: SharedConfigManager,
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

    // Check if user has admin permissions
    let user_id = UserId::new(discord_id.parse().unwrap_or(0));
    let is_admin = check_admin_permissions(&state.serenity_http, state.guild_id, user_id).await;

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

    // Get season info
    let config = state.config_manager.read().await;
    let seasons: Vec<_> = config
        .get_seasons()
        .iter()
        .map(|(id, season)| {
            format!(
                "<tr><td><a href=\"/admin/season/{}\">{}</a></td><td>{}</td><td>{}</td></tr>",
                id,
                id,
                if season.name.is_empty() { id } else { &season.name },
                season.users.len()
            )
        })
        .collect();

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
            <a href="/admin/edit/global">Edit Global Config</a>
            <a href="/admin/new-season">New Season</a>
            <a href="/admin/logs">Logs</a>
        </div>
        <div class="cards">
            <div class="card">
                <h2>Seasons</h2>
                <div class="value">{}</div>
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
    let session = match require_auth(&headers, &state).await {
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
            <a href="/admin/edit/season/{}/category" class="btn btn-secondary" style="background:rgba(255,255,255,0.1);color:#fff;padding:0.5rem 1rem;border-radius:6px;text-decoration:none;margin-right:0.5rem;">Edit category.json</a>
            <a href="/admin/edit/season/{}/roles" class="btn btn-secondary" style="background:rgba(255,255,255,0.1);color:#fff;padding:0.5rem 1rem;border-radius:6px;text-decoration:none;">Edit roles.json</a>
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
        if season.name.is_empty() { &season_id } else { &season.name },
        season.users.len(),
        season.active,
        season_id, // edit users link
        season_id, // edit category link
        season_id, // edit roles link
        users_html.join("\n")
    );

    Html(html).into_response()
}

/// GET /admin/logs - Log viewer page
async fn logs_page(
    headers: HeaderMap,
    State(state): State<AdminState>,
) -> impl IntoResponse {
    let session = match require_auth(&headers, &state).await {
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

/// GET /admin/edit/global - Edit global.json
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

    let content = match config.export_config("global", None) {
        Ok((_, bytes)) => String::from_utf8_lossy(&bytes).to_string(),
        Err(_) => "{}".to_string(),
    };

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
    <style>{}</style>
</head>
<body>
    <nav class="navbar">
        <h1>Eventy Admin</h1>
        <a href="/admin/logout">Logout</a>
    </nav>
    <div class="container">
        <div class="back"><a href="/admin">← Back to Dashboard</a></div>
        <h2>Edit Global Configuration</h2>
        {}
        <form method="POST" action="/admin/edit/global">
            <div class="editor-container">
                <textarea name="content" id="editor">{}</textarea>
            </div>
            <button type="submit" class="btn btn-primary">Save Changes</button>
            <a href="/admin" class="btn btn-secondary">Cancel</a>
        </form>
        <p class="hint">This file controls global settings like default roles, channels, and permission definitions.</p>
    </div>
    <script>
        // Basic JSON validation on input
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
        editor_css(),
        message,
        html_escape(&content)
    );

    Html(html).into_response()
}

/// POST /admin/edit/global - Save global.json
async fn save_global(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Form(form): Form<JsonEditorForm>,
) -> impl IntoResponse {
    let _session = match require_auth(&headers, &state).await {
        Ok(s) => s,
        Err(redirect) => return redirect,
    };

    // Validate JSON
    if let Err(e) = serde_json::from_str::<serde_json::Value>(&form.content) {
        let err_msg = format!("error:Invalid JSON: {}", e);
        return Redirect::to(&format!("/admin/edit/global?msg={}", urlencoding::encode(&err_msg))).into_response();
    }

    // Save to file
    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();
    drop(config);

    let file_path = format!("{}/global.json", data_path);

    // Save file with explicit sync to ensure data is flushed to disk
    match tokio::fs::File::create(&file_path).await {
        Ok(file) => {
            use tokio::io::AsyncWriteExt;
            let mut file = file;
            if let Err(e) = file.write_all(form.content.as_bytes()).await {
                let err_msg = format!("error:Failed to write: {}", e);
                return Redirect::to(&format!("/admin/edit/global?msg={}", urlencoding::encode(&err_msg))).into_response();
            }
            // Sync to disk to prevent race condition with reload
            if let Err(e) = file.sync_all().await {
                warn!("Failed to sync file to disk: {}", e);
            }
        }
        Err(e) => {
            let err_msg = format!("error:Failed to save: {}", e);
            return Redirect::to(&format!("/admin/edit/global?msg={}", urlencoding::encode(&err_msg))).into_response();
        }
    }

    // Reload config
    let mut config = state.config_manager.write().await;
    if let Err(e) = config.load_all().await {
        warn!("Failed to reload config after save: {}", e);
    }

    info!("Global config saved via admin panel");
    Redirect::to("/admin/edit/global?msg=saved").into_response()
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

    // Determine which file to load
    let (file_path, config_type, description) = match params.file.as_str() {
        "users" | "users.json" => (
            format!("{}/seasons/{}/users.json", data_path, params.id),
            "users",
            "User list with verification IDs"
        ),
        "category" | "category.json" => (
            format!("{}/seasons/{}/category.json", data_path, params.id),
            "category",
            "Category and channel structure"
        ),
        "roles" | "roles.json" => (
            format!("{}/seasons/{}/roles.json", data_path, params.id),
            "roles",
            "Special role assignments"
        ),
        _ => {
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
    drop(config);

    // Read file content
    let content = match tokio::fs::read_to_string(&file_path).await {
        Ok(c) => c,
        Err(_) => {
            // File doesn't exist, provide template
            match config_type {
                "users" => format!(r#"{{
  "season_id": "{}",
  "name": "",
  "active": true,
  "users": []
}}"#, params.id),
                "category" => r#"{
  "category_name": "Season Category",
  "channels": []
}"#.to_string(),
                "roles" => r#"{
  "roles": {}
}"#.to_string(),
                _ => "{}".to_string(),
            }
        }
    };

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
    let file_name = match params.file.as_str() {
        "users" | "users.json" => "users.json",
        "category" | "category.json" => "category.json",
        "roles" | "roles.json" => "roles.json",
        _ => {
            return Redirect::to(&format!("{}?msg={}", redirect_url, urlencoding::encode("error:Unknown file type"))).into_response();
        }
    };

    let dir_path = format!("{}/seasons/{}", data_path, params.id);
    let file_path = format!("{}/{}", dir_path, file_name);

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

    info!("Season {} {} saved via admin panel", params.id, file_name);
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

/// POST /admin/new-season - Create a new season
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

    let config = state.config_manager.read().await;
    let data_path = config.get_data_path().to_string();

    // Check if season already exists
    if config.get_seasons().contains_key(season_id) {
        return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode("error:Season already exists"))).into_response();
    }
    drop(config);

    // Create season directory
    let season_dir = format!("{}/seasons/{}", data_path, season_id);
    if let Err(e) = tokio::fs::create_dir_all(&season_dir).await {
        let err_msg = format!("error:Failed to create directory: {}", e);
        return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode(&err_msg))).into_response();
    }

    // Create default users.json with correct SeasonConfig structure
    let users_content = serde_json::json!({
        "season_id": season_id,
        "name": form.name.trim(),
        "active": true,
        "users": []
    });

    let users_path = format!("{}/users.json", season_dir);
    if let Err(e) = tokio::fs::write(&users_path, serde_json::to_string_pretty(&users_content).unwrap()).await {
        let err_msg = format!("error:Failed to create users.json: {}", e);
        return Redirect::to(&format!("/admin/new-season?msg={}", urlencoding::encode(&err_msg))).into_response();
    }

    // Reload config
    let mut config = state.config_manager.write().await;
    if let Err(e) = config.load_all().await {
        warn!("Failed to reload config after creating season: {}", e);
    }

    info!("Season {} created via admin panel", season_id);
    Redirect::to(&format!("/admin/season/{}", season_id)).into_response()
}
