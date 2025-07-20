use anyhow::{Context, Result};
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};
use std::{collections::HashMap, path::Path, sync::Arc};
use tokio::sync::RwLock;
use tracing::{ info, warn};

// Make sure this matches your existing permissions module
// use crate::permissions::RoleConfig;

use crate::role_manager::DiscordRoleConfig as RoleConfig;
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPermission {
    pub name: String,
    pub permission: PermissionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PermissionType {
    Read,
    ReadWrite,
    Admin,
    None,
}

impl PermissionType {
    pub fn to_serenity_permissions(&self) -> (serenity::Permissions, serenity::Permissions) {
        match self {
            PermissionType::Read => (
                serenity::Permissions::VIEW_CHANNEL | serenity::Permissions::READ_MESSAGE_HISTORY,
                serenity::Permissions::SEND_MESSAGES | serenity::Permissions::MANAGE_MESSAGES,
            ),
            PermissionType::ReadWrite => (
                serenity::Permissions::VIEW_CHANNEL
                    | serenity::Permissions::SEND_MESSAGES
                    | serenity::Permissions::READ_MESSAGE_HISTORY
                    | serenity::Permissions::ADD_REACTIONS
                    | serenity::Permissions::USE_EXTERNAL_EMOJIS
                    | serenity::Permissions::EMBED_LINKS
                    | serenity::Permissions::ATTACH_FILES,
                serenity::Permissions::empty(),
            ),
            PermissionType::Admin => (
                serenity::Permissions::VIEW_CHANNEL
                    | serenity::Permissions::SEND_MESSAGES
                    | serenity::Permissions::READ_MESSAGE_HISTORY
                    | serenity::Permissions::MANAGE_MESSAGES
                    | serenity::Permissions::MANAGE_CHANNELS
                    | serenity::Permissions::ADD_REACTIONS
                    | serenity::Permissions::USE_EXTERNAL_EMOJIS
                    | serenity::Permissions::EMBED_LINKS
                    | serenity::Permissions::ATTACH_FILES,
                serenity::Permissions::empty(),
            ),
            PermissionType::None => (serenity::Permissions::empty(), serenity::Permissions::all()),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RolePermissions {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub view_channel: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub send_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub read_message_history: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_messages: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_channels: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub add_reactions: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub use_external_emojis: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub embed_links: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub attach_files: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub connect: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub speak: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub mute_members: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub move_members: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub administrator: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub kick_members: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub ban_members: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_guild: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_roles: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_webhooks: Option<bool>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub manage_emojis_and_stickers: Option<bool>,
}

impl Default for RolePermissions {
    fn default() -> Self {
        Self {
            view_channel: None,
            send_messages: None,
            read_message_history: None,
            manage_messages: None,
            manage_channels: None,
            add_reactions: None,
            use_external_emojis: None,
            embed_links: None,
            attach_files: None,
            connect: None,
            speak: None,
            mute_members: None,
            move_members: None,
            administrator: None,
            kick_members: None,
            ban_members: None,
            manage_guild: None,
            manage_roles: None,
            manage_webhooks: None,
            manage_emojis_and_stickers: None,
        }
    }
}

impl RolePermissions {
    pub fn to_serenity_permissions(&self) -> serenity::Permissions {
        let mut permissions = serenity::Permissions::empty();

        if self.view_channel.unwrap_or(false) {
            permissions |= serenity::Permissions::VIEW_CHANNEL;
        }
        if self.send_messages.unwrap_or(false) {
            permissions |= serenity::Permissions::SEND_MESSAGES;
        }
        if self.read_message_history.unwrap_or(false) {
            permissions |= serenity::Permissions::READ_MESSAGE_HISTORY;
        }
        if self.manage_messages.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_MESSAGES;
        }
        if self.manage_channels.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_CHANNELS;
        }
        if self.add_reactions.unwrap_or(false) {
            permissions |= serenity::Permissions::ADD_REACTIONS;
        }
        if self.use_external_emojis.unwrap_or(false) {
            permissions |= serenity::Permissions::USE_EXTERNAL_EMOJIS;
        }
        if self.embed_links.unwrap_or(false) {
            permissions |= serenity::Permissions::EMBED_LINKS;
        }
        if self.attach_files.unwrap_or(false) {
            permissions |= serenity::Permissions::ATTACH_FILES;
        }
        if self.connect.unwrap_or(false) {
            permissions |= serenity::Permissions::CONNECT;
        }
        if self.speak.unwrap_or(false) {
            permissions |= serenity::Permissions::SPEAK;
        }
        if self.mute_members.unwrap_or(false) {
            permissions |= serenity::Permissions::MUTE_MEMBERS;
        }
        if self.move_members.unwrap_or(false) {
            permissions |= serenity::Permissions::MOVE_MEMBERS;
        }
        if self.administrator.unwrap_or(false) {
            permissions |= serenity::Permissions::ADMINISTRATOR;
        }
        if self.kick_members.unwrap_or(false) {
            permissions |= serenity::Permissions::KICK_MEMBERS;
        }
        if self.ban_members.unwrap_or(false) {
            permissions |= serenity::Permissions::BAN_MEMBERS;
        }
        if self.manage_guild.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_GUILD;
        }
        if self.manage_roles.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_ROLES;
        }
        if self.manage_webhooks.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_WEBHOOKS;
        }
        if self.manage_emojis_and_stickers.unwrap_or(false) {
            permissions |= serenity::Permissions::MANAGE_EMOJIS_AND_STICKERS;
        }

        permissions
    }
}

// Use a different name to avoid conflicts with your existing RoleConfig
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct DiscordRoleConfig {
    pub role: String,
    // #[serde(skip_serializing_if = "Option::is_none")]
    // pub category: Option<String>,
    // TODO: convert to option.
    pub category: String,
    #[serde(default)]
    pub channels: Vec<ChannelPermission>,
    #[serde(default, skip_serializing_if = "is_default_permissions")]
    pub permissions: RolePermissions,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,
    #[serde(default = "default_true", skip_serializing_if = "is_true")]
    pub mentionable: bool,
    #[serde(default, skip_serializing_if = "is_false")]
    pub hoist: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u16>,
}

// Conversion method to your existing PermissionRoleConfig
// impl DiscordRoleConfig {
//     pub fn to_permission_role_config(&self) -> RoleConfig {
//         RoleConfig {
//             role: self.role.clone(),
//             category: self.category.clone(),//.unwrap_or_default(),
//             channels: self.channels.iter().map(|ch| {
//                 crate::permissions::ChannelPermissionConfig {
//                     name: ch.name.clone(),
//                     permission: match ch.permission {
//                         PermissionType::Read => crate::permissions::ChannelPermissionType::Read,
//                         PermissionType::ReadWrite => crate::permissions::ChannelPermissionType::ReadWrite,
//                         PermissionType::Admin => crate::permissions::ChannelPermissionType::Admin,
//                         PermissionType::None => crate::permissions::ChannelPermissionType::None,
//                     }
//                 }
//             }).collect(),
//         }
//     }
// }

fn default_true() -> bool {
    true
}

fn is_true(b: &bool) -> bool {
    *b
}

fn is_false(b: &bool) -> bool {
    !*b
}

fn is_default_permissions(perms: &RolePermissions) -> bool {
    perms.view_channel.is_none()
        && perms.send_messages.is_none()
        && perms.read_message_history.is_none()
        && perms.manage_messages.is_none()
        && perms.manage_channels.is_none()
        && perms.add_reactions.is_none()
        && perms.use_external_emojis.is_none()
        && perms.embed_links.is_none()
        && perms.attach_files.is_none()
        && perms.connect.is_none()
        && perms.speak.is_none()
        && perms.mute_members.is_none()
        && perms.move_members.is_none()
        && perms.administrator.is_none()
        && perms.kick_members.is_none()
        && perms.ban_members.is_none()
        && perms.manage_guild.is_none()
        && perms.manage_roles.is_none()
        && perms.manage_webhooks.is_none()
        && perms.manage_emojis_and_stickers.is_none()
}

#[derive(Debug, Clone)]
pub struct RoleManager {
    roles_config: Vec<DiscordRoleConfig>,
    config_path: Option<std::path::PathBuf>,
}

impl RoleManager {
    /// Create a new RoleManager from a JSON file
    pub async fn from_file<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_buf = path.as_ref().to_path_buf();
        let content = tokio::fs::read_to_string(&path_buf)
            .await
            .context("Failed to read roles configuration file")?;
        
        let roles_config: Vec<DiscordRoleConfig> = serde_json::from_str(&content)
            .context("Failed to parse roles configuration JSON").unwrap();

        info!("Loaded {} role configurations from {:?}", roles_config.len(), path_buf);

        Ok(Self { 
            roles_config,
            config_path: Some(path_buf),
        })
    }

    /// Reload configuration from file if available
    pub async fn reload_config(&mut self) -> Result<bool> {
        if let Some(ref path) = self.config_path {
            info!("Reloading role configuration from {:?}", path);
            
            let content = tokio::fs::read_to_string(path)
                .await
                .context("Failed to read roles configuration file")?;
            
            let new_config: Vec<DiscordRoleConfig> = serde_json::from_str(&content)
                .context("Failed to parse roles configuration JSON").unwrap();

            self.roles_config = new_config;
            info!("Successfully reloaded {} role configurations", self.roles_config.len());
            Ok(true)
        } else {
            warn!("No config file path available for reload");
            Ok(false)
        }
    }

    /// Get all role configurations
    pub fn get_role_configs(&self) -> &[DiscordRoleConfig] {
        &self.roles_config
    }

    /// Get a specific role configuration by name
    pub fn get_role_config(&self, role_name: &str) -> Option<&DiscordRoleConfig> {
        self.roles_config
            .iter()
            .find(|config| config.role.eq_ignore_ascii_case(role_name))
    }

    /// Create all roles defined in the configuration for a specific guild
    pub async fn create_roles_in_guild(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
    ) -> Result<HashMap<String, serenity::RoleId>> {
        info!("Creating roles for guild {}", guild_id);

        let guild = guild_id
            .to_partial_guild(http)
            .await
            .context("Failed to fetch guild information")?;

        let existing_roles = self.get_existing_roles(&guild).await;
        let mut created_roles = HashMap::new();

        for role_config in &self.roles_config {
            let role_id = if let Some(existing_role_id) = existing_roles.get(&role_config.role) {
                info!("Role '{}' already exists, skipping creation", role_config.role);
                *existing_role_id
            } else {
                info!("Creating role: {}", role_config.role);
                self.create_single_role(http, guild_id, role_config).await?
            };

            created_roles.insert(role_config.role.clone(), role_id);
        }

        info!("Successfully processed {} roles for guild {}", 
              created_roles.len(), guild_id);

        Ok(created_roles)
    }

    /// Create a single role in the guild
    async fn create_single_role(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        role_config: &DiscordRoleConfig,
    ) -> Result<serenity::RoleId> {
        let role_permissions = role_config.permissions.to_serenity_permissions();
        let role_color = self.parse_color(&role_config.color);

        let builder = serenity::EditRole::new()
            .name(&role_config.role)
            .permissions(role_permissions)
            .colour(role_color)
            .mentionable(role_config.mentionable)
            .hoist(role_config.hoist);

        let builder = if let Some(position) = role_config.position {
            builder.position(position)
        } else {
            builder
        };

        let role = guild_id
            .create_role(http, builder)
            .await
            .context(format!("Failed to create role '{}'", role_config.role))?;

        info!("Successfully created role '{}' with ID {}", 
              role_config.role, role.id);

        Ok(role.id)
    }

    /// Parse color from string (hex, rgb, or named colors)
    fn parse_color(&self, color_str: &Option<String>) -> serenity::Colour {
        match color_str {
            Some(color) => {
                let color = color.trim().to_lowercase();
                
                // Handle hex colors
                if color.starts_with('#') {
                    if let Ok(hex_val) = u32::from_str_radix(&color[1..], 16) {
                        return serenity::Colour::new(hex_val);
                    }
                } else if color.starts_with("0x") {
                    if let Ok(hex_val) = u32::from_str_radix(&color[2..], 16) {
                        return serenity::Colour::new(hex_val);
                    }
                }
                
                // Handle named colors
                match color.as_str() {
                    "red" => serenity::Colour::RED,
                    "green" => serenity::Colour::DARK_GREEN,
                    "blue" => serenity::Colour::BLUE,
                    "yellow" => serenity::Colour::GOLD,
                    "orange" => serenity::Colour::ORANGE,
                    "purple" => serenity::Colour::PURPLE,
                    "pink" => serenity::Colour::MAGENTA,
                    "cyan" => serenity::Colour::TEAL,
                    "white" => serenity::Colour::LIGHTER_GREY,
                    "black" => serenity::Colour::DARKER_GREY,
                    "grey" | "gray" => serenity::Colour::LIGHT_GREY,
                    "dark_red" => serenity::Colour::DARK_RED,
                    "dark_green" => serenity::Colour::DARK_GREEN,
                    "dark_blue" => serenity::Colour::DARK_BLUE,
                    "dark_purple" => serenity::Colour::DARK_PURPLE,
                    "dark_orange" => serenity::Colour::DARK_ORANGE,
                    _ => {
                        warn!("Unknown color '{}', using default", color);
                        serenity::Colour::default()
                    }
                }
            }
            None => serenity::Colour::default(),
        }
    }

    /// Get existing roles in the guild
    async fn get_existing_roles(&self, guild: &serenity::PartialGuild) -> HashMap<String, serenity::RoleId> {
        let mut existing_roles = HashMap::new();
        
        for role in guild.roles.values() {
            existing_roles.insert(role.name.clone(), role.id);
        }

        existing_roles
    }

    /// Update existing roles with new configurations
    pub async fn update_roles_in_guild(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
    ) -> Result<HashMap<String, serenity::RoleId>> {
        info!("Updating roles for guild {}", guild_id);

        let guild = guild_id
            .to_partial_guild(http)
            .await
            .context("Failed to fetch guild information")?;

        let existing_roles = self.get_existing_roles(&guild).await;
        let mut updated_roles = HashMap::new();

        for role_config in &self.roles_config {
            let role_id = if let Some(&existing_role_id) = existing_roles.get(&role_config.role) {
                info!("Updating existing role: {}", role_config.role);
                self.update_single_role(http, guild_id, existing_role_id, role_config).await?;
                existing_role_id
            } else {
                info!("Creating new role: {}", role_config.role);
                self.create_single_role(http, guild_id, role_config).await?
            };

            updated_roles.insert(role_config.role.clone(), role_id);
        }

        info!("Successfully updated {} roles for guild {}", 
              updated_roles.len(), guild_id);

        Ok(updated_roles)
    }

    /// Update a single existing role
    async fn update_single_role(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        role_id: serenity::RoleId,
        role_config: &DiscordRoleConfig,
    ) -> Result<()> {
        let role_permissions = role_config.permissions.to_serenity_permissions();
        let role_color = self.parse_color(&role_config.color);

        let builder = serenity::EditRole::new()
            .name(&role_config.role)
            .permissions(role_permissions)
            .colour(role_color)
            .mentionable(role_config.mentionable)
            .hoist(role_config.hoist);

        let builder = if let Some(position) = role_config.position {
            builder.position(position)
        } else {
            builder
        };

        guild_id
            .edit_role(http, role_id, builder)
            .await
            .context(format!("Failed to update role '{}'", role_config.role))?;

        info!("Successfully updated role '{}' with ID {}", 
              role_config.role, role_id);

        Ok(())
    }

    /// Delete roles that are not in the current configuration
    pub async fn cleanup_unused_roles(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        managed_role_prefix: Option<&str>,
    ) -> Result<Vec<String>> {
        info!("Cleaning up unused roles for guild {}", guild_id);

        let guild = guild_id
            .to_partial_guild(http)
            .await
            .context("Failed to fetch guild information")?;

        let configured_roles: std::collections::HashSet<String> = self
            .roles_config
            .iter()
            .map(|config| config.role.clone())
            .collect();

        let mut deleted_roles = Vec::new();

        for role in guild.roles.values() {
            // Skip @everyone role
            if role.name == "@everyone" {
                continue;
            }

            // If prefix is specified, only manage roles with that prefix
            if let Some(prefix) = managed_role_prefix {
                if !role.name.starts_with(prefix) {
                    continue;
                }
            }

            // Delete role if it's not in our configuration
            if !configured_roles.contains(&role.name) {
                info!("Deleting unused role: {}", role.name);
                
                if let Err(e) = guild_id.delete_role(http, role.id).await {
                    warn!("Failed to delete role '{}': {}", role.name, e);
                } else {
                    deleted_roles.push(role.name.clone());
                }
            }
        }

        info!("Deleted {} unused roles from guild {}", 
              deleted_roles.len(), guild_id);

        Ok(deleted_roles)
    }
}

// Type alias for easier integration with your existing code
pub type SharedRoleManager = Arc<RwLock<RoleManager>>;

/// Helper function to create a shared RoleManager
pub async fn create_shared_role_manager<P: AsRef<Path>>(path: P) -> Result<SharedRoleManager> {
    let role_manager = RoleManager::from_file(path).await?;
    Ok(Arc::new(RwLock::new(role_manager)))
}