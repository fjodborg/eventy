use crate::{Context, Error};
use poise::serenity_prelude as serenity;

/// Update a category structure for a specific season
#[poise::command(slash_command, guild_only)]
pub async fn update_category(
    ctx: Context<'_>,
    #[description = "Season ID to update (e.g., 2025E)"] season_id: String,
) -> Result<(), Error> {
    ctx.defer().await?;

    let guild_id = ctx.guild_id().ok_or("Must be run in a guild")?;

    // Manual permission check
    let is_owner = if let Some(guild) = ctx.guild() {
        guild.owner_id == ctx.author().id
    } else {
        false
    };

    let has_perms = if let Some(member) = ctx.author_member().await {
        if let Some(guild) = ctx.guild() {
            if let Some(guild_channel) = guild.channels.get(&ctx.channel_id()) {
                guild
                    .user_permissions_in(guild_channel, &member)
                    .contains(serenity::Permissions::MANAGE_CHANNELS)
            } else {
                false
            }
        } else {
            false
        }
    } else {
        false
    };

    if !is_owner && !has_perms {
        ctx.say("You are missing the `MANAGE_CHANNELS` permission to run this command.")
            .await?;
        return Ok(());
    }

    let config_manager = ctx.data().config_manager.read().await;
    let channel_manager = ctx.data().channel_manager.read().await;

    // Get global structure
    let global_structure = match config_manager.get_global_structure() {
        Some(gs) => gs,
        None => {
            ctx.say("Global structure not configured.").await?;
            return Ok::<(), Error>(());
        }
    };

    // Get category structure
    let category_structure = match config_manager.get_category_structure(&season_id) {
        Some(cs) => cs,
        None => {
            ctx.say(format!(
                "Category structure for season '{}' not found.",
                season_id
            ))
            .await?;
            return Ok::<(), Error>(());
        }
    };

    // Run update
    match channel_manager
        .ensure_structure_exists(
            ctx.http(),
            guild_id,
            global_structure,
            Some(category_structure),
        )
        .await
    {
        Ok(summary) => {
            let response = format!(
                "**Updated category structure for season '{}'**\n\n{}",
                season_id,
                summary.format()
            );
            ctx.say(response).await?;
        }
        Err(e) => {
            ctx.say(format!("Failed to update category structure: {}", e))
                .await?;
        }
    }

    Ok(())
}
