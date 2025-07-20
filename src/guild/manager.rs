use anyhow::Result;
use poise::serenity_prelude as serenity;
use tracing::{error, info, warn};

use crate::permissions::PermissionManager;
use crate::verification::{UserData, VerificationManager};

use crate::role_manager::{create_shared_role_manager, SharedRoleManager};

#[derive(Debug)]
pub struct GuildManager {
    pub verification_manager: VerificationManager,
    pub permission_manager: PermissionManager,
    pub role_manager: SharedRoleManager, // Add this field
}

impl GuildManager {
    pub async fn new(
        verification_manager: VerificationManager,
        permission_manager: PermissionManager,
    ) -> Result<Self> {
        // Load role configuration
        let config_file =
            std::env::var("ROLES_CONFIG").unwrap_or_else(|_| "data/roles.json".to_string());

        let role_manager = create_shared_role_manager(&config_file).await?;

        info!("Loaded role configuration from: {}", config_file);

        Ok(Self {
            verification_manager,
            permission_manager,
            role_manager,
        })
    }

    // Add method to setup roles for a guild
    pub async fn setup_guild_roles(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
    ) -> Result<()> {
        info!("Setting up guild roles for guild: {}", guild_id);
        // let role_manager = self.role_manager.read().await;
        // role_manager.create_roles_in_guild(http, guild_id).await?;
        Ok(())
    }
    pub async fn handle_new_member(
        &self,
        ctx: &serenity::Context,
        new_member: &serenity::Member,
    ) -> Result<()> {
        info!(
            "Handling new member: {} in guild: {}",
            new_member.user.name, new_member.guild_id
        );

        if self
            .verification_manager
            .is_user_verified(new_member.user.id)
        {
            info!(
                "User {} is already verified, updating access",
                new_member.user.name
            );
            self.update_verified_user_access(ctx, new_member.guild_id, new_member.user.id)
                .await?;
        } else {
            info!(
                "Starting verification process for new user: {}",
                new_member.user.name
            );
            self.setup_unverified_user(ctx, new_member).await?;
            self.start_verification_process(ctx, &new_member.user)
                .await?;
        }

        // You can add logic here to assign default roles to new members
        // For example, assign the "unverified" role by default

        Ok(())
    }

    pub async fn update_user_in_guild(
        &self,
        http: &serenity::Http,
        cache: &serenity::Cache,
        guild_id: serenity::GuildId,
        discord_id: serenity::UserId,
        user_data: &UserData,
    ) -> Result<()> {
        info!("Updating user {} in guild {}", user_data.name, guild_id);

        // Update nickname and basic roles
        self.update_user_profile(http, cache, guild_id, discord_id, user_data)
            .await?;

        // Apply permission grants
        let grants = self.permission_manager.get_user_grants(discord_id);
        if !grants.is_empty() {
            self.permission_manager
                .apply_permissions_to_guild(http, guild_id, discord_id, &grants)
                .await?;
            info!(
                "Successfully verified user {} in guild {}",
                user_data.name, guild_id
            );

        // TODO: default user should have permissions removed. (fallback for e.g. kicking etc.)
        } else {
            // Apply default member permissions if no specific grants
            self.apply_default_member_permissions(http, guild_id, discord_id)
                .await?;
            warn!("TODO:SDefault user permission fallback not implemented yet.")
        }

        Ok(())
    }

    async fn update_user_profile(
        &self,
        http: &serenity::Http,
        cache: &serenity::Cache,
        guild_id: serenity::GuildId,
        discord_id: serenity::UserId,
        user_data: &UserData,
    ) -> Result<()> {
        let guild = guild_id.to_partial_guild(http).await?;
        let bot_user_id = cache.current_user().id;
        let bot_member = guild_id.member(http, bot_user_id).await?;
        let target_member = guild_id.member(http, discord_id).await?;

        let bot_permissions = guild.member_permissions(&bot_member);

        // Update nickname
        if bot_permissions.manage_nicknames() {
            if let Err(e) = guild_id
                .edit_member(
                    http,
                    discord_id,
                    serenity::EditMember::new().nickname(&user_data.name),
                )
                .await
            {
                warn!("Failed to update nickname for user {}: {}", discord_id, e);
            } else {
                info!(
                    "Updated nickname for user {} to '{}'",
                    discord_id, user_data.name
                );
            }
        }

        // Manage roles
        if bot_permissions.manage_roles() {
            // TODO: roles.
            // Remove unverified role
            // if let Some(unverified_role) = guild.roles.values().find(|r| r.name.to_lowercase() == "unverified") {
            //     if let Err(e) = target_member.remove_role(http, unverified_role.id).await {
            //         warn!("Failed to remove Unverified role from user {}: {}", discord_id, e);
            //     }
            // }

            // TODO: roles. keep this in since this is what grants specific channel permissions.
            // Add member role
            // if let Some(member_role) = guild.roles.values().find(|r| r.name.to_lowercase() == "medlem") {
            //     if let Err(e) = target_member.add_role(http, member_role.id).await {
            //         warn!("Failed to assign Member role to user {}: {}", discord_id, e);
            //     } else {
            //         info!("Assigned Member role to user {}", discord_id);
            //     }
            // }
        }

        Ok(())
    }

    async fn apply_default_member_permissions(
        &self,
        http: &serenity::Http,
        guild_id: serenity::GuildId,
        user_id: serenity::UserId,
    ) -> Result<()> {
        // Apply default permissions based on configuration
        let config = self.permission_manager.get_config();

        // TODO: roles. keep this in since this is what grants specific channel permissions.
        //       Keep in mind that this is hardcoded and we should set user roles in the user file.
        // Find default member role configuration
        if let Some(member_config) = config.get_role_config("Medlem") {
            info!("Granting medlem role to userid: {}", user_id);
            let grant = crate::permissions::PermissionGrant {
                user_id,
                guild_id,
                role_config: member_config.clone(),
                granted_at: std::time::SystemTime::now()
                    .duration_since(std::time::UNIX_EPOCH)?
                    .as_secs(),
                granted_by: serenity::UserId::new(1), // System grant
            };

            self.permission_manager
                .apply_permissions_to_guild(http, guild_id, user_id, &[grant])
                .await?;
        }

        Ok(())
    }

    async fn setup_unverified_user(
        &self,
        ctx: &serenity::Context,
        member: &serenity::Member,
    ) -> Result<()> {
        // Setup basic unverified permissions
        let channels = member.guild_id.channels(&ctx.http).await?;

        for (channel_id, channel) in channels {
            let channel_name = channel.name.to_lowercase();
            let should_have_access = matches!(channel_name.as_str(), "welcome" | "rules");

            let permissions = if should_have_access {
                serenity::PermissionOverwrite {
                    allow: serenity::Permissions::VIEW_CHANNEL
                        | serenity::Permissions::READ_MESSAGE_HISTORY,
                    deny: serenity::Permissions::SEND_MESSAGES,
                    kind: serenity::PermissionOverwriteType::Member(member.user.id),
                }
            } else {
                serenity::PermissionOverwrite {
                    allow: serenity::Permissions::empty(),
                    deny: serenity::Permissions::VIEW_CHANNEL,
                    kind: serenity::PermissionOverwriteType::Member(member.user.id),
                }
            };

            if let Err(e) = channel_id.create_permission(&ctx.http, permissions).await {
                warn!(
                    "Failed to set permissions for channel {}: {}",
                    channel_name, e
                );
            }
        }

        Ok(())
    }

    async fn start_verification_process(
        &self,
        ctx: &serenity::Context,
        user: &serenity::User,
    ) -> Result<()> {
        // Send welcome message and start DM verification
        // This would include the welcome channel logic from your original code
        match user.create_dm_channel(&ctx.http).await {
            Ok(dm_channel) => {
                self.verification_manager
                    .add_pending_verification(user.id, dm_channel.id);

                let verification_msg = crate::messages::verification_message(&user.name);
                dm_channel
                    .send_message(
                        &ctx.http,
                        serenity::CreateMessage::new().content(verification_msg),
                    )
                    .await?;

                info!("Started verification process for user: {}", user.name);
            }
            Err(e) => {
                error!("Failed to create DM channel for user {}: {}", user.name, e);
                return Err(e.into());
            }
        }

        Ok(())
    }

    async fn update_verified_user_access(
        &self,
        ctx: &serenity::Context,
        guild_id: serenity::GuildId,
        user_id: serenity::UserId,
    ) -> Result<()> {
        if let Some(verified_user) = self.verification_manager.get_verified_user(user_id) {
            self.update_user_in_guild(
                &ctx.http,
                &ctx.cache,
                guild_id,
                user_id,
                &verified_user.user_data,
            )
            .await?;
        }
        Ok(())
    }

    pub fn get_verification_manager(&self) -> &VerificationManager {
        &self.verification_manager
    }

    pub fn get_permission_manager(&self) -> &PermissionManager {
        &self.permission_manager
    }
}
