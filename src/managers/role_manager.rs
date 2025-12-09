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
                    error!("Failed to create role '{}': {}", role_def.name, e);
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
