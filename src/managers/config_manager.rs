use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::{
    ConfigChange, ConfigChangeType, ConfigDiff, GlobalPermissionsConfig, GlobalRolesConfig,
    Season, SeasonConfig, SeasonUser, SpecialMembersConfig, StagedConfig, load_users_from_file,
};
use crate::error::{BotError, Result};

/// Manages all configuration loading, staging, and committing
pub struct ConfigManager {
    /// Currently active seasons (season_id -> Season)
    seasons: HashMap<String, Season>,

    /// Special members config (from global/assignments.json)
    special_members: Option<SpecialMembersConfig>,

    /// Global roles config (from global/roles.json)
    global_roles: Option<GlobalRolesConfig>,

    /// Global permissions config (from global/permissions.json)
    global_permissions: Option<GlobalPermissionsConfig>,

    /// Staged configuration waiting to be committed
    staged: StagedConfig,

    /// Base path for data files
    data_path: String,
}

impl ConfigManager {
    /// Create a new config manager
    pub fn new(data_path: &str) -> Self {
        Self {
            seasons: HashMap::new(),
            special_members: None,
            global_roles: None,
            global_permissions: None,
            staged: StagedConfig::new(),
            data_path: data_path.to_string(),
        }
    }

    /// Load all configurations from the data directory
    ///
    /// Structure:
    /// data/
    /// ├── global/
    /// │   ├── roles.json        # Role definitions
    /// │   ├── assignments.json  # Who has which special role
    /// │   └── permissions.json  # Permission definitions
    /// └── seasons/
    ///     └── {season_id}/
    ///         ├── season.json   # Season config (name, active, channels)
    ///         └── users.json    # Users array (externally generated)
    pub async fn load_all(&mut self) -> Result<()> {
        self.load_global_config().await;
        self.load_seasons().await;

        info!(
            "Config loaded: {} seasons, special_members={}, global_roles={}, global_permissions={}",
            self.seasons.len(),
            self.special_members.is_some(),
            self.global_roles.is_some(),
            self.global_permissions.is_some(),
        );

        Ok(())
    }

    /// Load global config from data/global/ directory
    async fn load_global_config(&mut self) {
        let global_dir = format!("{}/global", self.data_path);

        // Load roles.json
        let roles_path = format!("{}/roles.json", global_dir);
        if std::path::Path::new(&roles_path).exists() {
            match GlobalRolesConfig::load_from_file(&roles_path) {
                Ok(config) => {
                    info!("Loaded {} roles from global/roles.json", config.roles.len());
                    self.global_roles = Some(config);
                }
                Err(e) => warn!("Failed to load global/roles.json: {}", e),
            }
        }

        // Load assignments.json (special members)
        let assignments_path = format!("{}/assignments.json", global_dir);
        if std::path::Path::new(&assignments_path).exists() {
            match SpecialMembersConfig::load_from_file(&assignments_path) {
                Ok(config) => {
                    info!(
                        "Loaded {} role assignments from global/assignments.json",
                        config.discord_usernames_by_role.len()
                    );
                    self.special_members = Some(config);
                }
                Err(e) => warn!("Failed to load global/assignments.json: {}", e),
            }
        }

        // Load permissions.json
        let permissions_path = format!("{}/permissions.json", global_dir);
        if std::path::Path::new(&permissions_path).exists() {
            match GlobalPermissionsConfig::load_from_file(&permissions_path) {
                Ok(config) => {
                    info!(
                        "Loaded {} permission definitions from global/permissions.json",
                        config.definitions.len()
                    );
                    self.global_permissions = Some(config);
                }
                Err(e) => warn!("Failed to load global/permissions.json: {}", e),
            }
        }
    }

    /// Load seasons from data/seasons/ directories
    async fn load_seasons(&mut self) {
        let seasons_path = format!("{}/seasons", self.data_path);

        let Ok(mut entries) = tokio::fs::read_dir(&seasons_path).await else {
            return;
        };

        while let Ok(Some(entry)) = entries.next_entry().await {
            let path = entry.path();

            // Only process directories
            if !path.is_dir() {
                continue;
            }

            let season_id = path
                .file_name()
                .and_then(|n| n.to_str())
                .unwrap_or("unknown")
                .to_string();

            // Skip template directory
            if season_id == "template" {
                info!("Skipping template directory");
                continue;
            }

            // Load season.json
            let season_json_path = path.join("season.json");
            let config = if season_json_path.exists() {
                match SeasonConfig::load_from_file(season_json_path.to_str().unwrap_or_default()) {
                    Ok(config) => {
                        info!("Loaded season config for '{}'", season_id);
                        config
                    }
                    Err(e) => {
                        warn!("Failed to load season.json for {}: {}", season_id, e);
                        continue;
                    }
                }
            } else {
                // Create default config if season.json doesn't exist
                SeasonConfig {
                    name: season_id.clone(),
                    active: true,
                    member_role: None, // Will fallback to "Medlem{season_id}"
                    channels: vec![],
                }
            };

            // Load users.json
            let users_path = path.join("users.json");
            let users = if users_path.exists() {
                match load_users_from_file(users_path.to_str().unwrap_or_default()) {
                    Ok(users) => {
                        info!("Loaded {} users for season '{}'", users.len(), season_id);
                        users
                    }
                    Err(e) => {
                        warn!("Failed to load users.json for {}: {}", season_id, e);
                        vec![]
                    }
                }
            } else {
                vec![]
            };

            // Create Season combining config + users
            let season = Season::new(season_id.clone(), config, users);
            self.seasons.insert(season_id, season);
        }
    }

    // ========== Query Operations ==========

    /// Get a season by ID
    pub fn get_season(&self, season_id: &str) -> Option<&Season> {
        self.seasons.get(season_id)
    }

    /// Get all seasons
    pub fn get_all_seasons(&self) -> Vec<&Season> {
        self.seasons.values().collect()
    }

    /// Get seasons as a HashMap reference
    pub fn get_seasons(&self) -> &HashMap<String, Season> {
        &self.seasons
    }

    /// Find a user in any active season by their verification ID
    pub fn find_user_by_verification_id(
        &self,
        verification_id: &str,
    ) -> Option<(&Season, &SeasonUser)> {
        for season in self.seasons.values() {
            if season.is_active() {
                if let Some(user) = season.find_user_by_id(verification_id) {
                    return Some((season, user));
                }
            }
        }
        None
    }

    /// Get special roles for a user by their Discord username
    pub fn get_special_roles_for_user(&self, discord_username: &str) -> Vec<String> {
        let roles = self.special_members
            .as_ref()
            .map(|sm| sm.get_roles_for_user(discord_username))
            .unwrap_or_default();

        if self.special_members.is_none() {
            tracing::debug!("get_special_roles_for_user: No special_members config loaded (assignments.json missing?)");
        } else if roles.is_empty() {
            tracing::debug!(
                "get_special_roles_for_user: No special roles found for username '{}'",
                discord_username
            );
        } else {
            tracing::info!(
                "get_special_roles_for_user: Found {} special roles for '{}': {:?}",
                roles.len(),
                discord_username,
                roles
            );
        }

        roles
    }

    /// Get the global roles config
    pub fn get_global_roles(&self) -> Option<&GlobalRolesConfig> {
        self.global_roles.as_ref()
    }

    /// Get the global permissions config
    pub fn get_global_permissions(&self) -> Option<&GlobalPermissionsConfig> {
        self.global_permissions.as_ref()
    }

    /// Get the special members (assignments) config
    pub fn get_special_members(&self) -> Option<&SpecialMembersConfig> {
        self.special_members.as_ref()
    }

    /// Check if a user is a maintainer (can access admin panel)
    pub fn is_maintainer(&self, discord_username: &str) -> bool {
        self.special_members
            .as_ref()
            .map(|sm| sm.is_maintainer(discord_username))
            .unwrap_or(false)
    }

    /// Get the data path
    pub fn get_data_path(&self) -> &str {
        &self.data_path
    }

    /// Get the default member role name
    pub fn get_default_member_role_name(&self) -> &str {
        if let Some(roles) = &self.global_roles {
            if let Some(role) = roles.get_default_member_role() {
                return &role.name;
            }
        }
        "Medlem"
    }

    /// Get all roles
    pub fn get_all_roles(&self) -> Vec<&crate::config::RoleDefinition> {
        if let Some(roles) = &self.global_roles {
            roles.roles.iter().collect()
        } else {
            vec![]
        }
    }

    // ========== Staging Operations ==========

    /// Check if there's anything staged
    pub fn has_staged(&self) -> bool {
        !self.staged.is_empty()
    }

    /// Get summary of staged config
    pub fn get_staged_summary(&self) -> String {
        self.staged.get_summary()
    }

    /// Generate diff between staged and current config
    pub fn get_staged_diff(&self) -> ConfigDiff {
        let mut diff = ConfigDiff::new();

        // Compare seasons
        for (season_id, staged) in &self.staged.seasons {
            if let Some(current_season) = self.seasons.get(season_id) {
                let added = staged.users.len() as i64 - current_season.user_count() as i64;
                if added != 0 {
                    diff.add_modification(ConfigChange::new(
                        ConfigChangeType::Modify,
                        "season",
                        season_id,
                        &format!("{} users ({:+} change)", staged.users.len(), added),
                    ));
                }
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "season",
                    season_id,
                    &format!("{} users", staged.users.len()),
                ));
            }
        }

        // Compare special members
        if let Some(staged_sm) = &self.staged.special_members {
            if self.special_members.is_some() {
                diff.add_modification(ConfigChange::new(
                    ConfigChangeType::Modify,
                    "special_members",
                    "assignments.json",
                    &format!("{} roles defined", staged_sm.discord_usernames_by_role.len()),
                ));
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "special_members",
                    "assignments.json",
                    &format!("{} roles defined", staged_sm.discord_usernames_by_role.len()),
                ));
            }
        }

        diff
    }

    /// Stage users for a season from bytes with explicit season ID
    pub fn stage_season_from_bytes_with_id(
        &mut self,
        data: &[u8],
        _filename: &str,
        season_id: Option<&str>,
        staged_by: Option<String>,
    ) -> Result<()> {
        let sid = season_id.ok_or_else(|| BotError::ConfigValidation {
            message: "Season ID is required".to_string(),
        })?;

        let users: Vec<SeasonUser> = serde_json::from_slice(data).map_err(|e| {
            BotError::ConfigValidation {
                message: format!("Failed to parse users JSON: {}", e),
            }
        })?;

        self.staged.stage_season_users(sid.to_string(), users, staged_by);
        Ok(())
    }

    /// Stage users for a season from bytes (derives season_id from filename)
    pub fn stage_season_from_bytes(
        &mut self,
        data: &[u8],
        filename: &str,
        staged_by: Option<String>,
    ) -> Result<String> {
        let season_id = filename.trim_end_matches(".json").to_uppercase();

        let users: Vec<SeasonUser> = serde_json::from_slice(data).map_err(|e| {
            BotError::ConfigValidation {
                message: format!("Failed to parse users JSON: {}", e),
            }
        })?;

        let user_count = users.len();
        self.staged.stage_season_users(season_id.clone(), users, staged_by);
        Ok(format!("Staged {} users for season {}", user_count, season_id))
    }

    /// Stage special members config from bytes
    pub fn stage_special_members_from_bytes(
        &mut self,
        data: &[u8],
        staged_by: Option<String>,
    ) -> Result<String> {
        let config: SpecialMembersConfig = serde_json::from_slice(data).map_err(|e| {
            BotError::ConfigValidation {
                message: format!("Failed to parse assignments JSON: {}", e),
            }
        })?;

        let role_count = config.discord_usernames_by_role.len();
        self.staged.stage_special_members(config, staged_by);
        Ok(format!("Staged assignments with {} roles", role_count))
    }

    /// Stage a raw config from bytes (for Discord command uploads)
    pub fn stage_raw_config(&mut self, config_type: &str, season_id: Option<&str>, data: Vec<u8>) {
        match config_type {
            "users" => {
                if let Ok(users) = serde_json::from_slice::<Vec<SeasonUser>>(&data) {
                    if let Some(sid) = season_id {
                        self.staged.stage_season_users(sid.to_string(), users, None);
                        info!("Staged users for season {}", sid);
                    }
                } else {
                    warn!("Failed to parse users JSON");
                }
            }
            "roles" | "assignments" => {
                if let Ok(config) = serde_json::from_slice::<SpecialMembersConfig>(&data) {
                    self.staged.stage_special_members(config, None);
                    info!("Staged assignments config");
                } else {
                    warn!("Failed to parse assignments JSON");
                }
            }
            _ => {
                warn!("Unknown config type: {}", config_type);
            }
        }
    }

    /// Commit staged configuration
    pub async fn commit_staged(&mut self) -> Result<Vec<ConfigChange>> {
        if self.staged.is_empty() {
            return Err(BotError::NoStagedConfig);
        }

        let mut changes = Vec::new();

        // Commit seasons
        for (season_id, staged) in self.staged.seasons.drain() {
            let dir_path = format!("{}/seasons/{}", self.data_path, season_id);
            let users_path = format!("{}/users.json", dir_path);

            tokio::fs::create_dir_all(&dir_path).await.ok();

            // Save users as array
            let content = serde_json::to_string_pretty(&staged.users)?;
            tokio::fs::write(&users_path, content)
                .await
                .map_err(|e| BotError::StateSave {
                    path: users_path.clone(),
                    source: e,
                })?;

            changes.push(ConfigChange::new(
                ConfigChangeType::Add,
                "season",
                &season_id,
                &format!("Saved {} users", staged.users.len()),
            ));

            // Update in-memory season
            if let Some(season) = self.seasons.get_mut(&season_id) {
                season.users = staged.users;
            } else {
                let season = Season::new(
                    season_id.clone(),
                    SeasonConfig {
                        name: season_id.clone(),
                        active: true,
                        member_role: None, // Will fallback to "Medlem{season_id}"
                        channels: vec![],
                    },
                    staged.users,
                );
                self.seasons.insert(season_id, season);
            }
        }

        // Commit special members to global/assignments.json
        if let Some(config) = self.staged.special_members.take() {
            let dir_path = format!("{}/global", self.data_path);
            let path = format!("{}/assignments.json", dir_path);

            tokio::fs::create_dir_all(&dir_path).await.ok();

            let content = serde_json::to_string_pretty(&config)?;
            tokio::fs::write(&path, content)
                .await
                .map_err(|e| BotError::StateSave {
                    path: path.clone(),
                    source: e,
                })?;

            changes.push(ConfigChange::new(
                ConfigChangeType::Add,
                "special_members",
                "assignments.json",
                &format!("{} roles", config.discord_usernames_by_role.len()),
            ));

            self.special_members = Some(config);
        }

        self.staged.clear();
        Ok(changes)
    }

    /// Clear staged configuration
    pub fn clear_staged(&mut self) {
        self.staged.clear();
    }

    /// Export a config as JSON bytes
    pub fn export_config(
        &self,
        config_type: &str,
        name: Option<&str>,
    ) -> Result<(String, Vec<u8>)> {
        match config_type {
            "season" | "users" => {
                let name = name.ok_or_else(|| BotError::ConfigNotFound {
                    config_type: "season".to_string(),
                    name: "unspecified".to_string(),
                })?;
                let season = self
                    .seasons
                    .get(name)
                    .ok_or_else(|| BotError::ConfigNotFound {
                        config_type: "season".to_string(),
                        name: name.to_string(),
                    })?;
                let bytes = serde_json::to_vec_pretty(&season.users)?;
                Ok((format!("seasons/{}/users.json", name), bytes))
            }
            "special_members" | "roles" | "assignments" => {
                let config =
                    self.special_members
                        .as_ref()
                        .ok_or_else(|| BotError::ConfigNotFound {
                            config_type: "assignments".to_string(),
                            name: "".to_string(),
                        })?;
                let bytes = serde_json::to_vec_pretty(config)?;
                Ok(("global/assignments.json".to_string(), bytes))
            }
            "global" => {
                let config = self
                    .global_roles
                    .as_ref()
                    .ok_or_else(|| BotError::ConfigNotFound {
                        config_type: "global".to_string(),
                        name: "".to_string(),
                    })?;
                let bytes = serde_json::to_vec_pretty(config)?;
                Ok(("global/roles.json".to_string(), bytes))
            }
            _ => Err(BotError::ConfigNotFound {
                config_type: config_type.to_string(),
                name: name.unwrap_or("").to_string(),
            }),
        }
    }

    /// Get list of all loaded config files
    pub fn get_all_config_files(&self) -> Vec<(String, String)> {
        let mut files = Vec::new();

        // Global configs
        if self.global_roles.is_some() {
            files.push(("global/roles.json".to_string(), "Role definitions".to_string()));
        }
        if self.special_members.is_some() {
            files.push(("global/assignments.json".to_string(), "Role assignments".to_string()));
        }
        if self.global_permissions.is_some() {
            files.push((
                "global/permissions.json".to_string(),
                "Permission definitions".to_string(),
            ));
        }

        // Seasons
        for (season_id, season) in &self.seasons {
            files.push((
                format!("seasons/{}/season.json", season_id),
                format!("Season: {}", season.name()),
            ));
            files.push((
                format!("seasons/{}/users.json", season_id),
                format!("{} users", season.user_count()),
            ));
        }

        files
    }

    /// Check which config files exist for a season
    pub fn get_season_file_status(&self, season_id: &str) -> (Vec<String>, Vec<String>) {
        let mut existing = Vec::new();
        let mut missing = Vec::new();

        if self.seasons.contains_key(season_id) {
            existing.push(format!("seasons/{}/season.json", season_id));
            existing.push(format!("seasons/{}/users.json", season_id));
        } else {
            missing.push(format!("seasons/{}/season.json", season_id));
            missing.push(format!("seasons/{}/users.json", season_id));
        }

        (existing, missing)
    }
}

/// Shared config manager type
pub type SharedConfigManager = Arc<tokio::sync::RwLock<ConfigManager>>;

pub fn create_shared_config_manager(data_path: &str) -> SharedConfigManager {
    Arc::new(tokio::sync::RwLock::new(ConfigManager::new(data_path)))
}
