use poise::serenity_prelude as serenity;
use tracing::info;

use crate::{Context, Error};

/// Check if the bot is running
#[poise::command(prefix_command, slash_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    info!("Ping command called by {}", ctx.author().name);
    ctx.send(poise::CreateReply::default()
        .content("Pong! Bot is working!")
        .ephemeral(true))
        .await?;
    Ok(())
}

/// Show help information
#[poise::command(prefix_command, slash_command)]
pub async fn help(ctx: Context<'_>) -> Result<(), Error> {
    let embed = serenity::CreateEmbed::new()
        .title("Bot Commands")
        .description("Available commands:")
        .field("/ping", "Check if the bot is running", false)
        .field("/verify", "Start the verification process", false)
        .field("/preview-config", "Preview staged configuration changes (Admin)", false)
        .field("/commit-config", "Apply staged configuration (Admin)", false)
        .field("/get-config", "Download a configuration file (Admin)", false)
        .color(0x3498db);

    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true)).await?;
    Ok(())
}
