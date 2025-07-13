use anyhow::Result;
use poise::serenity_prelude as serenity;
use tracing::{debug, error, info, warn};

use crate::{messages::{error_message, success_message, verification_message}, Context, Error};

#[poise::command(prefix_command, slash_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    info!("Ping command called by {}", ctx.author().name);
    ctx.say("🏓 Pong! Bot is working!").await?;
    Ok(())
}

#[poise::command(prefix_command, slash_command)]
pub async fn verify(ctx: Context<'_>) -> Result<(), Error> {
    let discord_id = ctx.author().id;
    debug!(
        "Verify command called by {} ({})",
        ctx.author().name,
        discord_id
    );

    // Check if this is a DM
    if ctx.guild_id().is_none() {
        info!("Verify command called in DM by {}", ctx.author().name);
        // This is a DM - handle verification directly
        return handle_dm_verify_command(ctx).await;
    }

    info!("Verify command called in guild by {}", ctx.author().name);

    // This is in a guild - check verification status
    if ctx.data().user_manager.is_user_verified(discord_id) {
        debug!("User {} is already verified", ctx.author().name);
        ctx.send(
            poise::CreateReply::default()
                .content("✅ You are already verified!")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    // Check if user is already in verification process
    if ctx.data().user_manager.is_pending_verification(discord_id) {
        debug!(
            "User {} already has pending verification",
            ctx.author().name
        );
        ctx.send(poise::CreateReply::default()
            .content("🔄 You already have a pending verification. Please check your DMs and respond there.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    // Start verification process
    start_verification_for_user(ctx).await
}

async fn handle_dm_verify_command(ctx: Context<'_>) -> Result<(), Error> {
    let discord_id = ctx.author().id;
    debug!("Handling DM verify command for {}", ctx.author().name);

    // Check if user is already verified
    if ctx.data().user_manager.is_user_verified(discord_id) {
        info!(
            "User {} tried to verify in DM but is already verified",
            ctx.author().name
        );
        ctx.say("✅ You are already verified!").await?;
        return Ok(());
    }

    // Start or restart verification process in DM
    ctx.data()
        .user_manager
        .add_pending_verification(discord_id, ctx.channel_id());

    ctx.say(verification_message(&ctx.author().name)).await?;
    info!("Started DM verification for {}", ctx.author().name);

    Ok(())
}

async fn start_verification_for_user(ctx: Context<'_>) -> Result<(), Error> {
    let discord_id = ctx.author().id;
    debug!(
        "Starting verification for user {} in guild",
        ctx.author().name
    );

    // Send ephemeral message in channel directing user to DMs
    let reply = ctx.send(poise::CreateReply::default()
        .content("📨 **Verification Process Started**\n\nI've sent you a private message with verification instructions. Please check your DMs and respond there.\n\nIf you don't receive a DM, please contact an administrator.")
        .ephemeral(true))
        .await?;

    // Start verification process via DM
    match ctx.author().create_dm_channel(&ctx.http()).await {
        Ok(dm_channel) => {
            debug!(
                "Created DM channel for {} during manual verification",
                ctx.author().name
            );
            ctx.data()
                .user_manager
                .add_pending_verification(discord_id, dm_channel.id);

            match dm_channel
                .send_message(
                    &ctx.http(),
                    serenity::CreateMessage::new().content(verification_message(&ctx.author().name)),
                )
                .await
            {
                Ok(_) => {
                    info!(
                        "Sent verification DM to {} via manual command",
                        ctx.author().name
                    );
                }
                Err(e) => {
                    error!(
                        "Failed to send verification DM to {} via manual command: {}",
                        ctx.author().name,
                        e
                    );

                    // Update the ephemeral message to indicate failure
                    if let Err(edit_err) = reply.edit(ctx, poise::CreateReply::default()
                        .content("❌ **Verification Failed**\n\nI couldn't send you a private message. This might be because:\n• You have DMs disabled\n• We don't share a server\n\nPlease contact an administrator for manual verification.")
                        .ephemeral(true))
                        .await
                    {
                        error!("Failed to edit reply message: {}", edit_err);
                    }

                    ctx.data()
                        .user_manager
                        .remove_pending_verification(discord_id);
                    return Err(e.into());
                }
            }
        }
        Err(e) => {
            error!(
                "Failed to create DM channel for {} via manual command: {}",
                ctx.author().name,
                e
            );

            if let Err(edit_err) = reply.edit(ctx, poise::CreateReply::default()
                .content("❌ **Verification Failed**\n\nI couldn't create a private message channel. Please contact an administrator for manual verification.")
                .ephemeral(true))
                .await
            {
                error!("Failed to edit reply message: {}", edit_err);
            }

            return Err(e.into());
        }
    }

    Ok(())
}

pub async fn handle_dm_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    user_manager: &crate::user_manager::UserManager,
) -> Result<(), Error> {
    debug!(
        "Processing DM message from {}: '{}'",
        msg.author.name, msg.content
    );

    // Only process DMs from users with pending verification
    if !user_manager.is_pending_verification(msg.author.id) {
        debug!(
            "User {} sent DM but has no pending verification",
            msg.author.name
        );
        return Ok(());
    }

    let user_id = msg.content.trim();
    info!(
        "User {} attempting verification with ID: '{}'",
        msg.author.name, user_id
    );

    // Attempt to find user in database
    match user_manager.find_user_by_id(user_id) {
        Some(user_data) => {
            info!(
                "Verification successful for {} with ID {} -> {}",
                msg.author.name, user_id, user_data.name
            );

            // Verification successful
            user_manager.complete_verification(msg.author.id, user_id.to_string());

            match msg
                .channel_id
                .send_message(
                    &ctx.http,
                    serenity::CreateMessage::new().content(success_message(&user_data.name)),
                )
                .await
            {
                Ok(_) => {
                    debug!("Sent success message to {}", msg.author.name);
                }
                Err(e) => {
                    error!(
                        "Failed to send success message to {}: {}",
                        msg.author.name, e
                    );
                }
            }

            // Get list of guilds from cache
            let guild_ids: Vec<serenity::GuildId> = ctx.cache.guilds().into_iter().collect();
            debug!(
                "Updating user {} in {} guilds",
                msg.author.name,
                guild_ids.len()
            );

            // Update user in all guilds where the bot and user are both members
            for guild_id in guild_ids {
                debug!(
                    "Checking if user {} is in guild {}",
                    msg.author.name, guild_id
                );
                // Check if user is in this guild
                match guild_id.member(&ctx.http, msg.author.id).await {
                    Ok(_member) => {
                        info!("Updating user {} in guild {}", msg.author.name, guild_id);
                        if let Err(e) = user_manager
                            .update_user_in_guild(
                                &ctx.http,
                                &ctx.cache,
                                guild_id,
                                msg.author.id,
                                &user_data,
                            )
                            .await
                        {
                            error!(
                                "Failed to update user {} in guild {}: {}",
                                msg.author.name, guild_id, e
                            );
                        } else {
                            info!(
                                "Successfully updated user {} in guild {}",
                                msg.author.name, guild_id
                            );
                        }
                    }
                    Err(e) => {
                        debug!(
                            "User {} not in guild {} or failed to fetch: {}",
                            msg.author.name, guild_id, e
                        );
                    }
                }
            }

            info!(
                "User {} ({}) verified as {} ({})",
                msg.author.name, msg.author.id, user_data.name, user_id
            );
        }
        None => {
            warn!(
                "Verification failed for {}: User ID '{}' not found",
                msg.author.name, user_id
            );


            match msg
                .channel_id
                .send_message(
                    &ctx.http,
                    serenity::CreateMessage::new().content(error_message(user_id)),
                )
                .await
            {
                Ok(_) => {
                    debug!("Sent error message to {}", msg.author.name);
                }
                Err(e) => {
                    error!("Failed to send error message to {}: {}", msg.author.name, e);
                }
            }
        }
    }

    Ok(())
}

#[poise::command(prefix_command, slash_command, guild_only)]
pub async fn list_users(ctx: Context<'_>) -> Result<(), Error> {
    debug!("List users command called by {}", ctx.author().name);

    let guild_id = match ctx.guild_id() {
        Some(id) => id,
        None => {
            error!("List users command called outside of guild");
            return Ok(());
        }
    };

    let member = match guild_id.member(&ctx.http(), ctx.author().id).await {
        Ok(member) => member,
        Err(e) => {
            error!("Failed to get member info for {}: {}", ctx.author().name, e);
            return Err(e.into());
        }
    };

    let guild = match guild_id.to_partial_guild(&ctx.http()).await {
        Ok(guild) => guild,
        Err(e) => {
            error!("Failed to get guild info: {}", e);
            return Err(e.into());
        }
    };

    // Only allow administrators to use this command
    let permissions = guild.member_permissions(&member);
    if !permissions.administrator() {
        warn!(
            "User {} tried to use list_users without admin permissions",
            ctx.author().name
        );
        ctx.send(
            poise::CreateReply::default()
                .content("❌ You don't have permission to use this command!")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    let users: Vec<_> = ctx
        .data()
        .user_manager
        .database
        .iter()
        .map(|entry| entry.value().clone())
        .collect();

    info!(
        "Admin {} requested user list ({} users)",
        ctx.author().name,
        users.len()
    );

    if users.is_empty() {
        ctx.say("No users found in database.").await?;
        return Ok(());
    }

    let mut description = String::new();
    for user in users.iter().take(20) {
        description.push_str(&format!(
            "**{}** (ID: {})\n",
            user.name,
            user.id,
            // user.seasons.join(", ")
        ));
    }

    let embed = serenity::CreateEmbed::new()
        .title("User Database")
        .description(description)
        .color(0x0099ff);

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}
