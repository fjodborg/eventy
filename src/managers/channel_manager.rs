use poise::serenity_prelude::{self as serenity, ChannelId, GuildId, Http, Permissions};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, info, warn};

use super::config_manager::SharedConfigManager;
use super::role_manager::SharedRoleManager;
use crate::config::{
    CategoryStructureConfig, ChannelDefinition, ChannelPermissionLevel, ChannelType,
    GlobalStructureConfig,
};
use crate::error::Result;
use crate::state::{ChannelState, SharedChannelState};

/// Summary of changes made during a structure update
#[derive(Debug, Default, Clone)]
pub struct UpdateSummary {
    pub roles_created: Vec<String>,
    pub roles_existing: Vec<String>,
    pub category_created: Option<String>,
    pub category_existing: Option<String>,
    pub channels_created: Vec<String>,
    pub channels_updated: Vec<String>,
    pub permissions_applied: Vec<(String, String, String)>, // (channel, role, level)
    pub warnings: Vec<String>,
}

impl UpdateSummary {
    /// Format the summary as a human-readable string
    pub fn format(&self) -> String {
        let mut lines = Vec::new();

        // Roles
        if !self.roles_created.is_empty() {
            lines.push(format!("**Roles created:** {}", self.roles_created.join(", ")));
        }
        if !self.roles_existing.is_empty() {
            lines.push(format!("**Roles verified:** {}", self.roles_existing.join(", ")));
        }

        // Category
        if let Some(cat) = &self.category_created {
            lines.push(format!("**Category created:** {}", cat));
        }
        if let Some(cat) = &self.category_existing {
            lines.push(format!("**Category verified:** {}", cat));
        }

        // Channels
        if !self.channels_created.is_empty() {
            lines.push(format!("**Channels created:** {}", self.channels_created.join(", ")));
        }
        if !self.channels_updated.is_empty() {
            lines.push(format!("**Channels updated:** {}", self.channels_updated.join(", ")));
        }

        // Permissions summary
        if !self.permissions_applied.is_empty() {
            lines.push(format!("**Permissions configured:** {} role/channel pairs", self.permissions_applied.len()));
        }

        // Warnings
        if !self.warnings.is_empty() {
            lines.push(format!("\n**Warnings:**\n- {}", self.warnings.join("\n- ")));
        }

        if lines.is_empty() {
            "No changes were made.".to_string()
        } else {
            lines.join("\n")
        }
    }
}

/// Manages Discord channel and category creation
pub struct ChannelManager {
    /// Channel state for caching
    state: SharedChannelState,

    /// Role manager for permission lookups
    role_manager: SharedRoleManager,

    /// Config manager for permission definitions
    config_manager: SharedConfigManager,
}

impl ChannelManager {
    pub fn new(
        state: SharedChannelState,
        role_manager: SharedRoleManager,
        config_manager: SharedConfigManager,
    ) -> Self {
        Self {
            state,
            role_manager,
            config_manager,
        }
    }

    /// Ensure the maintainers channel exists (admin-only)
    pub async fn ensure_maintainers_channel(
        &self,
        http: &Http,
        guild_id: GuildId,
    ) -> Result<ChannelId> {
        const MAINTAINERS_CHANNEL_NAME: &str = "maintainers";

        // Check if it already exists in cache
        {
            let state: tokio::sync::RwLockReadGuard<'_, ChannelState> = self.state.read().await;
            if let Some(guild) = state.get_guild(&guild_id.to_string()) {
                if let Some(channel_id) = &guild.maintainers_channel_id {
                    if let Ok(id) = channel_id.parse::<u64>() {
                        return Ok(ChannelId::new(id));
                    }
                }
            }
        }

        // Check if it exists in Discord
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.name == MAINTAINERS_CHANNEL_NAME {
                // Update cache
                let mut state: tokio::sync::RwLockWriteGuard<'_, ChannelState> =
                    self.state.write().await;
                let guild_name = "";
                let guild = state.get_guild_mut(&guild_id.to_string(), &guild_name);
                guild.set_maintainers_channel(&channel_id.to_string());

                info!("Found existing maintainers channel: {}", channel_id);
                return Ok(*channel_id);
            }
        }

        // Create the channel with admin-only permissions
        let guild = guild_id.to_partial_guild(http).await?;
        let everyone_role = guild.role_by_name("@everyone").map(|r| r.id);

        let mut permission_overwrites = vec![];

        // Deny everyone
        if let Some(everyone_id) = everyone_role {
            permission_overwrites.push(serenity::PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL,
                kind: serenity::PermissionOverwriteType::Role(everyone_id),
            });
        }

        let channel = guild_id
            .create_channel(
                http,
                serenity::CreateChannel::new(MAINTAINERS_CHANNEL_NAME)
                    .kind(serenity::ChannelType::Text)
                    .topic(
                        "Bot configuration channel. Upload JSON files here to configure the bot.",
                    )
                    .permissions(permission_overwrites),
            )
            .await?;

        // Update cache
        {
            let mut state: tokio::sync::RwLockWriteGuard<'_, ChannelState> =
                self.state.write().await;
            let guild_name = "";
            let guild_state = state.get_guild_mut(&guild_id.to_string(), &guild_name);
            guild_state.set_maintainers_channel(&channel.id.to_string());
            guild_state.add_channel(
                MAINTAINERS_CHANNEL_NAME,
                &channel.id.to_string(),
                None,
                "text",
            );
        }

        info!("Created maintainers channel: {}", channel.id);
        Ok(channel.id)
    }

    /// Check if a channel is the maintainers channel
    pub async fn is_maintainers_channel(&self, channel_id: ChannelId, guild_id: GuildId) -> bool {
        let state = self.state.read().await;
        if let Some(guild) = state.get_guild(&guild_id.to_string()) {
            if let Some(maintainers_id) = &guild.maintainers_channel_id {
                return maintainers_id == &channel_id.to_string();
            }
        }
        false
    }

    /// Ensure a category exists
    pub async fn ensure_category_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        name: &str,
    ) -> Result<ChannelId> {
        // Check cache
        {
            let state = self.state.read().await;
            if let Some(guild) = state.get_guild(&guild_id.to_string()) {
                if let Some(id_str) = guild.get_category_id(name) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        return Ok(ChannelId::new(id));
                    }
                }
            }
        }

        // Check Discord
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.kind == serenity::ChannelType::Category && channel.name == name {
                // Update cache
                let mut state = self.state.write().await;
                let guild_name = "";
                let guild = state.get_guild_mut(&guild_id.to_string(), &guild_name);
                guild.add_category(name, &channel_id.to_string(), channel.position as u16);

                return Ok(*channel_id);
            }
        }

        // Create category
        let channel = guild_id
            .create_channel(
                http,
                serenity::CreateChannel::new(name).kind(serenity::ChannelType::Category),
            )
            .await?;

        // Update cache
        {
            let mut state = self.state.write().await;
            let guild_name = "";
            let guild = state.get_guild_mut(&guild_id.to_string(), &guild_name);
            guild.add_category(name, &channel.id.to_string(), 0);
        }

        info!("Created category '{}'", name);
        Ok(channel.id)
    }

    /// Ensure a channel exists within a category
    pub async fn ensure_channel_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        channel_def: &ChannelDefinition,
        parent_id: Option<ChannelId>,
    ) -> Result<ChannelId> {
        // Check cache - Disabled to ensure we check parent_id correctly
        /*
        {
            let state = self.state.read().await;
            if let Some(guild) = state.get_guild(&guild_id.to_string()) {
                if let Some(id_str) = guild.get_channel_id(&channel_def.name) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        return Ok(ChannelId::new(id));
                    }
                }
            }
        }
        */

        // Check Discord
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.name == channel_def.name {
                // Check if parent matches (if specified)
                if let Some(parent) = parent_id {
                    if channel.parent_id != Some(parent) {
                        continue;
                    }
                }

                // Update cache
                {
                    let mut state = self.state.write().await;
                    let guild_name = "";
                    let guild = state.get_guild_mut(&guild_id.to_string(), &guild_name);
                    let parent_name = parent_id.map(|id| id.to_string());
                    guild.add_channel(
                        &channel_def.name,
                        &channel_id.to_string(),
                        parent_name.as_deref(),
                        &format!("{:?}", channel_def.channel_type),
                    );
                }

                // Ensure permissions are up to date
                let permission_overwrites = self
                    .build_permission_overwrites(
                        http,
                        guild_id,
                        &channel_def.role_permissions,
                        &channel_def.channel_type,
                    )
                    .await?;

                // We only update if permissions are different to avoid rate limits
                // But for now, let's just update to be safe and ensure correctness
                if let Err(e) = channel_id
                    .edit(
                        http,
                        serenity::EditChannel::new().permissions(permission_overwrites),
                    )
                    .await
                {
                    warn!(
                        "Failed to update permissions for channel '{}': {}",
                        channel_def.name, e
                    );
                } else {
                    info!("Verified permissions for channel '{}'", channel_def.name);
                }

                return Ok(*channel_id);
            }
        }

        // Build permission overwrites
        // Build permission overwrites
        let permission_overwrites = self
            .build_permission_overwrites(
                http,
                guild_id,
                &channel_def.role_permissions,
                &channel_def.channel_type,
            )
            .await?;

        // Create channel
        let mut create_channel = serenity::CreateChannel::new(&channel_def.name)
            .kind(channel_def.channel_type.to_serenity())
            .permissions(permission_overwrites);

        if let Some(parent) = parent_id {
            create_channel = create_channel.category(parent);
        }

        let channel = guild_id.create_channel(http, create_channel).await?;

        // Update cache
        {
            let mut state = self.state.write().await;
            let guild_name = "";
            let guild = state.get_guild_mut(&guild_id.to_string(), &guild_name);
            let parent_name = parent_id.map(|id| id.to_string());
            guild.add_channel(
                &channel_def.name,
                &channel.id.to_string(),
                parent_name.as_deref(),
                &format!("{:?}", channel_def.channel_type),
            );
        }

        info!("Created channel '{}'", channel_def.name);
        Ok(channel.id)
    }

    /// Build permission overwrites from role permissions map
    async fn build_permission_overwrites(
        &self,
        http: &Http,
        guild_id: GuildId,
        role_permissions: &HashMap<String, ChannelPermissionLevel>,
        channel_type: &ChannelType,
    ) -> Result<Vec<serenity::PermissionOverwrite>> {
        let mut overwrites = Vec::new();
        let role_manager = self.role_manager.read().await;

        for (role_name, level) in role_permissions {
            match role_manager.get_role_id(http, guild_id, role_name).await {
                Ok(role_id) => {
                    let (allow, deny) = level.to_permissions(
                        channel_type,
                        self.config_manager
                            .read()
                            .await
                            .get_global_structure()
                            .unwrap_or(&GlobalStructureConfig::default()),
                    );
                    overwrites.push(serenity::PermissionOverwrite {
                        allow,
                        deny,
                        kind: serenity::PermissionOverwriteType::Role(role_id),
                    });
                }
                Err(e) => {
                    warn!("Could not find role '{}' for permissions: {}", role_name, e);
                }
            }
        }

        Ok(overwrites)
    }

    /// Ensure all channels from a structure config exist
    pub async fn ensure_structure_exists(
        &self,
        http: &Http,
        guild_id: GuildId,
        global: &GlobalStructureConfig,
        category_override: Option<&CategoryStructureConfig>,
    ) -> Result<UpdateSummary> {
        let mut summary = UpdateSummary::default();

        // First, ensure all roles from global config exist
        info!("Ensuring roles exist...");
        let role_manager = self.role_manager.read().await;
        let existing_roles = guild_id.roles(http).await?;

        for role_def in &global.default_roles {
            if existing_roles.iter().any(|(_, r)| r.name == role_def.name) {
                debug!("Role '{}' already exists", role_def.name);
                summary.roles_existing.push(role_def.name.clone());
            } else {
                // Create the role
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
                        info!("Created role '{}' (ID: {})", role_def.name, role.id);
                        summary.roles_created.push(role_def.name.clone());
                    }
                    Err(e) => {
                        let msg = format!("Failed to create role '{}': {}", role_def.name, e);
                        warn!("{}", msg);
                        summary.warnings.push(msg);
                    }
                }
            }
        }
        drop(role_manager);

        // Merge configurations
        let channels = if let Some(override_config) = category_override {
            let merged = override_config.merge_with_global(global);

            // Create category first
            info!("Ensuring category '{}' exists...", merged.category_name);
            let (category_id, cat_created) = self
                .ensure_category_exists_tracked(http, guild_id, &merged.category_name)
                .await?;

            if cat_created {
                summary.category_created = Some(merged.category_name.clone());
            } else {
                summary.category_existing = Some(merged.category_name.clone());
            }

            // Create channels within category
            for channel_def in &merged.channels {
                let (_, created, updated) = self
                    .ensure_channel_exists_tracked(
                        http,
                        guild_id,
                        channel_def,
                        Some(category_id),
                        &mut summary,
                    )
                    .await?;

                if created {
                    summary.channels_created.push(channel_def.name.clone());
                } else if updated {
                    summary.channels_updated.push(channel_def.name.clone());
                }
            }

            merged.channels
        } else {
            // Just use global channels
            for channel_def in &global.default_channels {
                let (_, created, updated) = self
                    .ensure_channel_exists_tracked(http, guild_id, channel_def, None, &mut summary)
                    .await?;

                if created {
                    summary.channels_created.push(channel_def.name.clone());
                } else if updated {
                    summary.channels_updated.push(channel_def.name.clone());
                }
            }

            global.default_channels.clone()
        };

        info!(
            "Structure update complete: {} roles, {} channels",
            summary.roles_created.len() + summary.roles_existing.len(),
            channels.len()
        );
        Ok(summary)
    }

    /// Ensure a category exists, returning whether it was created
    async fn ensure_category_exists_tracked(
        &self,
        http: &Http,
        guild_id: GuildId,
        name: &str,
    ) -> Result<(ChannelId, bool)> {
        // Check cache
        {
            let state = self.state.read().await;
            if let Some(guild) = state.get_guild(&guild_id.to_string()) {
                if let Some(id_str) = guild.get_category_id(name) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        debug!("Category '{}' found in cache", name);
                        return Ok((ChannelId::new(id), false));
                    }
                }
            }
        }

        // Check Discord
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.kind == serenity::ChannelType::Category && channel.name == name {
                // Update cache
                let mut state = self.state.write().await;
                let guild_name = "";
                let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
                guild.add_category(name, &channel_id.to_string(), channel.position as u16);

                debug!("Category '{}' already exists on Discord", name);
                return Ok((*channel_id, false));
            }
        }

        // Create category
        let channel = guild_id
            .create_channel(
                http,
                serenity::CreateChannel::new(name).kind(serenity::ChannelType::Category),
            )
            .await?;

        // Update cache
        {
            let mut state = self.state.write().await;
            let guild_name = "";
            let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
            guild.add_category(name, &channel.id.to_string(), 0);
        }

        info!("Created category '{}'", name);
        Ok((channel.id, true))
    }

    /// Ensure a channel exists, returning whether it was created or updated
    async fn ensure_channel_exists_tracked(
        &self,
        http: &Http,
        guild_id: GuildId,
        channel_def: &ChannelDefinition,
        parent_id: Option<ChannelId>,
        summary: &mut UpdateSummary,
    ) -> Result<(ChannelId, bool, bool)> {
        // Check Discord for existing channel
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.name == channel_def.name {
                // Check if parent matches (if specified)
                if let Some(parent) = parent_id {
                    if channel.parent_id != Some(parent) {
                        continue;
                    }
                }

                // Update cache
                {
                    let mut state = self.state.write().await;
                    let guild_name = "";
                    let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
                    let parent_name = parent_id.map(|id| id.to_string());
                    guild.add_channel(
                        &channel_def.name,
                        &channel_id.to_string(),
                        parent_name.as_deref(),
                        &format!("{:?}", channel_def.channel_type),
                    );
                }

                // Build and apply permissions
                let permission_overwrites = self
                    .build_permission_overwrites_tracked(
                        http,
                        guild_id,
                        &channel_def.role_permissions,
                        &channel_def.channel_type,
                        &channel_def.name,
                        summary,
                    )
                    .await?;

                if let Err(e) = channel_id
                    .edit(
                        http,
                        serenity::EditChannel::new().permissions(permission_overwrites),
                    )
                    .await
                {
                    let msg = format!(
                        "Failed to update permissions for channel '{}': {}",
                        channel_def.name, e
                    );
                    warn!("{}", msg);
                    summary.warnings.push(msg);
                    return Ok((*channel_id, false, false));
                }

                info!(
                    "Updated permissions for existing channel '{}'",
                    channel_def.name
                );
                return Ok((*channel_id, false, true));
            }
        }

        // Build permission overwrites for new channel
        let permission_overwrites = self
            .build_permission_overwrites_tracked(
                http,
                guild_id,
                &channel_def.role_permissions,
                &channel_def.channel_type,
                &channel_def.name,
                summary,
            )
            .await?;

        // Create channel
        let mut create_channel = serenity::CreateChannel::new(&channel_def.name)
            .kind(channel_def.channel_type.to_serenity())
            .permissions(permission_overwrites);

        if let Some(parent) = parent_id {
            create_channel = create_channel.category(parent);
        }

        let channel = guild_id.create_channel(http, create_channel).await?;

        // Update cache
        {
            let mut state = self.state.write().await;
            let guild_name = "";
            let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
            let parent_name = parent_id.map(|id| id.to_string());
            guild.add_channel(
                &channel_def.name,
                &channel.id.to_string(),
                parent_name.as_deref(),
                &format!("{:?}", channel_def.channel_type),
            );
        }

        info!(
            "Created channel '{}' ({:?})",
            channel_def.name, channel_def.channel_type
        );
        Ok((channel.id, true, false))
    }

    /// Build permission overwrites and track them in the summary
    async fn build_permission_overwrites_tracked(
        &self,
        http: &Http,
        guild_id: GuildId,
        role_permissions: &HashMap<String, ChannelPermissionLevel>,
        channel_type: &ChannelType,
        channel_name: &str,
        summary: &mut UpdateSummary,
    ) -> Result<Vec<serenity::PermissionOverwrite>> {
        let mut overwrites = Vec::new();
        let role_manager = self.role_manager.read().await;

        for (role_name, level) in role_permissions {
            match role_manager.get_role_id(http, guild_id, role_name).await {
                Ok(role_id) => {
                    let (allow, deny) = level.to_permissions(
                        channel_type,
                        self.config_manager
                            .read()
                            .await
                            .get_global_structure()
                            .unwrap_or(&GlobalStructureConfig::default()),
                    );
                    overwrites.push(serenity::PermissionOverwrite {
                        allow,
                        deny,
                        kind: serenity::PermissionOverwriteType::Role(role_id),
                    });

                    let level_str = format!("{:?}", level).to_lowercase();
                    info!(
                        "  {} -> {} = {} (allow: {:?}, deny: {:?})",
                        channel_name, role_name, level_str, allow, deny
                    );
                    summary.permissions_applied.push((
                        channel_name.to_string(),
                        role_name.clone(),
                        level_str,
                    ));
                }
                Err(e) => {
                    let msg = format!(
                        "Could not find role '{}' for channel '{}': {}",
                        role_name, channel_name, e
                    );
                    warn!("{}", msg);
                    summary.warnings.push(msg);
                }
            }
        }

        Ok(overwrites)
    }

    /// Get channel ID by name
    pub async fn get_channel_id(&self, guild_id: GuildId, name: &str) -> Option<ChannelId> {
        let state: tokio::sync::RwLockReadGuard<'_, ChannelState> = self.state.read().await;
        if let Some(guild) = state.get_guild(&guild_id.to_string()) {
            if let Some(id_str) = guild.get_channel_id(name) {
                if let Ok(id) = id_str.parse::<u64>() {
                    return Some(ChannelId::new(id));
                }
            }
        }
        None
    }

    /// Get category ID by name
    pub async fn get_category_id(&self, guild_id: GuildId, name: &str) -> Option<ChannelId> {
        let state: tokio::sync::RwLockReadGuard<'_, ChannelState> = self.state.read().await;
        if let Some(guild) = state.get_guild(&guild_id.to_string()) {
            if let Some(id_str) = guild.get_category_id(name) {
                if let Ok(id) = id_str.parse::<u64>() {
                    return Some(ChannelId::new(id));
                }
            }
        }
        None
    }
}

/// Shared channel manager type
pub type SharedChannelManager = Arc<tokio::sync::RwLock<ChannelManager>>;

pub fn create_shared_channel_manager(
    state: SharedChannelState,
    role_manager: SharedRoleManager,
    config_manager: SharedConfigManager,
) -> SharedChannelManager {
    Arc::new(tokio::sync::RwLock::new(ChannelManager::new(
        state,
        role_manager,
        config_manager,
    )))
}
