use anyhow::Result;
use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use std::sync::Arc;
use tracing::{debug, info, warn};

use super::config::PermissionConfig;
use super::types::*;

use crate::role_manager::DiscordRoleConfig as RoleConfig;

#[derive(Debug)]
pub struct PermissionManager {
    config: PermissionConfig,
    active_grants: Arc<DashMap<serenity::UserId, Vec<PermissionGrant>>>,
}

impl PermissionManager {
    pub async fn new() -> Result<Self> {
        let config = PermissionConfig::load().await?;

        Ok(Self {
            config,
            active_grants: Arc::new(DashMap::new()),
        })
    }
    // Add this method to your existing PermissionManager
    pub fn add_role_config(&mut self, config: RoleConfig) {
        // Update or add the role configuration
        if let Some(existing_config) = self
            .config
            .roles
            .iter_mut()
            .find(|c| c.role.eq_ignore_ascii_case(&config.role))
        {
            *existing_config = config;
        } else {
            self.config.roles.push(config);
        }
    }
    pub fn get_config(&self) -> &PermissionConfig {
        &self.config
    }

    pub async fn reload_config(&mut self) -> Result<()> {
        info!("Reloading permission configuration");
        self.config = PermissionConfig::load().await?;
        Ok(())
    }

    pub fn get_user_grants(&self, user_id: serenity::UserId) -> Vec<PermissionGrant> {
        self.active_grants
            .get(&user_id)
            .map(|grants| grants.clone())
            .unwrap_or_default()
    }

    fn find_role_by_name(
        &self,
        guild_roles: &std::collections::HashMap<serenity::RoleId, serenity::Role>,
        role_name: &str,
    ) -> Option<serenity::RoleId> {
        guild_roles
            .iter()
            .find(|(_, role)| role.name.eq_ignore_ascii_case(role_name))
            .map(|(role_id, _)| *role_id)
    }

    // TODO: utilize revoke permissions.
    pub fn revoke_permission(&self, user_id: serenity::UserId, role_name: &str) -> bool {
        if let Some(mut grants) = self.active_grants.get_mut(&user_id) {
            if let Some(pos) = grants.iter().position(|g| g.role_config.role == role_name) {
                grants.remove(pos);
                info!("Revoked permission for user {} role {}", user_id, role_name);
                return true;
            }
        }
        false
    }

    // TODO: I keep getting server warnings that category doesn't exist on a different server.
    // Perhaps this method should be unique to each server so we don't run the code for other servers when applying permission.
    pub async fn apply_permissions_to_guild(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        user_id: serenity::UserId,
        grants: &[PermissionGrant],
    ) -> Result<()> {
        info!(
            "Applying {} permission grants for user {} in guild {}",
            grants.len(),
            user_id,
            guild_id
        );

        let channels = guild_id.channels(http).await?;
        let guild_roles = guild_id.roles(http).await?;
        let mut categories_found = std::collections::HashMap::new();
        let mut channels_by_category = std::collections::HashMap::new();

        // Map channels and categories
        for (channel_id, channel) in &channels {
            match channel.kind {
                serenity::ChannelType::Category => {
                    categories_found.insert(channel.name.clone(), *channel_id);
                }
                serenity::ChannelType::Text => {
                    if let Some(parent_id) = channel.parent_id {
                        channels_by_category
                            .entry(parent_id)
                            .or_insert_with(Vec::new)
                            .push((*channel_id, channel.name.clone()));
                    }
                }
                _ => {}
            }
        }

        // Apply permissions for each grant
        for grant in grants {
            debug!(
                "Applying permissions for role: {} in category: {}",
                grant.role_config.role, grant.role_config.category
            );
            // 1. Assign Discord role if it exists
            if let Some(role_id) = self.find_role_by_name(&guild_roles, &grant.role_config.role) {
                if let Err(e) = guild_id
                    .member(http, user_id)
                    .await?
                    .add_role(http, role_id)
                    .await
                {
                    warn!(
                        "Failed to assign role '{}' to user {}: {}",
                        grant.role_config.role, user_id, e
                    );
                } else {
                    info!(
                        "Successfully assigned role '{}' to user {}",
                        grant.role_config.role, user_id
                    );
                }
            } else {
                warn!(
                    "Role '{}' not found in guild {}",
                    grant.role_config.role, guild_id
                );
            }

            // TODO: avoid nested options. Perhaps make utility methods for this.
            // 2. Apply channel permissions (existing logic)
            if let Some(&category_id) = categories_found.get(&grant.role_config.category) {
                // Grant category access
                let category_permissions = serenity::PermissionOverwrite {
                    allow: serenity::Permissions::VIEW_CHANNEL,
                    deny: serenity::Permissions::empty(),
                    kind: serenity::PermissionOverwriteType::Member(user_id),
                };

                if let Err(e) = category_id
                    .create_permission(http, category_permissions)
                    .await
                {
                    warn!("Failed to grant category access: {}", e);
                }

                // Apply channel permissions
                if let Some(category_channels) = channels_by_category.get(&category_id) {
                    for (channel_id, channel_name) in category_channels {
                        if let Some(channel_config) = grant
                            .role_config
                            .channels
                            .iter()
                            .find(|c| c.name.eq_ignore_ascii_case(channel_name))
                        {
                            let (allow, deny) = channel_config.permission.to_serenity_permissions();
                            let permissions = serenity::PermissionOverwrite {
                                allow,
                                deny,
                                kind: serenity::PermissionOverwriteType::Member(user_id),
                            };

                            if let Err(e) = channel_id.create_permission(http, permissions).await {
                                warn!(
                                    "Failed to apply channel permissions to {}: {}",
                                    channel_name, e
                                );
                            } else {
                                info!(
                                    "Applied {:?} permissions to channel: {}",
                                    channel_config.permission, channel_name
                                );
                            }
                        }
                    }
                }
            } else {
                warn!(
                    "Category '{}' not found in guild {}",
                    grant.role_config.category, guild_id
                );
            }
        }

        Ok(())
    }
}
