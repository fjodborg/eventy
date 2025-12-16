use tracing::{error, info};

use crate::{Context, Error};

/// Update a category structure for a specific season
#[poise::command(slash_command, guild_only)]
pub async fn update_category(
    ctx: Context<'_>,
    #[description = "Season ID to update (e.g., 2025E)"] season_id: String,
) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("This command must be used in a guild")?;

    // Defer the response since channel creation can take a while
    ctx.defer().await?;

    let config_manager = ctx.data().config_manager.read().await;

    // Get season
    let season = match config_manager.get_season(&season_id) {
        Some(s) => s,
        None => {
            ctx.say(format!("Season '{}' not found.", season_id)).await?;
            return Ok(());
        }
    };

    // Check if season has channels defined
    if season.channels().is_empty() {
        ctx.say(format!(
            "Season '{}' has no channels defined in season.json.\n\nTo define channels, add them to `data/seasons/{}/season.json`",
            season_id, season_id
        ))
        .await?;
        return Ok(());
    }

    // Clone the channels since we need to drop config_manager before using channel_manager
    let channels = season.channels().to_vec();
    let category_name = season.name().to_string();
    drop(config_manager);

    // Get the channel manager and HTTP client
    let channel_manager = ctx.data().channel_manager.read().await;
    let http = ctx.serenity_context().http.as_ref();

    // First, ensure the category exists (using season name as category name)
    info!("Creating/verifying category '{}' for season '{}'", category_name, season_id);
    let category_id = channel_manager
        .ensure_category_exists(http, guild_id, &category_name)
        .await?;

    let mut created_channels = Vec::new();
    let mut errors = Vec::new();

    // Create each channel within the category
    for channel_def in &channels {
        info!("Creating/verifying channel '{}' in category '{}'", channel_def.name, category_name);

        match channel_manager
            .ensure_channel_exists(http, guild_id, channel_def, Some(category_id))
            .await
        {
            Ok(_channel_id) => {
                // Check if we created it or it already existed
                // For now, assume it was handled (created or updated)
                created_channels.push(channel_def.name.clone());
            }
            Err(e) => {
                error!("Failed to configure channel '{}': {}", channel_def.name, e);
                errors.push(format!("{}: {}", channel_def.name, e));
            }
        }
    }

    drop(channel_manager);

    // Build response message
    let mut response = format!("**Season '{}' structure updated:**\n", season_id);
    response.push_str(&format!("Category: {}\n\n", category_name));

    if !created_channels.is_empty() {
        response.push_str(&format!(
            "Channels configured ({}):\n",
            created_channels.len()
        ));
        for ch in &created_channels {
            response.push_str(&format!("  - #{}\n", ch));
        }
    }

    if !errors.is_empty() {
        response.push_str(&format!("\nErrors ({}):\n", errors.len()));
        for err in &errors {
            response.push_str(&format!("  - {}\n", err));
        }
    }

    ctx.say(response).await?;

    Ok(())
}
