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
    pub channels_reordered: Vec<String>,
    pub permissions_applied: Vec<(String, String, String)>, // (channel, role, level)
    pub missing_roles: Vec<String>,
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
        if !self.channels_reordered.is_empty() {
            lines.push(format!("**Channels reordered:** {}", self.channels_reordered.join(", ")));
        }

        // Permissions summary
        if !self.permissions_applied.is_empty() {
            lines.push(format!("**Permissions configured:** {} role/channel pairs", self.permissions_applied.len()));
        }

        // Missing roles
        if !self.missing_roles.is_empty() {
            lines.push(format!("**Missing roles:** {}", self.missing_roles.join(", ")));
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

        // Deny everyone - VIEW_CHANNEL and CONNECT for channel isolation
        // Note: MANAGE_NICKNAMES and CHANGE_NICKNAME are server-level permissions,
        // they cannot be set as channel overwrites
        if let Some(everyone_id) = everyone_role {
            permission_overwrites.push(serenity::PermissionOverwrite {
                allow: Permissions::empty(),
                deny: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
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

        // Get permission definitions from config
        let config = self.config_manager.read().await;
        let permission_definitions = config
            .get_global_permissions()
            .map(|p| p.definitions.clone())
            .unwrap_or_default();
        drop(config);

        // Build a GlobalStructureConfig with the loaded permission definitions
        let global_config = GlobalStructureConfig {
            permission_definitions,
            ..Default::default()
        };

        for (role_name, level) in role_permissions {
            match role_manager.get_role_id(http, guild_id, role_name).await {
                Ok(role_id) => {
                    let (allow, deny) = level.to_permissions(channel_type, &global_config);
                    info!(
                        "Permission overwrite for role '{}': allow={:?}, deny={:?}",
                        role_name, allow, deny
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

        // Get the bot's user ID to add explicit permission for it
        let bot_user = http.get_current_user().await?;
        let bot_user_id = bot_user.id;

        // Check Discord
        let channels = guild_id.channels(http).await?;
        for (channel_id, channel) in &channels {
            if channel.kind == serenity::ChannelType::Category && channel.name == name {
                // Update cache
                {
                    let mut state = self.state.write().await;
                    let guild_name = "";
                    let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
                    guild.add_category(name, &channel_id.to_string(), channel.position as u16);
                }

                // Ensure bot has permission on existing category
                // This is needed in case @everyone was denied but bot wasn't given explicit access
                let existing_overwrites = channel.permission_overwrites.clone();
                let has_bot_permission = existing_overwrites.iter().any(|ow| {
                    matches!(ow.kind, serenity::PermissionOverwriteType::Member(m) if m == bot_user_id)
                });

                if !has_bot_permission {
                    debug!("Adding bot permission to existing category '{}'", name);
                    let mut new_overwrites = existing_overwrites;
                    // Note: We don't include MANAGE_ROLES here because that's a server-level permission
                    // The bot needs MANAGE_ROLES at server level, not channel level
                    new_overwrites.push(serenity::PermissionOverwrite {
                        allow: Permissions::VIEW_CHANNEL
                            | Permissions::MANAGE_CHANNELS
                            | Permissions::SEND_MESSAGES
                            | Permissions::CONNECT,
                        deny: Permissions::empty(),
                        kind: serenity::PermissionOverwriteType::Member(bot_user_id),
                    });

                    if let Err(e) = channel_id
                        .edit(http, serenity::EditChannel::new().permissions(new_overwrites))
                        .await
                    {
                        warn!("Failed to add bot permission to category '{}': {}", name, e);
                    }
                }

                debug!("Category '{}' already exists on Discord", name);
                return Ok((*channel_id, false));
            }
        }

        // Create category with bot permission already set
        // Note: We don't include MANAGE_ROLES in the overwrite - that's a server-level permission
        let channel = guild_id
            .create_channel(
                http,
                serenity::CreateChannel::new(name)
                    .kind(serenity::ChannelType::Category)
                    .permissions(vec![serenity::PermissionOverwrite {
                        allow: Permissions::VIEW_CHANNEL
                            | Permissions::MANAGE_CHANNELS
                            | Permissions::SEND_MESSAGES
                            | Permissions::CONNECT,
                        deny: Permissions::empty(),
                        kind: serenity::PermissionOverwriteType::Member(bot_user_id),
                    }]),
            )
            .await?;

        // Update cache
        {
            let mut state = self.state.write().await;
            let guild_name = "";
            let guild = state.get_guild_mut(&guild_id.to_string(), guild_name);
            guild.add_category(name, &channel.id.to_string(), 0);
        }

        info!("Created category '{}' with bot permission", name);
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

        // Get permission definitions from config
        let config = self.config_manager.read().await;
        let permission_definitions = config
            .get_global_permissions()
            .map(|p| p.definitions.clone())
            .unwrap_or_default();
        drop(config);

        // Build a GlobalStructureConfig with the loaded permission definitions
        let global_config = GlobalStructureConfig {
            permission_definitions,
            ..Default::default()
        };

        for (role_name, level) in role_permissions {
            match role_manager.get_role_id(http, guild_id, role_name).await {
                Ok(role_id) => {
                    let (allow, deny) = level.to_permissions(channel_type, &global_config);
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
                    warn!(
                        "Could not find role '{}' for channel '{}': {}",
                        role_name, channel_name, e
                    );
                    if !summary.missing_roles.contains(role_name) {
                        summary.missing_roles.push(role_name.clone());
                    }
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

    /// Sync a season's channels to Discord
    ///
    /// Creates/updates a category with the given name and syncs all channels within it.
    /// Automatically denies @everyone access to ensure season isolation.
    /// Returns an UpdateSummary with details of what was created/updated.
    pub async fn sync_season_channels(
        &self,
        http: &Http,
        guild_id: GuildId,
        category_name: &str,
        channels: &[ChannelDefinition],
    ) -> Result<UpdateSummary> {
        let mut summary = UpdateSummary::default();

        // Ensure category exists
        info!("Syncing season channels: category='{}', {} channels", category_name, channels.len());
        let (category_id, cat_created) = self
            .ensure_category_exists_tracked(http, guild_id, category_name)
            .await?;

        if cat_created {
            summary.category_created = Some(category_name.to_string());
        } else {
            summary.category_existing = Some(category_name.to_string());
        }

        // Set @everyone deny on the category itself for season isolation
        if let Err(e) = self.deny_everyone_on_channel(http, guild_id, category_id).await {
            warn!("Failed to set @everyone deny on category '{}': {}", category_name, e);
            summary.warnings.push(format!("Failed to deny @everyone on category: {}", e));
        } else {
            info!("Set @everyone deny on category '{}'", category_name);
        }

        // Create/update each channel
        for channel_def in channels {
            let (channel_id, created, updated) = self
                .ensure_channel_exists_tracked_with_everyone_deny(
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

        // Reorder channels based on their position field
        match self.reorder_channels_in_category(http, guild_id, category_id, channels).await {
            Ok(reordered) => {
                if !reordered.is_empty() {
                    info!("Reordered {} channels in category '{}'", reordered.len(), category_name);
                    summary.channels_reordered = reordered;
                }
            }
            Err(e) => {
                warn!("Failed to reorder channels in category '{}': {}", category_name, e);
                summary.warnings.push(format!("Failed to reorder channels: {}", e));
            }
        }

        info!(
            "Season sync complete: category={}, created={}, updated={}, missing_roles={}",
            category_name,
            summary.channels_created.len(),
            summary.channels_updated.len(),
            summary.missing_roles.len()
        );

        Ok(summary)
    }

    /// Reorder channels within a category based on their position field
    /// Returns a list of channel names that were reordered
    async fn reorder_channels_in_category(
        &self,
        http: &Http,
        guild_id: GuildId,
        category_id: ChannelId,
        channel_defs: &[ChannelDefinition],
    ) -> Result<Vec<String>> {
        // Get all channels in the guild
        let guild_channels = guild_id.channels(http).await?;

        // Find channels that belong to this category and have position defined
        let mut channels_to_reorder: Vec<(ChannelId, String, u16)> = Vec::new();

        for channel_def in channel_defs {
            // Only process channels that have a position defined
            if let Some(position) = channel_def.position {
                // Find the channel ID by name within this category
                for (channel_id, channel) in &guild_channels {
                    if channel.name == channel_def.name && channel.parent_id == Some(category_id) {
                        channels_to_reorder.push((*channel_id, channel_def.name.clone(), position));
                        break;
                    }
                }
            }
        }

        if channels_to_reorder.is_empty() {
            debug!("No channels with position field to reorder");
            return Ok(Vec::new());
        }

        // Sort by position
        channels_to_reorder.sort_by_key(|(_, _, pos)| *pos);

        info!(
            "Reordering {} channels in category",
            channels_to_reorder.len()
        );

        let mut reordered_names = Vec::new();

        // Use Discord's edit channel endpoint to set positions
        // We need to set positions relative to each other within the category
        for (channel_id, channel_name, position) in &channels_to_reorder {
            if let Err(e) = channel_id
                .edit(http, serenity::EditChannel::new().position(*position))
                .await
            {
                warn!(
                    "Failed to set position {} for channel '{}': {}",
                    position, channel_name, e
                );
            } else {
                debug!("Set channel '{}' position to {}", channel_name, position);
                reordered_names.push(channel_name.clone());
            }
        }

        Ok(reordered_names)
    }

    /// Deny @everyone access to a channel/category while ensuring bot keeps access
    async fn deny_everyone_on_channel(
        &self,
        http: &Http,
        guild_id: GuildId,
        channel_id: ChannelId,
    ) -> Result<()> {
        let guild = guild_id.to_partial_guild(http).await?;
        let everyone_role_id = guild.id.everyone_role();

        // Get the bot's user ID to add explicit permission for it
        let bot_user = http.get_current_user().await?;
        let bot_user_id = bot_user.id;

        // Get existing permission overwrites to preserve them
        let channel = channel_id.to_channel(http).await?;
        let existing_overwrites = match &channel {
            serenity::Channel::Guild(gc) => gc.permission_overwrites.clone(),
            _ => vec![],
        };

        // Build new overwrites, updating @everyone if it exists or adding it
        // Also remove any existing bot user overwrite so we can add a fresh one
        let mut new_overwrites: Vec<serenity::PermissionOverwrite> = existing_overwrites
            .into_iter()
            .filter(|ow| {
                // Remove existing @everyone overwrite, we'll add our own
                if matches!(ow.kind, serenity::PermissionOverwriteType::Role(r) if r == everyone_role_id) {
                    return false;
                }
                // Remove existing bot user overwrite, we'll add our own
                if matches!(ow.kind, serenity::PermissionOverwriteType::Member(m) if m == bot_user_id) {
                    return false;
                }
                true
            })
            .collect();

        // Add bot user permission FIRST - ensure bot can always access and manage
        // Note: MANAGE_ROLES is a server-level permission, not channel-level
        new_overwrites.push(serenity::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL
                | Permissions::MANAGE_CHANNELS
                | Permissions::SEND_MESSAGES
                | Permissions::CONNECT,
            deny: Permissions::empty(),
            kind: serenity::PermissionOverwriteType::Member(bot_user_id),
        });

        // Add @everyone deny - VIEW_CHANNEL and CONNECT for channel isolation
        // Note: MANAGE_NICKNAMES and CHANGE_NICKNAME are server-level permissions,
        // they cannot be set as channel overwrites
        new_overwrites.push(serenity::PermissionOverwrite {
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
            kind: serenity::PermissionOverwriteType::Role(everyone_role_id),
        });

        channel_id
            .edit(http, serenity::EditChannel::new().permissions(new_overwrites))
            .await?;

        Ok(())
    }

    /// Ensure a channel exists with @everyone denied, returning whether it was created or updated
    async fn ensure_channel_exists_tracked_with_everyone_deny(
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

                // Build permissions WITH @everyone deny
                let permission_overwrites = self
                    .build_permission_overwrites_with_everyone_deny(
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
                    "Updated permissions for existing channel '{}' (with @everyone deny)",
                    channel_def.name
                );
                return Ok((*channel_id, false, true));
            }
        }

        // Build permission overwrites for new channel WITH @everyone deny
        let permission_overwrites = self
            .build_permission_overwrites_with_everyone_deny(
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
            "Created channel '{}' ({:?}) with @everyone deny",
            channel_def.name, channel_def.channel_type
        );
        Ok((channel.id, true, false))
    }

    /// Build permission overwrites with @everyone denied and track them in the summary
    async fn build_permission_overwrites_with_everyone_deny(
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

        // Get @everyone role ID and deny it
        let guild = guild_id.to_partial_guild(http).await?;
        let everyone_role_id = guild.id.everyone_role();

        // Get the bot's user ID to add explicit permission for it
        let bot_user = http.get_current_user().await?;
        let bot_user_id = bot_user.id;

        // Add bot user permission FIRST - ensure bot can always access and manage the channel
        // Note: MANAGE_ROLES is a server-level permission, not channel-level
        overwrites.push(serenity::PermissionOverwrite {
            allow: Permissions::VIEW_CHANNEL
                | Permissions::MANAGE_CHANNELS
                | Permissions::SEND_MESSAGES
                | Permissions::CONNECT,
            deny: Permissions::empty(),
            kind: serenity::PermissionOverwriteType::Member(bot_user_id),
        });
        debug!(
            "  {} -> Bot = allow (VIEW_CHANNEL, MANAGE_CHANNELS, SEND_MESSAGES, CONNECT)",
            channel_name
        );

        // Add @everyone deny - VIEW_CHANNEL and CONNECT for channel isolation
        // Note: MANAGE_NICKNAMES and CHANGE_NICKNAME are server-level permissions,
        // they cannot be set as channel overwrites
        overwrites.push(serenity::PermissionOverwrite {
            allow: Permissions::empty(),
            deny: Permissions::VIEW_CHANNEL | Permissions::CONNECT,
            kind: serenity::PermissionOverwriteType::Role(everyone_role_id),
        });
        info!(
            "  {} -> @everyone = deny (VIEW_CHANNEL, CONNECT)",
            channel_name
        );

        // Get permission definitions from config
        let config = self.config_manager.read().await;
        let permission_definitions = config
            .get_global_permissions()
            .map(|p| p.definitions.clone())
            .unwrap_or_default();
        drop(config);

        // Build a GlobalStructureConfig with the loaded permission definitions
        let global_config = GlobalStructureConfig {
            permission_definitions,
            ..Default::default()
        };

        for (role_name, level) in role_permissions {
            match role_manager.get_role_id(http, guild_id, role_name).await {
                Ok(role_id) => {
                    let (allow, deny) = level.to_permissions(channel_type, &global_config);
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
                    warn!(
                        "Could not find role '{}' for channel '{}': {}",
                        role_name, channel_name, e
                    );
                    if !summary.missing_roles.contains(role_name) {
                        summary.missing_roles.push(role_name.clone());
                    }
                }
            }
        }

        Ok(overwrites)
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
