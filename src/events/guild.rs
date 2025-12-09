use poise::serenity_prelude as serenity;
use tracing::{debug, error, info};

use crate::{Data, Error};

/// Handle when the bot joins a new guild or starts up
pub async fn handle_guild_create(
    ctx: &serenity::Context,
    guild: &serenity::Guild,
    data: &Data,
) -> Result<(), Error> {
    info!("Processing guild: {} ({})", guild.name, guild.id);

    // Ensure maintainers channel exists
    let maintainers_manager = data.maintainers_manager.read().await;
    match maintainers_manager
        .ensure_channel_exists(&ctx.http, guild.id)
        .await
    {
        Ok(channel_id) => {
            debug!("Maintainers channel ready: {}", channel_id);
        }
        Err(e) => {
            error!(
                "Failed to create maintainers channel for guild {}: {}",
                guild.id, e
            );
        }
    }

    // If we have a global structure, ensure roles and channels exist
    let config_manager = data.config_manager.read().await;
    if let Some(global_structure) = config_manager.get_global_structure() {
        // Ensure roles exist
        {
            let role_manager = data.role_manager.read().await;
            match role_manager
                .ensure_roles_exist(&ctx.http, guild.id, &global_structure.default_roles)
                .await
            {
                Ok(roles) => {
                    info!("Ensured {} roles exist in guild {}", roles.len(), guild.id);
                }
                Err(e) => {
                    error!("Failed to setup roles for guild {}: {}", guild.id, e);
                }
            }
        }

        // Ensure category structures exist
        // Automatic verification is disabled to prevent overwriting manual changes or causing lag
        // Users should run /update_category manually
        info!(
            "Automatic structure verification is disabled. Run /update_category to apply changes."
        );

        /*
        let channel_manager = data.channel_manager.read().await;
        let category_structures = config_manager.get_all_category_structures();

        for category in category_structures {
            match channel_manager
                .ensure_structure_exists(&ctx.http, guild.id, global_structure, Some(category))
                .await
            {
                Ok(_) => {
                    info!("Ensured structure for season {} exists", category.season_id);
                }
                Err(e) => {
                    error!(
                        "Failed to ensure structure for season {}: {}",
                        category.season_id, e
                    );
                }
            }
        }
        */
    }

    Ok(())
}

/// Handle when a new member joins the guild
pub async fn handle_member_add(
    ctx: &serenity::Context,
    new_member: &serenity::Member,
    data: &Data,
) -> Result<(), Error> {
    let user_id = new_member.user.id;
    let guild_id = new_member.guild_id;

    info!(
        "New member joined: {} in guild {}",
        new_member.user.name, guild_id
    );

    // Check if user is already verified
    let verification_manager = &data.verification_manager;

    if let Some(tracked_user) = verification_manager.get_verified_user(user_id).await {
        // User is already verified - apply their roles and nickname
        info!(
            "Returning verified user: {} ({})",
            tracked_user.display_name, user_id
        );

        // Set nickname
        if let Err(e) = new_member
            .clone()
            .edit(
                &ctx.http,
                serenity::EditMember::new().nickname(&tracked_user.display_name),
            )
            .await
        {
            error!(
                "Failed to set nickname for {} in guild {}: {}. Bot requires 'Manage Nicknames' permission and must have a higher role than the target user.",
                user_id, guild_id, e
            );
        }

        // Get roles to assign
        let config_manager = data.config_manager.read().await;
        let default_role = config_manager.get_default_member_role_name().to_string();

        let mut roles_to_assign = vec![default_role];
        roles_to_assign.extend(tracked_user.special_roles.clone());

        // Assign roles
        let role_manager = data.role_manager.read().await;
        for role_name in &roles_to_assign {
            if let Err(e) = role_manager
                .assign_role_to_user(&ctx.http, guild_id, user_id, role_name)
                .await
            {
                error!(
                    "Failed to assign role '{}' to {} in guild {}: {}. Bot requires 'Manage Roles' permission and the bot's role must be higher than the role being assigned.",
                    role_name, user_id, guild_id, e
                );
            }
        }

        // Send welcome back message in DM
        if let Ok(dm_channel) = new_member.user.create_dm_channel(&ctx.http).await {
            let _ = dm_channel
                .send_message(
                    &ctx.http,
                    serenity::CreateMessage::new().content(format!(
                        "**Welcome back, {}!**\n\n\
                    You're already verified, so your roles have been restored automatically.",
                        tracked_user.display_name
                    )),
                )
                .await;
        }
    } else {
        // User is not verified - send welcome message with verification instructions
        if let Ok(dm_channel) = new_member.user.create_dm_channel(&ctx.http).await {
            let _ = dm_channel.send_message(&ctx.http, serenity::CreateMessage::new()
                .content(format!(
                    "**Welcome to the server, {}!**\n\n\
                    To get access to the server, please verify your identity using the `/verify` command.\n\n\
                    You can use the command in any channel or reply here.",
                    new_member.user.name
                ))
            ).await;
        }
    }

    Ok(())
}

/// Find the welcome channel in a guild
pub async fn find_welcome_channel(
    ctx: &serenity::Context,
    guild_id: serenity::GuildId,
) -> Option<serenity::GuildChannel> {
    let channels = guild_id.channels(&ctx.http).await.ok()?;

    // Look for a channel named "welcome"
    for (_, channel) in &channels {
        if channel.kind == serenity::ChannelType::Text && channel.name.to_lowercase() == "welcome" {
            return Some(channel.clone());
        }
    }

    // Fall back to first text channel
    for (_, channel) in &channels {
        if channel.kind == serenity::ChannelType::Text {
            return Some(channel.clone());
        }
    }

    None
}
