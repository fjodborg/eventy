use anyhow::{Error, Result};
use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use std::{
    sync::Arc,
    time::{SystemTime, UNIX_EPOCH},
};
use tokio::fs;
use tracing::{debug, error, info, warn};

use crate::models::{PendingVerification, UserData, UserDatabase};

const USER_DATABASE_FILE: &str = "data/users.json";

#[derive(Debug)]
pub struct UserManager {
    pub database: Arc<DashMap<String, UserData>>,
    pub pending_verifications: Arc<DashMap<serenity::UserId, PendingVerification>>,
    pub verified_users: Arc<DashMap<serenity::UserId, String>>, // Discord ID -> User ID mapping
}

impl UserManager {
    pub fn new() -> Self {
        debug!("Creating new UserManager instance");
        Self {
            database: Arc::new(DashMap::new()),
            pending_verifications: Arc::new(DashMap::new()),
            verified_users: Arc::new(DashMap::new()),
        }
    }

    pub async fn load_database(&self) -> Result<()> {
        debug!("Loading user database from: {}", USER_DATABASE_FILE);

        let path = std::path::Path::new(USER_DATABASE_FILE);
        if !path.exists() {
            warn!(
                "User database file not found at {}, starting with empty database",
                USER_DATABASE_FILE
            );
            return Ok(());
        }

        match fs::read_to_string(USER_DATABASE_FILE).await {
            Ok(content) => {
                info!("Successfully read database file, parsing JSON...");

                let user_database: UserDatabase = serde_json::from_str(&content).unwrap();
                match user_database {
                    Ok(user_db) => {
                        for user in user_db.users {
                            let id = user.id.clone();
                            debug!("Loading user: {} ({})", user.name, user.id);
                            self.database.insert(id, user);
                        }
                        info!(
                            "Successfully loaded {} users from database",
                            self.database.len()
                        );
                    }
                    Err(e) => {
                        error!("Failed to parse user database JSON: {}", e);
                        return Err(e.into());
                    }
                }
            }
            Err(e) => {
                error!("Failed to read user database file: {}", e);
                return Err(e.into());
            }
        }

        Ok(())
    }

    pub fn find_user_by_id(&self, id: &str) -> Option<UserData> {
        debug!("Looking up user by ID: {}", id);
        debug!("available ids {:?}", self.database);
        let result = self.database.get(id).map(|entry| entry.value().clone());
        match &result {
            Some(user) => debug!("Found user: {} for ID: {}", user.name, id),
            None => debug!("No user found for ID: {}", id),
        }
        result
    }

    pub fn is_user_verified(&self, discord_id: serenity::UserId) -> bool {
        let is_verified = self.verified_users.contains_key(&discord_id);
        debug!(
            "Checking if user {} is verified: {}",
            discord_id, is_verified
        );
        is_verified
    }

    pub fn get_verified_user_data(&self, discord_id: serenity::UserId) -> Option<UserData> {
        debug!("Getting verified user data for: {}", discord_id);
        if let Some(user_id) = self.verified_users.get(&discord_id) {
            let user_data = self.find_user_by_id(&user_id);
            match &user_data {
                Some(data) => debug!(
                    "Found verified user data: {} for discord ID: {}",
                    data.name, discord_id
                ),
                None => warn!(
                    "User {} marked as verified but no data found for ID: {}",
                    discord_id,
                    user_id.value()
                ),
            }
            user_data
        } else {
            debug!("No verified user data found for: {}", discord_id);
            None
        }
    }

    pub fn add_pending_verification(
        &self,
        user_id: serenity::UserId,
        channel_id: serenity::ChannelId,
    ) {
        debug!(
            "Adding pending verification for user {} in channel {}",
            user_id, channel_id
        );
        let verification = PendingVerification {
            user_id,
            channel_id,
            timestamp: SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .unwrap()
                .as_secs(),
        };
        self.pending_verifications.insert(user_id, verification);
        info!("Added pending verification for user: {}", user_id);
    }

    pub fn complete_verification(&self, discord_id: serenity::UserId, user_id: String) {
        debug!(
            "Completing verification for discord ID {} with user ID {}",
            discord_id, user_id
        );
        self.verified_users.insert(discord_id, user_id.clone());
        self.pending_verifications.remove(&discord_id);
        info!(
            "Completed verification for discord ID {} -> user ID {}",
            discord_id, user_id
        );
    }

    pub fn is_pending_verification(&self, discord_id: serenity::UserId) -> bool {
        let is_pending = self.pending_verifications.contains_key(&discord_id);
        debug!(
            "Checking if user {} has pending verification: {}",
            discord_id, is_pending
        );
        is_pending
    }

    pub fn remove_pending_verification(&self, discord_id: serenity::UserId) {
        debug!("Removing pending verification for user: {}", discord_id);
        if self.pending_verifications.remove(&discord_id).is_some() {
            info!("Removed pending verification for user: {}", discord_id);
        } else {
            warn!(
                "Tried to remove pending verification for user {} but none found",
                discord_id
            );
        }
    }

    pub async fn update_user_in_guild(
        &self,
        http: &serenity::Http,
        cache: &serenity::Cache,
        guild_id: serenity::GuildId,
        discord_id: serenity::UserId,
        user_data: &UserData,
    ) -> Result<()> {
        info!(
            "Updating user {} ({}) in guild {}",
            user_data.name, discord_id, guild_id
        );

        // Extract bot user ID from cache first
        let bot_user_id = cache.current_user().id;
        debug!("Bot user ID: {}", bot_user_id);

        // Get guild and members
        let guild = match guild_id.to_partial_guild(http).await {
            Ok(guild) => {
                debug!("Successfully fetched guild: {}", guild.name);
                guild
            }
            Err(e) => {
                error!("Failed to fetch guild {}: {}", guild_id, e);
                return Err(e.into());
            }
        };

        let bot_member = match guild_id.member(http, bot_user_id).await {
            Ok(member) => {
                debug!("Successfully fetched bot member");
                member
            }
            Err(e) => {
                error!("Failed to fetch bot member in guild {}: {}", guild_id, e);
                return Err(e.into());
            }
        };

        let target_member = match guild_id.member(http, discord_id).await {
            Ok(member) => {
                debug!("Successfully fetched target member");
                member
            }
            Err(e) => {
                error!(
                    "Failed to fetch target member {} in guild {}: {}",
                    discord_id, guild_id, e
                );
                return Err(e.into());
            }
        };

        // Check if bot has permission to manage nicknames
        let bot_permissions = guild.member_permissions(&bot_member);
        debug!("Bot permissions: {:?}", bot_permissions);

        if !bot_permissions.manage_nicknames() {
            error!("Missing permission: MANAGE_NICKNAMES - Cannot update user nicknames");
        } else {
            debug!("Bot has MANAGE_NICKNAMES permission");
            // Check role hierarchy using cached guild if available
            let can_modify = if let Some(cached_guild) = guild_id.to_guild_cached(cache) {
                let bot_highest = cached_guild.member_highest_role(&bot_member);
                let target_highest = cached_guild.member_highest_role(&target_member);

                match (bot_highest, target_highest) {
                    (Some(bot_role), Some(target_role)) => {
                        debug!(
                            "Bot highest role: {} (pos: {}), Target highest role: {} (pos: {})",
                            bot_role.name,
                            bot_role.position,
                            target_role.name,
                            target_role.position
                        );
                        bot_role.position > target_role.position
                    }
                    (Some(bot_role), None) => {
                        debug!(
                            "Bot has role: {} (pos: {}), target has no roles",
                            bot_role.name, bot_role.position
                        );
                        true
                    }
                    _ => {
                        debug!("Bot has no role or both have no roles");
                        false
                    }
                }
            } else {
                debug!("Guild not in cache, assuming we can modify");
                true // If we can't check, try anyway
            };

            if !can_modify {
                warn!(
                    "Cannot update nickname for user {} - target user has equal or higher role",
                    discord_id
                );
            } else {
                // Update nickname
                debug!("Attempting to update nickname to: {}", user_data.name);
                match guild_id
                    .edit_member(
                        http,
                        discord_id,
                        serenity::EditMember::new().nickname(&user_data.name),
                    )
                    .await
                {
                    Ok(_) => {
                        info!(
                            "Successfully updated nickname for user {} to '{}'",
                            discord_id, user_data.name
                        );
                    }
                    Err(e) => {
                        warn!("Failed to update nickname for user {}: {}", discord_id, e);
                    }
                }
            }
        }

        // Handle role and channel access management
        if !bot_permissions.manage_roles() {
            error!("Missing permission: MANAGE_ROLES - Cannot assign roles to users");
        } else {
            debug!("Bot has MANAGE_ROLES permission");

            // Remove unverified role if it exists
            if let Some(unverified_role) = guild
                .roles
                .values()
                .find(|r| r.name.to_lowercase() == "unverified")
            {
                debug!(
                    "Found unverified role: {} ({})",
                    unverified_role.name, unverified_role.id
                );
                match target_member.remove_role(http, unverified_role.id).await {
                    Ok(_) => {
                        info!(
                            "Successfully removed Unverified role from user {}",
                            discord_id
                        );
                    }
                    Err(e) => {
                        warn!(
                            "Failed to remove Unverified role from user {}: {}",
                            discord_id, e
                        );
                    }
                }
            } else {
                debug!("No unverified role found in guild");
            }

            // Assign Member role
            if let Some(member_role) = guild
                .roles
                .values()
                .find(|r| r.name.to_lowercase() == "member")
            {
                debug!(
                    "Found member role: {} ({})",
                    member_role.name, member_role.id
                );
                match target_member.add_role(http, member_role.id).await {
                    Ok(_) => {
                        info!("Successfully assigned Member role to user {}", discord_id);
                    }
                    Err(e) => {
                        warn!("Failed to assign Member role to user {}: {}", discord_id, e);
                    }
                }
            } else {
                warn!("Member role not found in guild - please create a role named 'Member'");
            }
        }

        // Update channel permissions
        debug!("Updating channel permissions for verified user");
        if let Err(e) = self
            .update_channel_permissions(http, guild_id, discord_id, true)
            .await
        {
            warn!(
                "Failed to update channel permissions for user {}: {}",
                discord_id, e
            );
        } else {
            info!(
                "Successfully updated channel permissions for user {}",
                discord_id
            );
        }

        info!(
            "Completed updating user {} in guild {}",
            discord_id, guild_id
        );
        Ok(())
    }

    async fn update_channel_permissions(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        user_id: serenity::UserId,
        is_verified: bool,
    ) -> Result<()> {
        debug!(
            "Updating channel permissions for user {} (verified: {})",
            user_id, is_verified
        );

        let channels = match guild_id.channels(http).await {
            Ok(channels) => {
                debug!("Successfully fetched {} channels", channels.len());
                channels
            }
            Err(e) => {
                error!("Failed to fetch channels for guild {}: {}", guild_id, e);
                return Err(e.into());
            }
        };

        let mut updated_channels = 0;
        let mut failed_channels = 0;

        for (channel_id, channel) in channels {
            // Skip voice channels and categories
            if !matches!(channel.kind, serenity::ChannelType::Text) {
                continue;
            }

            let channel_name = channel.name.to_lowercase();
            debug!("Processing channel: {} ({})", channel_name, channel_id);

            // Define channel access rules
            let should_have_access = match channel_name.as_str() {
                // Always accessible channels
                "welcome" | "rules" => {
                    debug!("Channel {} is always accessible", channel_name);
                    true
                }
                // Member-only channels
                "general" | "chat" | "discussion" | "off-topic" => {
                    debug!(
                        "Channel {} requires verification: {}",
                        channel_name, is_verified
                    );
                    is_verified
                }
                // Default: member-only for other channels
                _ => {
                    debug!(
                        "Channel {} default access (member-only): {}",
                        channel_name, is_verified
                    );
                    is_verified
                }
            };

            if should_have_access {
                // Grant read and send permissions
                let permissions = serenity::PermissionOverwrite {
                    allow: serenity::Permissions::VIEW_CHANNEL
                        | serenity::Permissions::SEND_MESSAGES
                        | serenity::Permissions::READ_MESSAGE_HISTORY,
                    deny: serenity::Permissions::empty(),
                    kind: serenity::PermissionOverwriteType::Member(user_id),
                };

                debug!("Granting access to channel: {}", channel_name);
                match channel_id.create_permission(http, permissions).await {
                    Ok(_) => {
                        debug!("Successfully granted access to channel: {}", channel_name);
                        updated_channels += 1;
                    }
                    Err(e) => {
                        // Only log as warning since the permission might already exist
                        warn!(
                            "Failed to grant access to channel {} for user {}: {}",
                            channel_name, user_id, e
                        );
                        failed_channels += 1;
                    }
                }
            } else {
                // Deny access to member channels for unverified users
                let permissions = serenity::PermissionOverwrite {
                    allow: serenity::Permissions::empty(),
                    deny: serenity::Permissions::VIEW_CHANNEL,
                    kind: serenity::PermissionOverwriteType::Member(user_id),
                };

                debug!("Denying access to channel: {}", channel_name);
                match channel_id.create_permission(http, permissions).await {
                    Ok(_) => {
                        debug!("Successfully denied access to channel: {}", channel_name);
                        updated_channels += 1;
                    }
                    Err(e) => {
                        warn!(
                            "Failed to deny access to channel {} for user {}: {}",
                            channel_name, user_id, e
                        );
                        failed_channels += 1;
                    }
                }
            }
        }

        info!(
            "Updated channel permissions for user {} (verified: {}) - Success: {}, Failed: {}",
            user_id, is_verified, updated_channels, failed_channels
        );
        Ok(())
    }

    pub async fn setup_unverified_user_permissions(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        user_id: serenity::UserId,
    ) -> Result<()> {
        info!(
            "Setting up unverified user permissions for user {} in guild {}",
            user_id, guild_id
        );
        // Set up permissions for unverified users (restrict access)
        self.update_channel_permissions(http, guild_id, user_id, false)
            .await
    }
}
