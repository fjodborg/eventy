use poise::serenity_prelude as serenity;
use tracing::{info, warn};

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
        .field("/restart", "Restart the bot (Owner only)", false)
        .color(0x3498db);

    ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true)).await?;
    Ok(())
}

/// Restart the bot (requires Administrator permission)
#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
    guild_only
)]
pub async fn restart(ctx: Context<'_>) -> Result<(), Error> {
    info!("Restart command called by {} ({})", ctx.author().name, ctx.author().id);

    ctx.send(poise::CreateReply::default()
        .content("Restarting bot... This may take a few seconds.")
        .ephemeral(true))
        .await?;

    warn!("Bot restart initiated by {} via Discord command", ctx.author().name);

    // Give time for the message to be sent
    tokio::time::sleep(std::time::Duration::from_millis(500)).await;

    // Exit with code 0 - systemd/process manager will restart
    std::process::exit(0);
}
