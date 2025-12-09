use poise::serenity_prelude as serenity;
use tracing::{debug, info};

use crate::commands::verification::handle_dm_verification;
use crate::{Data, Error};

/// Handle incoming messages
pub async fn handle_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    // Ignore bot messages
    if msg.author.bot {
        return Ok(());
    }

    // Check if this is a DM
    if msg.guild_id.is_none() {
        return handle_dm_message(ctx, msg, data).await;
    }

    // Check if this is the maintainers channel
    if let Some(guild_id) = msg.guild_id {
        let maintainers_manager = data.maintainers_manager.read().await;
        if maintainers_manager
            .is_maintainers_channel(msg.channel_id, guild_id)
            .await
        {
            return handle_maintainers_message(ctx, msg, data).await;
        }
    }

    Ok(())
}

/// Handle DM messages (for verification)
async fn handle_dm_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    debug!("Processing DM from: {}", msg.author.name);

    // Handle verification attempts
    handle_dm_verification(ctx, msg, data).await
}

/// Handle messages in the maintainers channel
async fn handle_maintainers_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    // Check if user has admin permissions
    // Check if user has admin permissions
    if let Some(guild_id) = msg.guild_id {
        if let Ok(member) = guild_id.member(ctx, msg.author.id).await {
            if let Ok(channel) = msg.channel(ctx).await {
                if let Some(guild_channel) = channel.guild() {
                    if let Some(guild) = guild_id.to_guild_cached(&ctx.cache) {
                        let permissions = guild.user_permissions_in(&guild_channel, &member);
                        if !permissions.administrator() {
                            // Non-admin posting in maintainers channel - ignore silently
                            debug!(
                                "Non-admin {} posted in maintainers channel",
                                msg.author.name
                            );
                            return Ok(());
                        }
                    }
                }
            }
        }
    }

    // Check if message has attachments
    if msg.attachments.is_empty() {
        return Ok(());
    }

    info!(
        "Processing {} attachment(s) from {} in maintainers channel",
        msg.attachments.len(),
        msg.author.name
    );

    let maintainers_manager = data.maintainers_manager.read().await;
    let results = maintainers_manager
        .handle_message_attachments(&msg.attachments, Some(msg.author.id.to_string()))
        .await;

    // Format and send response
    let response = maintainers_manager.format_results(&results);

    // React to indicate processing
    let emoji = if results.iter().all(|(_, r)| r.is_ok()) {
        serenity::ReactionType::Unicode("✅".to_string())
    } else if results.iter().any(|(_, r)| r.is_ok()) {
        serenity::ReactionType::Unicode("⚠️".to_string())
    } else {
        serenity::ReactionType::Unicode("❌".to_string())
    };

    let _ = msg.react(&ctx.http, emoji).await;

    // Send detailed response
    msg.reply(&ctx.http, response).await?;

    Ok(())
}
