//! REST API handlers for programmatic season management.
//!
//! Mounted under `/admin/api/v1/` and authenticated via a bearer token.

use super::admin::AdminState;
use crate::config::{ChannelDefinition, SeasonConfig, SeasonUser};
use crate::managers::SharedConfigManager;
use axum::{
    extract::State,
    http::{HeaderMap, StatusCode},
    routing::post,
    Json, Router,
};
use poise::serenity_prelude::{self as serenity, GuildId};
use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use tracing::{info, warn};

// ── Request / response types ──────────────────────────────────────────────────

#[derive(Debug, Deserialize)]
struct InitializeUsersRequest {
    #[serde(alias = "name")]
    season_name: String,
    #[serde(alias = "member_role")]
    role_name: String,
    users: Vec<SeasonUser>,
}

#[derive(Debug, Serialize)]
struct ApiErrorResponse {
    error: String,
    message: String,
}

#[derive(Debug, Serialize)]
struct BootstrapSeasonResponse {
    status: String,
    season_id: String,
    source_season_id: String,
    activated_season_id: String,
    deactivated_seasons: Vec<String>,
    sync: Option<ApiSyncSummary>,
    sync_error: Option<String>,
}

#[derive(Debug, Serialize)]
struct ApiSyncSummary {
    category_created: Option<String>,
    category_existing: Option<String>,
    channels_created: Vec<String>,
    channels_updated: Vec<String>,
    channels_reordered: Vec<String>,
    missing_roles: Vec<String>,
    warnings: Vec<String>,
}

impl From<crate::managers::channel_manager::UpdateSummary> for ApiSyncSummary {
    fn from(s: crate::managers::channel_manager::UpdateSummary) -> Self {
        Self {
            category_created: s.category_created,
            category_existing: s.category_existing,
            channels_created: s.channels_created,
            channels_updated: s.channels_updated,
            channels_reordered: s.channels_reordered,
            missing_roles: s.missing_roles,
            warnings: s.warnings,
        }
    }
}

type ApiResult<T> = Result<T, (StatusCode, Json<ApiErrorResponse>)>;

fn api_err(status: StatusCode, error: &str, message: String) -> (StatusCode, Json<ApiErrorResponse>) {
    (status, Json(ApiErrorResponse { error: error.to_string(), message }))
}

// ── Auth ──────────────────────────────────────────────────────────────────────

fn require_api_token(headers: &HeaderMap) -> ApiResult<()> {
    let expected = match std::env::var("ADMIN_API_TOKEN") {
        Ok(t) if !t.trim().is_empty() => t,
        _ => {
            return Err(api_err(
                StatusCode::SERVICE_UNAVAILABLE,
                "api_token_not_configured",
                "ADMIN_API_TOKEN is not configured".to_string(),
            ))
        }
    };

    let provided = headers
        .get(axum::http::header::AUTHORIZATION)
        .and_then(|v| v.to_str().ok())
        .and_then(|v| v.strip_prefix("Bearer "))
        .map(str::trim)
        .unwrap_or("");

    if provided.is_empty() || provided != expected {
        return Err(api_err(
            StatusCode::UNAUTHORIZED,
            "unauthorized",
            "Missing or invalid bearer token".to_string(),
        ));
    }
    Ok(())
}

// ── Validation ────────────────────────────────────────────────────────────────

fn validate_season_folder_name(season_id: &str) -> ApiResult<()> {
    if season_id.is_empty() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_season_id",
            "Season name is required".to_string(),
        ));
    }
    if season_id == "." || season_id == ".." {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_season_id",
            "Season name cannot be '.' or '..'".to_string(),
        ));
    }
    if season_id.eq_ignore_ascii_case("template") {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_season_id",
            "Season name 'template' is reserved".to_string(),
        ));
    }
    if season_id.contains('/') || season_id.contains('\\') || season_id.contains('\0') {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_season_id",
            "Season name cannot contain path separators".to_string(),
        ));
    }
    if season_id.chars().any(|c| c.is_control()) {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_season_id",
            "Season name cannot contain control characters".to_string(),
        ));
    }
    Ok(())
}

// ── Channel helpers ───────────────────────────────────────────────────────────

fn remap_member_role_in_channels(
    channels: &[ChannelDefinition],
    source_role: &str,
    target_role: &str,
) -> Vec<ChannelDefinition> {
    channels
        .iter()
        .cloned()
        .map(|mut ch| {
            if source_role != target_role {
                if let Some(level) = ch.role_permissions.get(source_role).cloned() {
                    ch.role_permissions.remove(source_role);
                    ch.role_permissions
                        .entry(target_role.to_string())
                        .or_insert(level);
                }
            }
            ch.children =
                remap_member_role_in_channels(&ch.children, source_role, target_role);
            ch
        })
        .collect()
}

// ── Phase helpers ─────────────────────────────────────────────────────────────

struct SeasonSourceInfo {
    data_path: String,
    source_season_id: String,
    source_member_role: String,
    source_channels: Vec<ChannelDefinition>,
    existing_seasons: HashMap<String, SeasonConfig>,
    deactivated_season_ids: Vec<String>,
    season_existed: bool,
    keep_active_flag: bool,
}

async fn resolve_season_source(
    config_manager: &SharedConfigManager,
    season_id: &str,
) -> ApiResult<SeasonSourceInfo> {
    let config = config_manager.read().await;
    let target_existing = config.get_season(season_id).cloned();

    let active_seasons: Vec<_> = config
        .get_seasons()
        .iter()
        .filter(|(_, s)| s.is_active())
        .collect();

    if target_existing.is_none() {
        if active_seasons.is_empty() {
            return Err(api_err(
                StatusCode::CONFLICT,
                "no_active_season",
                "No active season found. Please activate the newest season first.".to_string(),
            ));
        }
        if active_seasons.len() > 1 {
            let mut ids: Vec<String> =
                active_seasons.iter().map(|(id, _)| (*id).clone()).collect();
            ids.sort();
            return Err(api_err(
                StatusCode::CONFLICT,
                "multiple_active_seasons",
                format!(
                    "Multiple active seasons detected: {}. Please deactivate the non-newest \
                     season(s) and keep exactly one active season before retrying.",
                    ids.join(", ")
                ),
            ));
        }
    }

    let (source_season_id, source_member_role, source_channels, keep_active_flag) =
        if let Some(existing) = &target_existing {
            (
                existing.season_id.clone(),
                existing.member_role(),
                existing.channels().to_vec(),
                existing.is_active(),
            )
        } else {
            let (active_id, active_season) = active_seasons[0];
            (
                active_id.to_string(),
                active_season.member_role(),
                active_season.channels().to_vec(),
                true,
            )
        };

    let existing_seasons: HashMap<String, SeasonConfig> = config
        .get_seasons()
        .iter()
        .map(|(id, s)| (id.clone(), s.config.clone()))
        .collect();

    let deactivated_season_ids = if target_existing.is_some() {
        Vec::new()
    } else {
        config
            .get_seasons()
            .iter()
            .filter(|(id, s)| id.as_str() != season_id && s.is_active())
            .map(|(id, _)| id.clone())
            .collect()
    };

    Ok(SeasonSourceInfo {
        data_path: config.get_data_path().to_string(),
        source_season_id,
        source_member_role,
        source_channels,
        existing_seasons,
        deactivated_season_ids,
        season_existed: target_existing.is_some(),
        keep_active_flag,
    })
}

async fn write_season_files(
    data_path: &str,
    season_id: &str,
    config: &SeasonConfig,
    users: &[SeasonUser],
) -> ApiResult<()> {
    let season_dir = format!("{}/seasons/{}", data_path, season_id);

    tokio::fs::create_dir_all(&season_dir).await.map_err(|e| {
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "create_directory_failed",
            format!("Failed to create season directory: {}", e),
        )
    })?;

    let season_bytes = serde_json::to_vec_pretty(config).map_err(|e| {
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            format!("Failed to serialize season config: {}", e),
        )
    })?;

    let users_bytes = serde_json::to_vec_pretty(users).map_err(|e| {
        api_err(
            StatusCode::INTERNAL_SERVER_ERROR,
            "serialize_failed",
            format!("Failed to serialize users: {}", e),
        )
    })?;

    tokio::fs::write(format!("{}/season.json", season_dir), &season_bytes)
        .await
        .map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "write_failed",
                format!("Failed to write season.json: {}", e),
            )
        })?;

    tokio::fs::write(format!("{}/users.json", season_dir), &users_bytes)
        .await
        .map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "write_failed",
                format!("Failed to write users.json: {}", e),
            )
        })?;

    Ok(())
}

async fn deactivate_other_seasons(
    data_path: &str,
    existing_seasons: HashMap<String, SeasonConfig>,
    season_id: &str,
) -> ApiResult<()> {
    for (existing_id, mut cfg) in existing_seasons {
        if existing_id == season_id || !cfg.active {
            continue;
        }
        cfg.active = false;
        let path = format!("{}/seasons/{}/season.json", data_path, existing_id);
        let content = serde_json::to_vec_pretty(&cfg).map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "serialize_failed",
                format!("Failed to serialize deactivated season '{}': {}", existing_id, e),
            )
        })?;
        tokio::fs::write(&path, content).await.map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "write_failed",
                format!(
                    "Failed to write deactivated season config '{}': {}",
                    existing_id, e
                ),
            )
        })?;
    }
    Ok(())
}

async fn reload_config(config_manager: &SharedConfigManager) -> ApiResult<()> {
    config_manager
        .write()
        .await
        .load_all()
        .await
        .map_err(|e| {
            api_err(
                StatusCode::INTERNAL_SERVER_ERROR,
                "reload_failed",
                format!("Failed to reload configuration: {}", e),
            )
        })
}

async fn ensure_discord_role(
    http: &serenity::Http,
    guild_id: GuildId,
    role_name: &str,
    season_id: &str,
) {
    match guild_id.roles(http).await {
        Ok(roles) if roles.values().any(|r| r.name == role_name) => {
            info!("Discord role '{}' already exists", role_name);
        }
        Ok(_) => {
            match guild_id
                .create_role(
                    http,
                    serenity::EditRole::new().name(role_name).mentionable(true),
                )
                .await
            {
                Ok(role) => info!(
                    "Created Discord role '{}' (ID: {}) for season '{}'",
                    role_name, role.id, season_id
                ),
                Err(e) => warn!(
                    "Failed to create Discord role '{}' for season '{}': {}",
                    role_name, season_id, e
                ),
            }
        }
        Err(e) => warn!(
            "Failed to fetch Discord roles while preparing season '{}': {}",
            season_id, e
        ),
    }
}

fn build_sync_result(
    result: crate::error::Result<crate::managers::channel_manager::UpdateSummary>,
) -> (String, Option<ApiSyncSummary>, Option<String>) {
    match result {
        Ok(summary) => {
            let sync = ApiSyncSummary::from(summary);
            let status = if sync.warnings.is_empty() && sync.missing_roles.is_empty() {
                "ok"
            } else {
                "partial_success"
            };
            (status.to_string(), Some(sync), None)
        }
        Err(e) => (
            "partial_success".to_string(),
            None,
            Some(format!("Season upserted, but sync failed: {}", e)),
        ),
    }
}

// ── Core upsert logic ─────────────────────────────────────────────────────────

async fn api_upsert_season(
    state: AdminState,
    season_id: String,
    season_name: String,
    role_name: String,
    users: Vec<SeasonUser>,
) -> ApiResult<(StatusCode, Json<BootstrapSeasonResponse>)> {
    let season_id = season_id.trim().to_string();
    validate_season_folder_name(&season_id)?;

    let name = season_name.trim().to_string();
    if name.is_empty() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_name",
            "Season name is required".to_string(),
        ));
    }
    let member_role = role_name.trim().to_string();
    if member_role.is_empty() {
        return Err(api_err(
            StatusCode::BAD_REQUEST,
            "invalid_member_role",
            "role_name is required".to_string(),
        ));
    }

    let source = resolve_season_source(&state.config_manager, &season_id).await?;

    let remapped_channels = remap_member_role_in_channels(
        &source.source_channels,
        &source.source_member_role,
        &member_role,
    );

    let new_config = SeasonConfig {
        name: name.clone(),
        active: if source.season_existed {
            source.keep_active_flag
        } else {
            true
        },
        member_role: Some(member_role.clone()),
        channels: remapped_channels.clone(),
    };

    write_season_files(&source.data_path, &season_id, &new_config, &users).await?;

    if !source.season_existed {
        deactivate_other_seasons(&source.data_path, source.existing_seasons, &season_id).await?;
    }

    reload_config(&state.config_manager).await?;

    let http = state.serenity_http.as_ref();
    let guild_id = state.guild_id;

    ensure_discord_role(http, guild_id, &member_role, &season_id).await;

    let channel_manager = state.channel_manager.read().await;
    let sync_result = channel_manager
        .sync_season_channels(http, guild_id, &name, &remapped_channels)
        .await;
    drop(channel_manager);

    let (status, sync, sync_error) = build_sync_result(sync_result);

    info!(
        "Season '{}' upserted from '{}' via REST API",
        season_id, source.source_season_id
    );

    Ok((
        StatusCode::OK,
        Json(BootstrapSeasonResponse {
            status,
            season_id: season_id.clone(),
            source_season_id: source.source_season_id,
            activated_season_id: season_id,
            deactivated_seasons: source.deactivated_season_ids,
            sync,
            sync_error,
        }),
    ))
}

// ── Route handler ─────────────────────────────────────────────────────────────

/// POST /admin/api/v1/initialize_users
///
/// Creates or updates a season, its users list, the Discord member role, and
/// syncs channel permissions — all in one call.
async fn api_initialize_users(
    headers: HeaderMap,
    State(state): State<AdminState>,
    Json(payload): Json<InitializeUsersRequest>,
) -> ApiResult<(StatusCode, Json<BootstrapSeasonResponse>)> {
    require_api_token(&headers)?;
    let season_name = payload.season_name;
    api_upsert_season(
        state,
        season_name.clone(),
        season_name,
        payload.role_name,
        payload.users,
    )
    .await
}

// ── Router ────────────────────────────────────────────────────────────────────

pub fn api_router(state: AdminState) -> Router {
    Router::new()
        .route("/api/v1/initialize_users", post(api_initialize_users))
        .with_state(state)
}
