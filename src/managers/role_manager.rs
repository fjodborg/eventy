use poise::serenity_prelude::{self as serenity, Colour, GuildId, Http, RoleId, UserId};
use std::collections::HashMap;
use std::sync::Arc;
use tracing::{debug, error, info, warn};

use crate::config::RoleDefinition;
use crate::error::{BotError, Result};
use crate::state::{EntityType, SharedChannelState};

/// Manages Discord role creation and assignment
pub struct RoleManager {
    /// Channel state for caching role IDs
    state: SharedChannelState,
}

impl RoleManager {
    pub fn new(state: SharedChannelState) -> Self {
        Self { state }
    }

    /// Ensure all roles from the global structure exist in the guild
    pub async fn ensure_roles_exist(
        &self,
        http: &Http,
        guild_id: GuildId,
        roles: &[RoleDefinition],
    ) -> Result<HashMap<String, RoleId>> {
        let mut created_roles = HashMap::new();

        // Get existing roles in the guild
        let existing_roles = guild_id.roles(http).await?;

        for role_def in roles {
            // Check if role already exists
            if let Some((role_id, _)) = existing_roles.iter().find(|(_, r)| r.name == role_def.name)
            {
                debug!("Role '{}' already exists", role_def.name);
                created_roles.insert(role_def.name.clone(), *role_id);

                // Update state
                let mut state = self.state.write().await;
                let guild = state.get_guild_mut(&guild_id.to_string(), "");
                guild.add_role(
                    &role_def.name,
                    &role_id.to_string(),
                    role_def.color.as_deref(),
                );

                continue;
            }

            // Create the role
            match self.create_role(http, guild_id, role_def).await {
                Ok(role_id) => {
                    info!("Created role '{}' with ID {}", role_def.name, role_id);
                    created_roles.insert(role_def.name.clone(), role_id);

                    // Update state
                    let mut state = self.state.write().await;
                    let guild = state.get_guild_mut(&guild_id.to_string(), "");
                    guild.add_role(
                        &role_def.name,
                        &role_id.to_string(),
                        role_def.color.as_deref(),
                    );
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Missing Permissions") || err_str.contains("50013") {
                        error!(
                            "{}: Role hierarchy issue - move bot's role above '{}' in Discord server settings",
                            role_def.name, role_def.name
                        );
                    } else {
                        error!("Failed to create role '{}': {}", role_def.name, e);
                    }
                }
            }
        }

        Ok(created_roles)
    }

    /// Create a single role
    async fn create_role(
        &self,
        http: &Http,
        guild_id: GuildId,
        role_def: &RoleDefinition,
    ) -> Result<RoleId> {
        let color = role_def
            .color
            .as_ref()
            .and_then(|c| parse_hex_color(c))
            .unwrap_or(Colour::default());

        let role = guild_id
            .create_role(
                http,
                serenity::EditRole::new()
                    .name(&role_def.name)
                    .colour(color)
                    .hoist(role_def.hoist)
                    .mentionable(role_def.mentionable),
            )
            .await?;

        Ok(role.id)
    }

    /// Get role ID by name from cache or fetch from Discord
    pub async fn get_role_id(
        &self,
        http: &Http,
        guild_id: GuildId,
        role_name: &str,
    ) -> Result<RoleId> {
        // Check cache first
        {
            let state = self.state.write().await;
            if let Some(guild) = state.get_guild(&guild_id.to_string()) {
                if let Some(id_str) = guild.get_role_id(role_name) {
                    if let Ok(id) = id_str.parse::<u64>() {
                        return Ok(RoleId::new(id));
                    }
                }
            }
        }

        // Fetch from Discord
        let roles = guild_id.roles(http).await?;
        for (role_id, role) in roles {
            if role.name == role_name {
                // Update cache
                let mut state = self.state.write().await;
                let guild = state.get_guild_mut(&guild_id.to_string(), "");
                guild.add_role(role_name, &role_id.to_string(), None);

                return Ok(role_id);
            }
        }

        Err(BotError::RoleNotFound {
            name: role_name.to_string(),
        })
    }

    /// Assign a role to a user
    pub async fn assign_role_to_user(
        &self,
        http: &Http,
        guild_id: GuildId,
        user_id: UserId,
        role_name: &str,
    ) -> Result<()> {
        let role_id = self.get_role_id(http, guild_id, role_name).await?;

        let member = guild_id.member(http, user_id).await?;
        member.add_role(http, role_id).await?;

        info!("Assigned role '{}' to user {}", role_name, user_id);
        Ok(())
    }

    /// Assign multiple roles to a user
    pub async fn assign_roles_to_user(
        &self,
        http: &Http,
        guild_id: GuildId,
        user_id: UserId,
        role_names: &[String],
    ) -> Result<Vec<String>> {
        let mut assigned = Vec::new();

        for role_name in role_names {
            match self
                .assign_role_to_user(http, guild_id, user_id, role_name)
                .await
            {
                Ok(()) => {
                    assigned.push(role_name.clone());
                }
                Err(e) => {
                    warn!(
                        "Failed to assign role '{}' to user {}: {}",
                        role_name, user_id, e
                    );
                }
            }
        }

        Ok(assigned)
    }

    /// Remove a role from a user
    pub async fn remove_role_from_user(
        &self,
        http: &Http,
        guild_id: GuildId,
        user_id: UserId,
        role_name: &str,
    ) -> Result<()> {
        let role_id = self.get_role_id(http, guild_id, role_name).await?;

        let member = guild_id.member(http, user_id).await?;
        member.remove_role(http, role_id).await?;

        info!("Removed role '{}' from user {}", role_name, user_id);
        Ok(())
    }

    /// Check if a role exists in the guild
    pub async fn role_exists(&self, http: &Http, guild_id: GuildId, role_name: &str) -> bool {
        self.get_role_id(http, guild_id, role_name).await.is_ok()
    }

    /// Check if we need to sync a role
    pub async fn needs_sync(&self, guild_id: GuildId, role_name: &str) -> bool {
        let state = self.state.read().await;
        state.needs_sync(&guild_id.to_string(), EntityType::Role, role_name)
    }

    /// Sync assignment roles for a single user based on their Discord username
    /// Returns (roles_added, roles_failed)
    pub async fn sync_assignments_for_user(
        &self,
        http: &Http,
        guild_id: GuildId,
        user_id: UserId,
        discord_username: &str,
        special_roles: &[String],
    ) -> (Vec<String>, Vec<String>) {
        let mut added = Vec::new();
        let mut failed = Vec::new();

        if special_roles.is_empty() {
            debug!(
                "No special roles to assign for user '{}' ({})",
                discord_username, user_id
            );
            return (added, failed);
        }

        info!(
            "Syncing {} assignment roles for '{}' ({}): {:?}",
            special_roles.len(),
            discord_username,
            user_id,
            special_roles
        );

        // Get the member to check existing roles
        let member = match guild_id.member(http, user_id).await {
            Ok(m) => m,
            Err(e) => {
                error!(
                    "Failed to get member {} in guild {}: {}",
                    user_id, guild_id, e
                );
                for role in special_roles {
                    failed.push(role.clone());
                }
                return (added, failed);
            }
        };

        for role_name in special_roles {
            // Get role ID
            let role_id = match self.get_role_id(http, guild_id, role_name).await {
                Ok(id) => id,
                Err(e) => {
                    error!(
                        "Role '{}' not found in guild {}: {}. Make sure to sync roles first.",
                        role_name, guild_id, e
                    );
                    failed.push(role_name.clone());
                    continue;
                }
            };

            // Check if user already has this role
            if member.roles.contains(&role_id) {
                debug!(
                    "User {} already has role '{}', skipping",
                    user_id, role_name
                );
                continue;
            }

            // Assign the role
            match member.add_role(http, role_id).await {
                Ok(_) => {
                    info!(
                        "Assigned role '{}' to '{}' ({})",
                        role_name, discord_username, user_id
                    );
                    added.push(role_name.clone());
                }
                Err(e) => {
                    let err_str = e.to_string();
                    if err_str.contains("Missing Permissions") || err_str.contains("50013") {
                        error!(
                            "Failed to assign '{}' to {}: Role hierarchy issue - move bot's role above '{}'",
                            role_name, user_id, role_name
                        );
                    } else {
                        error!(
                            "Failed to assign role '{}' to {}: {}",
                            role_name, user_id, e
                        );
                    }
                    failed.push(role_name.clone());
                }
            }
        }

        (added, failed)
    }

    /// Full sync of assignment roles for a user - adds missing roles AND removes roles they shouldn't have
    /// `desired_roles` - roles the user SHOULD have according to assignments.json
    /// `all_assignment_roles` - all roles that are managed by assignments.json (used to know which roles to potentially remove)
    /// Returns (roles_added, roles_removed, roles_failed)
    pub async fn full_sync_assignments_for_user(
        &self,
        http: &Http,
        guild_id: GuildId,
        user_id: UserId,
        discord_username: &str,
        desired_roles: &[String],
        all_assignment_roles: &[String],
    ) -> (Vec<String>, Vec<String>, Vec<String>) {
        let mut added = Vec::new();
        let mut removed = Vec::new();
        let mut failed = Vec::new();

        info!(
            "Full sync for '{}' ({}): desired={:?}, managed={:?}",
            discord_username, user_id, desired_roles, all_assignment_roles
        );

        // Get the member to check existing roles
        let member = match guild_id.member(http, user_id).await {
            Ok(m) => m,
            Err(e) => {
                error!(
                    "Failed to get member {} in guild {}: {}",
                    user_id, guild_id, e
                );
                return (added, removed, failed);
            }
        };

        // Build a map of role_name -> role_id for all managed roles
        let mut managed_role_ids: HashMap<String, RoleId> = HashMap::new();
        for role_name in all_assignment_roles {
            if let Ok(role_id) = self.get_role_id(http, guild_id, role_name).await {
                managed_role_ids.insert(role_name.clone(), role_id);
            }
        }

        // Add missing roles
        for role_name in desired_roles {
            let role_id = match managed_role_ids.get(role_name) {
                Some(id) => *id,
                None => {
                    // Try to get it if not in managed list
                    match self.get_role_id(http, guild_id, role_name).await {
                        Ok(id) => id,
                        Err(e) => {
                            error!(
                                "Role '{}' not found in guild {}: {}. Make sure to sync roles first.",
                                role_name, guild_id, e
                            );
                            failed.push(format!("add:{}", role_name));
                            continue;
                        }
                    }
                }
            };

            if !member.roles.contains(&role_id) {
                match member.add_role(http, role_id).await {
                    Ok(_) => {
                        info!(
                            "Assigned role '{}' to '{}' ({})",
                            role_name, discord_username, user_id
                        );
                        added.push(role_name.clone());
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("Missing Permissions") || err_str.contains("50013") {
                            error!(
                                "Failed to assign '{}' to {}: Role hierarchy issue - move bot's role above '{}'",
                                role_name, user_id, role_name
                            );
                        } else {
                            error!(
                                "Failed to assign role '{}' to {}: {}",
                                role_name, user_id, e
                            );
                        }
                        failed.push(format!("add:{}", role_name));
                    }
                }
            }
        }

        // Remove roles they shouldn't have (only managed roles)
        for (role_name, role_id) in &managed_role_ids {
            // Skip if this role is in desired_roles
            if desired_roles.iter().any(|r| r == role_name) {
                continue;
            }

            // Check if user has this role
            if member.roles.contains(role_id) {
                match member.remove_role(http, *role_id).await {
                    Ok(_) => {
                        info!(
                            "Removed role '{}' from '{}' ({})",
                            role_name, discord_username, user_id
                        );
                        removed.push(role_name.clone());
                    }
                    Err(e) => {
                        let err_str = e.to_string();
                        if err_str.contains("Missing Permissions") || err_str.contains("50013") {
                            error!(
                                "Failed to remove '{}' from {}: Role hierarchy issue - move bot's role above '{}'",
                                role_name, user_id, role_name
                            );
                        } else {
                            error!(
                                "Failed to remove role '{}' from {}: {}",
                                role_name, user_id, e
                            );
                        }
                        failed.push(format!("remove:{}", role_name));
                    }
                }
            }
        }

        (added, removed, failed)
    }
}

/// Parse a hex color string to Colour
fn parse_hex_color(hex: &str) -> Option<Colour> {
    let hex = hex.trim_start_matches('#');
    u32::from_str_radix(hex, 16).ok().map(Colour::new)
}

/// Shared role manager type
pub type SharedRoleManager = Arc<tokio::sync::RwLock<RoleManager>>;

pub fn create_shared_role_manager(state: SharedChannelState) -> SharedRoleManager {
    Arc::new(tokio::sync::RwLock::new(RoleManager::new(state)))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_hex_color() {
        assert_eq!(parse_hex_color("#ff0000"), Some(Colour::new(0xff0000)));
        assert_eq!(parse_hex_color("00ff00"), Some(Colour::new(0x00ff00)));
        assert_eq!(parse_hex_color("#2ecc71"), Some(Colour::new(0x2ecc71)));
    }
}
