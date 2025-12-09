use std::collections::HashMap;
use std::sync::Arc;
use tracing::{info, warn};

use crate::config::{
    CategoryStructureConfig, ConfigChange, ConfigChangeType, ConfigDiff, GlobalStructureConfig,
    SeasonConfig, SeasonUser, SpecialMembersConfig, StagedConfig,
};
use crate::error::{BotError, Result};

/// Manages all configuration loading, staging, and committing
pub struct ConfigManager {
    /// Currently active season configs
    seasons: HashMap<String, SeasonConfig>,

    /// Currently active special members config
    special_members: Option<SpecialMembersConfig>,

    /// Currently active global structure config
    global_structure: Option<GlobalStructureConfig>,

    /// Currently active category structure configs
    category_structures: HashMap<String, CategoryStructureConfig>,

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
            global_structure: None,
            category_structures: HashMap::new(),
            staged: StagedConfig::new(),
            data_path: data_path.to_string(),
        }
    }

    /// Load all configurations from the data directory
    ///
    /// New structure:
    /// data/
    /// ├── seasons/
    /// │   └── 2025E/
    /// │       ├── users.json        # Users for this season
    /// │       ├── category.json     # Category/channel structure
    /// │       └── special_roles.json # Special roles for this season
    /// ├── global.json               # Global structure
    /// └── database.json             # Bot-managed verified users (loaded separately)
    pub async fn load_all(&mut self) -> Result<()> {
        // Load global structure from data/global.json (or fallback to global_structure.json)
        let global_path = format!("{}/global.json", self.data_path);
        let legacy_global_path = format!("{}/global_structure.json", self.data_path);

        if std::path::Path::new(&global_path).exists() {
            match GlobalStructureConfig::load_from_file(&global_path) {
                Ok(config) => {
                    info!("Loaded global config from global.json");
                    self.global_structure = Some(config);
                }
                Err(e) => {
                    warn!("Failed to load global.json: {}", e);
                }
            }
        } else if std::path::Path::new(&legacy_global_path).exists() {
            match GlobalStructureConfig::load_from_file(&legacy_global_path) {
                Ok(config) => {
                    info!("Loaded global config from legacy global_structure.json");
                    self.global_structure = Some(config);
                }
                Err(e) => {
                    warn!("Failed to load global_structure.json: {}", e);
                }
            }
        } else {
            // Use default global structure
            info!("Using default global structure");
            self.global_structure = Some(GlobalStructureConfig::default());
        }

        // Load seasons from data/seasons/{season_id}/ directories
        let seasons_path = format!("{}/seasons", self.data_path);
        if let Ok(mut entries) = tokio::fs::read_dir(&seasons_path).await {
            while let Ok(Some(entry)) = entries.next_entry().await {
                let path = entry.path();

                // Check if it's a directory (new structure)
                if path.is_dir() {
                    let season_id = path
                        .file_name()
                        .and_then(|n| n.to_str())
                        .unwrap_or("unknown")
                        .to_string();

                    // Load users.json from the season directory
                    let users_path = path.join("users.json");
                    if users_path.exists() {
                        match SeasonConfig::load_from_file(users_path.to_str().unwrap_or_default())
                        {
                            Ok(mut config) => {
                                config.season_id = season_id.clone();
                                if config.name.is_empty() || config.name == season_id {
                                    config.name = season_id.clone();
                                }
                                info!(
                                    "Loaded season '{}' with {} users",
                                    season_id,
                                    config.users.len()
                                );
                                self.seasons.insert(season_id.clone(), config);
                            }
                            Err(e) => {
                                warn!("Failed to load users.json for season {}: {}", season_id, e);
                            }
                        }
                    }

                    // Load category.json from the season directory
                    let category_path = path.join("category.json");
                    if category_path.exists() {
                        match CategoryStructureConfig::load_from_file(
                            category_path.to_str().unwrap_or_default(),
                        ) {
                            Ok(mut config) => {
                                config.season_id = season_id.clone();
                                info!("Loaded category structure for season '{}'", season_id);
                                self.category_structures.insert(season_id.clone(), config);
                            }
                            Err(e) => {
                                warn!(
                                    "Failed to load category.json for season {}: {}",
                                    season_id, e
                                );
                            }
                        }
                    }

                    // Load roles.json from the season directory
                    let roles_path = path.join("roles.json");
                    if roles_path.exists() {
                        match SpecialMembersConfig::load_from_file(
                            roles_path.to_str().unwrap_or_default(),
                        ) {
                            Ok(config) => {
                                info!(
                                    "Loaded roles for season '{}' with {} roles",
                                    season_id,
                                    config.roles.len()
                                );
                                // For now, merge into the global special_members
                                // TODO: Consider per-season special roles
                                if self.special_members.is_none() {
                                    self.special_members = Some(config);
                                } else {
                                    // Merge roles
                                    let existing = self.special_members.as_mut().unwrap();
                                    for (role_name, user_ids) in config.roles {
                                        existing
                                            .roles
                                            .entry(role_name)
                                            .or_insert_with(Vec::new)
                                            .extend(user_ids);
                                    }
                                }
                            }
                            Err(e) => {
                                warn!("Failed to load roles.json for season {}: {}", season_id, e);
                            }
                        }
                    }
                } else if path.extension().map(|e| e == "json").unwrap_or(false) {
                    // Legacy: flat JSON files in seasons/ directory
                    match SeasonConfig::load_from_file(path.to_str().unwrap_or_default()) {
                        Ok(config) => {
                            info!("Loaded legacy season config: {}", config.season_id);
                            self.seasons.insert(config.season_id.clone(), config);
                        }
                        Err(e) => {
                            warn!("Failed to load season config from {:?}: {}", path, e);
                        }
                    }
                }
            }
        }

        // Fallback: load legacy data/users.json if no seasons found
        if self.seasons.is_empty() {
            let legacy_path = format!("{}/users.json", self.data_path);
            if std::path::Path::new(&legacy_path).exists() {
                match SeasonConfig::load_from_file(&legacy_path) {
                    Ok(mut config) => {
                        config.season_id = "legacy".to_string();
                        config.name = "Legacy Users".to_string();
                        info!(
                            "Loaded legacy users.json as season 'legacy' with {} users",
                            config.users.len()
                        );
                        self.seasons.insert("legacy".to_string(), config);
                    }
                    Err(e) => {
                        warn!("Failed to load legacy users.json: {}", e);
                    }
                }
            }
        }

        // Fallback: load legacy data/special_members.json if no special roles loaded
        if self.special_members.is_none() {
            let legacy_special_path = format!("{}/special_members.json", self.data_path);
            if std::path::Path::new(&legacy_special_path).exists() {
                match SpecialMembersConfig::load_from_file(&legacy_special_path) {
                    Ok(config) => {
                        info!("Loaded legacy special_members.json");
                        self.special_members = Some(config);
                    }
                    Err(e) => {
                        warn!("Failed to load special_members.json: {}", e);
                    }
                }
            }
        }

        info!(
            "Config loaded: {} seasons, special_members={}, global_structure={}, {} category_structures",
            self.seasons.len(),
            self.special_members.is_some(),
            self.global_structure.is_some(),
            self.category_structures.len()
        );

        Ok(())
    }

    // ========== Staging Operations ==========

    /// Stage a season config from raw JSON bytes
    pub fn stage_season_from_bytes(
        &mut self,
        data: &[u8],
        filename: &str,
        staged_by: Option<String>,
    ) -> Result<String> {
        self.stage_season_from_bytes_with_id(data, filename, None, staged_by)
    }

    /// Stage a season config from raw JSON bytes with an explicit season ID override
    pub fn stage_season_from_bytes_with_id(
        &mut self,
        data: &[u8],
        filename: &str,
        season_id_override: Option<&str>,
        staged_by: Option<String>,
    ) -> Result<String> {
        // Try to parse as SeasonConfig first
        let mut config: SeasonConfig =
            if let Ok(config) = serde_json::from_slice::<SeasonConfig>(data) {
                config
            } else {
                // Try legacy format (array of users)
                let users: Vec<SeasonUser> =
                    serde_json::from_slice(data).map_err(|e| BotError::ConfigParse {
                        path: filename.to_string(),
                        source: e,
                    })?;

                // Extract season_id from filename (fallback)
                let season_id = std::path::Path::new(filename)
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .unwrap_or("unknown")
                    .to_string();

                SeasonConfig {
                    season_id: season_id.clone(),
                    name: season_id,
                    active: true,
                    users,
                }
            };

        // Apply season_id override if provided
        if let Some(override_id) = season_id_override {
            config.season_id = override_id.to_string();
            if config.name.is_empty() || config.name == "unknown" {
                config.name = override_id.to_string();
            }
        }

        let season_id = config.season_id.clone();
        let user_count = config.users.len();
        self.staged.stage_season(config, staged_by);

        Ok(format!(
            "Staged season '{}' with {} users",
            season_id, user_count
        ))
    }

    /// Stage special members config from raw JSON bytes
    pub fn stage_special_members_from_bytes(
        &mut self,
        data: &[u8],
        staged_by: Option<String>,
    ) -> Result<String> {
        let config: SpecialMembersConfig = serde_json::from_slice(data)?;
        let role_count = config.roles.len();
        self.staged.stage_special_members(config, staged_by);
        Ok(format!("Staged special members with {} roles", role_count))
    }

    /// Stage global structure config from raw JSON bytes
    pub fn stage_global_structure_from_bytes(
        &mut self,
        data: &[u8],
        staged_by: Option<String>,
    ) -> Result<String> {
        let config: GlobalStructureConfig = serde_json::from_slice(data)?;
        let role_count = config.default_roles.len();
        let channel_count = config.default_channels.len();
        self.staged.stage_global_structure(config, staged_by);
        Ok(format!(
            "Staged global structure with {} roles, {} channels",
            role_count, channel_count
        ))
    }

    /// Stage category structure config from raw JSON bytes
    pub fn stage_category_structure_from_bytes(
        &mut self,
        data: &[u8],
        staged_by: Option<String>,
    ) -> Result<String> {
        let config: CategoryStructureConfig = serde_json::from_slice(data)?;
        let season_id = config.season_id.clone();
        self.staged.stage_category_structure(config, staged_by);
        Ok(format!("Staged category structure for '{}'", season_id))
    }

    /// Stage a raw config from bytes with a generic interface
    /// config_type: "global", "users", "category", "roles"
    /// season_id: Only used for users, category
    pub fn stage_raw_config(&mut self, config_type: &str, season_id: Option<&str>, data: Vec<u8>) {
        match config_type {
            "global" => {
                if let Ok(config) = serde_json::from_slice::<GlobalStructureConfig>(&data) {
                    self.staged.stage_global_structure(config, None);
                    info!("Staged global config");
                } else {
                    warn!("Failed to parse global config JSON");
                }
            }
            "users" => {
                if let Ok(mut config) = serde_json::from_slice::<SeasonConfig>(&data) {
                    if let Some(sid) = season_id {
                        config.season_id = sid.to_string();
                        if config.name.is_empty() {
                            config.name = sid.to_string();
                        }
                    }
                    let sid = config.season_id.clone();
                    self.staged.stage_season(config, None);
                    info!("Staged users config for season {}", sid);
                } else {
                    warn!("Failed to parse users config JSON");
                }
            }
            "category" => {
                if let Ok(mut config) = serde_json::from_slice::<CategoryStructureConfig>(&data) {
                    if let Some(sid) = season_id {
                        config.season_id = sid.to_string();
                    }
                    let sid = config.season_id.clone();
                    self.staged.stage_category_structure(config, None);
                    info!("Staged category config for season {}", sid);
                } else {
                    warn!("Failed to parse category config JSON");
                }
            }
            "roles" => {
                if let Ok(config) = serde_json::from_slice::<SpecialMembersConfig>(&data) {
                    self.staged.stage_special_members(config, None);
                    info!("Staged roles config");
                } else {
                    warn!("Failed to parse roles config JSON");
                }
            }
            _ => {
                warn!("Unknown config type: {}", config_type);
            }
        }
    }

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
        for (season_id, staged_season) in &self.staged.seasons {
            if let Some(current_season) = self.seasons.get(season_id) {
                // Check for user changes
                let added = staged_season.users.len() as i64 - current_season.users.len() as i64;
                if added != 0 {
                    diff.add_modification(ConfigChange::new(
                        ConfigChangeType::Modify,
                        "season",
                        season_id,
                        &format!("{} users ({:+} change)", staged_season.users.len(), added),
                    ));
                }
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "season",
                    season_id,
                    &format!("{} users", staged_season.users.len()),
                ));
            }
        }

        // Compare special members
        if let Some(staged_sm) = &self.staged.special_members {
            if self.special_members.is_some() {
                diff.add_modification(ConfigChange::new(
                    ConfigChangeType::Modify,
                    "special_members",
                    "special_members.json",
                    &format!("{} roles defined", staged_sm.roles.len()),
                ));
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "special_members",
                    "special_members.json",
                    &format!("{} roles defined", staged_sm.roles.len()),
                ));
            }
        }

        // Compare global structure
        if let Some(staged_gs) = &self.staged.global_structure {
            if self.global_structure.is_some() {
                diff.add_modification(ConfigChange::new(
                    ConfigChangeType::Modify,
                    "global_structure",
                    "global_structure.json",
                    &format!(
                        "{} roles, {} channels",
                        staged_gs.default_roles.len(),
                        staged_gs.default_channels.len()
                    ),
                ));
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "global_structure",
                    "global_structure.json",
                    &format!(
                        "{} roles, {} channels",
                        staged_gs.default_roles.len(),
                        staged_gs.default_channels.len()
                    ),
                ));
            }
        }

        // Compare category structures
        for (season_id, _) in &self.staged.category_structures {
            if self.category_structures.contains_key(season_id) {
                diff.add_modification(ConfigChange::new(
                    ConfigChangeType::Modify,
                    "category_structure",
                    season_id,
                    "Updated category structure",
                ));
            } else {
                diff.add_addition(ConfigChange::new(
                    ConfigChangeType::Add,
                    "category_structure",
                    season_id,
                    "New category structure",
                ));
            }
        }

        diff
    }

    /// Commit staged configuration (apply and save to disk)
    pub async fn commit_staged(&mut self) -> Result<Vec<ConfigChange>> {
        if self.staged.is_empty() {
            return Err(BotError::NoStagedConfig);
        }

        let mut changes = Vec::new();

        // Commit seasons - save to seasons/{season_id}/users.json
        for (season_id, config) in self.staged.seasons.drain() {
            let dir_path = format!("{}/seasons/{}", self.data_path, season_id);
            let path = format!("{}/users.json", dir_path);

            // Ensure season directory exists
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
                "season",
                &season_id,
                &format!(
                    "Saved {} users to seasons/{}/users.json",
                    config.users.len(),
                    season_id
                ),
            ));

            self.seasons.insert(season_id, config);
        }

        // Commit special members
        if let Some(config) = self.staged.special_members.take() {
            let path = format!("{}/special_members.json", self.data_path);
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
                "special_members.json",
                &format!("{} roles", config.roles.len()),
            ));

            self.special_members = Some(config);
        }

        // Commit global structure
        if let Some(config) = self.staged.global_structure.take() {
            let path = format!("{}/global_structure.json", self.data_path);
            let content = serde_json::to_string_pretty(&config)?;
            tokio::fs::write(&path, content)
                .await
                .map_err(|e| BotError::StateSave {
                    path: path.clone(),
                    source: e,
                })?;

            changes.push(ConfigChange::new(
                ConfigChangeType::Add,
                "global_structure",
                "global_structure.json",
                "Saved global structure",
            ));

            self.global_structure = Some(config);
        }

        // Commit category structures
        for (season_id, config) in self.staged.category_structures.drain() {
            let path = format!("{}/structures/{}_structure.json", self.data_path, season_id);

            if let Some(parent) = std::path::Path::new(&path).parent() {
                tokio::fs::create_dir_all(parent).await.ok();
            }

            let content = serde_json::to_string_pretty(&config)?;
            tokio::fs::write(&path, content)
                .await
                .map_err(|e| BotError::StateSave {
                    path: path.clone(),
                    source: e,
                })?;

            changes.push(ConfigChange::new(
                ConfigChangeType::Add,
                "category_structure",
                &season_id,
                "Saved category structure",
            ));

            self.category_structures.insert(season_id, config);
        }

        self.staged.clear();
        Ok(changes)
    }

    /// Clear staged configuration without committing
    pub fn clear_staged(&mut self) {
        self.staged.clear();
    }

    // ========== Query Operations ==========

    /// Get a season config by ID
    pub fn get_season(&self, season_id: &str) -> Option<&SeasonConfig> {
        self.seasons.get(season_id)
    }

    /// Get all seasons
    pub fn get_all_seasons(&self) -> Vec<&SeasonConfig> {
        self.seasons.values().collect()
    }

    /// Find a user in any season by their verification ID
    pub fn find_user_by_verification_id(
        &self,
        verification_id: &str,
    ) -> Option<(&SeasonConfig, &SeasonUser)> {
        for season in self.seasons.values() {
            if season.active {
                if let Some(user) = season.find_user_by_id(verification_id) {
                    return Some((season, user));
                }
            }
        }
        None
    }

    /// Get special roles for a user by their verification ID
    pub fn get_special_roles_for_user(&self, verification_id: &str) -> Vec<String> {
        self.special_members
            .as_ref()
            .map(|sm| sm.get_roles_for_user(verification_id))
            .unwrap_or_default()
    }

    /// Get the global structure config
    pub fn get_global_structure(&self) -> Option<&GlobalStructureConfig> {
        self.global_structure.as_ref()
    }

    /// Get a category structure config
    pub fn get_category_structure(&self, season_id: &str) -> Option<&CategoryStructureConfig> {
        self.category_structures.get(season_id)
    }

    /// Get all category structures
    pub fn get_all_category_structures(&self) -> Vec<&CategoryStructureConfig> {
        self.category_structures.values().collect()
    }

    /// Get the default member role name
    pub fn get_default_member_role_name(&self) -> &str {
        self.global_structure
            .as_ref()
            .and_then(|gs| gs.get_default_member_role())
            .map(|r| r.name.as_str())
            .unwrap_or("Medlem")
    }

    /// Check which config files exist and which are missing for a season
    /// Returns (existing_files, missing_files)
    pub fn get_season_file_status(&self, season_id: &str) -> (Vec<String>, Vec<String>) {
        let mut existing = Vec::new();
        let mut missing = Vec::new();

        // Check users.json
        if self.seasons.contains_key(season_id) {
            existing.push(format!("seasons/{}/users.json", season_id));
        } else {
            missing.push(format!("seasons/{}/users.json", season_id));
        }

        // Check category.json
        if self.category_structures.contains_key(season_id) {
            existing.push(format!("seasons/{}/category.json", season_id));
        } else {
            missing.push(format!("seasons/{}/category.json", season_id));
        }

        // Check global.json (not per-season but important)
        if self.global_structure.is_some() {
            existing.push("global.json".to_string());
        } else {
            missing.push("global.json".to_string());
        }

        (existing, missing)
    }

    /// Get all season IDs that were just committed (from the changes list)
    pub fn get_committed_season_ids(changes: &[ConfigChange]) -> Vec<String> {
        changes
            .iter()
            .filter(|c| c.entity_type == "season")
            .map(|c| c.entity_name.clone())
            .collect()
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
                let bytes = serde_json::to_vec_pretty(season)?;
                Ok((format!("seasons/{}/users.json", name), bytes))
            }
            "special_members" | "special_roles" | "roles" => {
                let config =
                    self.special_members
                        .as_ref()
                        .ok_or_else(|| BotError::ConfigNotFound {
                            config_type: "roles".to_string(),
                            name: "".to_string(),
                        })?;
                let bytes = serde_json::to_vec_pretty(config)?;
                Ok(("roles.json".to_string(), bytes))
            }
            "global_structure" | "global" => {
                let config =
                    self.global_structure
                        .as_ref()
                        .ok_or_else(|| BotError::ConfigNotFound {
                            config_type: "global".to_string(),
                            name: "".to_string(),
                        })?;
                let bytes = serde_json::to_vec_pretty(config)?;
                Ok(("global.json".to_string(), bytes))
            }
            "category_structure" | "category" => {
                let name = name.ok_or_else(|| BotError::ConfigNotFound {
                    config_type: "category".to_string(),
                    name: "unspecified".to_string(),
                })?;
                let config =
                    self.category_structures
                        .get(name)
                        .ok_or_else(|| BotError::ConfigNotFound {
                            config_type: "category".to_string(),
                            name: name.to_string(),
                        })?;
                let bytes = serde_json::to_vec_pretty(config)?;
                Ok((format!("seasons/{}/category.json", name), bytes))
            }
            _ => Err(BotError::ConfigNotFound {
                config_type: config_type.to_string(),
                name: name.unwrap_or("").to_string(),
            }),
        }
    }

    /// Get list of all loaded config files for hint display
    pub fn get_all_config_files(&self) -> Vec<(String, String)> {
        let mut files = Vec::new();

        // Global config
        if self.global_structure.is_some() {
            files.push((
                "global.json".to_string(),
                "Global structure (roles, default channels)".to_string(),
            ));
        }

        // Seasons with their components
        for (season_id, season) in &self.seasons {
            files.push((
                format!("seasons/{}/users.json", season_id),
                format!("{} users", season.users.len()),
            ));

            if self.category_structures.contains_key(season_id) {
                files.push((
                    format!("seasons/{}/category.json", season_id),
                    "Category/channel structure".to_string(),
                ));
            }
        }

        // Roles (merged from all seasons)
        if let Some(sm) = &self.special_members {
            let role_count = sm.roles.len();
            let total_users: usize = sm.roles.values().map(|v| v.len()).sum();
            files.push((
                "roles.json".to_string(),
                format!("{} roles, {} assignments", role_count, total_users),
            ));
        }

        files
    }
}

/// Shared config manager type
pub type SharedConfigManager = Arc<tokio::sync::RwLock<ConfigManager>>;

pub fn create_shared_config_manager(data_path: &str) -> SharedConfigManager {
    Arc::new(tokio::sync::RwLock::new(ConfigManager::new(data_path)))
}
