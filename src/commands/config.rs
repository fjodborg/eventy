use poise::serenity_prelude as serenity;
use tracing::{info, error, warn};

use crate::{Context, Error};

/// Download configuration files interactively with buttons
#[poise::command(
    slash_command,
    guild_only,
    default_member_permissions = "ADMINISTRATOR"
)]
pub async fn get_config(ctx: Context<'_>) -> Result<(), Error> {
    info!("get_config called by {}", ctx.author().name);
    let config_manager = ctx.data().config_manager.read().await;
    let seasons: Vec<String> = config_manager.get_all_seasons()
        .iter()
        .map(|s| s.season_id.clone())
        .collect();
    let has_global = config_manager.get_global_structure().is_some();
    drop(config_manager);

    // Build buttons for main categories
    let mut buttons = Vec::new();

    if has_global {
        buttons.push(
            serenity::CreateButton::new("config_global")
                .label("Global Config")
                .style(serenity::ButtonStyle::Primary)
        );
    }

    // Add season buttons
    for season_id in &seasons {
        buttons.push(
            serenity::CreateButton::new(format!("config_season_{}", season_id))
                .label(format!("Season: {}", season_id))
                .style(serenity::ButtonStyle::Secondary)
        );
    }

    if buttons.is_empty() {
        ctx.send(poise::CreateReply::default()
            .content("No configurations are currently loaded.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    let components = vec![
        serenity::CreateActionRow::Buttons(buttons)
    ];

    let embed = serenity::CreateEmbed::new()
        .title("Configuration Download")
        .description("Click a button to select which configuration to download.\nButtons will expire after 60 seconds.")
        .color(0x3498db);

    let reply = ctx.send(poise::CreateReply::default()
        .embed(embed)
        .components(components)
        .ephemeral(true))
        .await?;

    // Wait for button interaction
    let message = reply.message().await?;

    while let Some(interaction) = message
        .await_component_interaction(ctx.serenity_context().shard.clone())
        .timeout(std::time::Duration::from_secs(60))
        .await
    {
        let custom_id = &interaction.data.custom_id;

        if custom_id == "config_global" {
            info!("User {} clicked config_global button", ctx.author().name);

            if let Err(e) = interaction.create_response(
                ctx.http(),
                serenity::CreateInteractionResponse::Acknowledge
            ).await {
                error!("Failed to acknowledge global config interaction: {}", e);
                continue;
            }

            let config_manager = ctx.data().config_manager.read().await;
            match config_manager.export_config("global", None) {
                Ok((filename, content)) => {
                    let attachment = serenity::CreateAttachment::bytes(content, filename.clone());
                    if let Err(e) = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("Here is `{}`:", filename))
                            .add_file(attachment)
                            .ephemeral(true)
                    ).await {
                        error!("Failed to send global config followup: {}", e);
                    } else {
                        info!("Config '{}' downloaded by {}", filename, ctx.author().name);
                    }
                }
                Err(e) => {
                    error!("Failed to export global config: {}", e);
                    let _ = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("Error exporting global config: {}", e))
                            .ephemeral(true)
                    ).await;
                }
            }
        } else if custom_id.starts_with("config_season_") {
            let season_id = custom_id.strip_prefix("config_season_").unwrap_or("");
            info!("User {} clicked config_season_{} button", ctx.author().name, season_id);

            // Show sub-menu for season files
            let season_buttons = vec![
                serenity::CreateButton::new(format!("download_users_{}", season_id))
                    .label("users.json")
                    .style(serenity::ButtonStyle::Primary),
                serenity::CreateButton::new(format!("download_category_{}", season_id))
                    .label("category.json")
                    .style(serenity::ButtonStyle::Secondary),
                serenity::CreateButton::new("config_back")
                    .label("<- Back")
                    .style(serenity::ButtonStyle::Danger),
            ];

            let embed = serenity::CreateEmbed::new()
                .title(format!("Season: {}", season_id))
                .description("Select a file to download:")
                .color(0x3498db);

            if let Err(e) = interaction.create_response(
                ctx.http(),
                serenity::CreateInteractionResponse::UpdateMessage(
                    serenity::CreateInteractionResponseMessage::new()
                        .embed(embed)
                        .components(vec![serenity::CreateActionRow::Buttons(season_buttons)])
                )
            ).await {
                error!("Failed to show season sub-menu: {}", e);
            }
        } else if custom_id == "config_back" {
            info!("User {} clicked back button", ctx.author().name);
            let config_manager = ctx.data().config_manager.read().await;
            let seasons: Vec<String> = config_manager.get_all_seasons()
                .iter()
                .map(|s| s.season_id.clone())
                .collect();
            let has_global = config_manager.get_global_structure().is_some();
            drop(config_manager);

            let mut buttons = Vec::new();
            if has_global {
                buttons.push(
                    serenity::CreateButton::new("config_global")
                        .label("Global Config")
                        .style(serenity::ButtonStyle::Primary)
                );
            }
            for season_id in &seasons {
                buttons.push(
                    serenity::CreateButton::new(format!("config_season_{}", season_id))
                        .label(format!("Season: {}", season_id))
                        .style(serenity::ButtonStyle::Secondary)
                );
            }

            let embed = serenity::CreateEmbed::new()
                .title("Configuration Download")
                .description("Click a button to select which configuration to download.\nButtons will expire after 60 seconds.")
                .color(0x3498db);

            if let Err(e) = interaction.create_response(
                ctx.http(),
                serenity::CreateInteractionResponse::UpdateMessage(
                    serenity::CreateInteractionResponseMessage::new()
                        .embed(embed)
                        .components(vec![serenity::CreateActionRow::Buttons(buttons)])
                )
            ).await {
                error!("Failed to show main menu: {}", e);
            }
        } else if custom_id.starts_with("download_users_") {
            let season_id = custom_id.strip_prefix("download_users_").unwrap_or("");
            info!("User {} clicked download_users_{} button", ctx.author().name, season_id);

            if let Err(e) = interaction.create_response(
                ctx.http(),
                serenity::CreateInteractionResponse::Acknowledge
            ).await {
                error!("Failed to acknowledge users download: {}", e);
                continue;
            }

            let config_manager = ctx.data().config_manager.read().await;
            match config_manager.export_config("users", Some(season_id)) {
                Ok((filename, content)) => {
                    let attachment = serenity::CreateAttachment::bytes(content, filename.clone());
                    if let Err(e) = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("Here is `{}`:", filename))
                            .add_file(attachment)
                            .ephemeral(true)
                    ).await {
                        error!("Failed to send users.json followup: {}", e);
                    } else {
                        info!("Config '{}' downloaded by {}", filename, ctx.author().name);
                    }
                }
                Err(e) => {
                    error!("Failed to export users.json for season {}: {}", season_id, e);
                    let _ = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("Error exporting users.json: {}", e))
                            .ephemeral(true)
                    ).await;
                }
            }
        } else if custom_id.starts_with("download_category_") {
            let season_id = custom_id.strip_prefix("download_category_").unwrap_or("");
            info!("User {} clicked download_category_{} button", ctx.author().name, season_id);

            if let Err(e) = interaction.create_response(
                ctx.http(),
                serenity::CreateInteractionResponse::Acknowledge
            ).await {
                error!("Failed to acknowledge category download: {}", e);
                continue;
            }

            let config_manager = ctx.data().config_manager.read().await;
            match config_manager.export_config("category", Some(season_id)) {
                Ok((filename, content)) => {
                    let attachment = serenity::CreateAttachment::bytes(content, filename.clone());
                    if let Err(e) = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("Here is `{}`:", filename))
                            .add_file(attachment)
                            .ephemeral(true)
                    ).await {
                        error!("Failed to send category.json followup: {}", e);
                    } else {
                        info!("Config '{}' downloaded by {}", filename, ctx.author().name);
                    }
                }
                Err(e) => {
                    error!("Failed to export category.json for season {}: {}", season_id, e);
                    let _ = interaction.create_followup(
                        ctx.http(),
                        serenity::CreateInteractionResponseFollowup::new()
                            .content(format!("No category.json found for season {}", season_id))
                            .ephemeral(true)
                    ).await;
                }
            }
        } else {
            warn!("Unknown button interaction: {}", custom_id);
        }
    }

    Ok(())
}

/// Upload global configuration (global.json)
#[poise::command(
    slash_command,
    guild_only,
    default_member_permissions = "ADMINISTRATOR"
)]
pub async fn set_config_global(
    ctx: Context<'_>,
    #[description = "The global.json file to upload"]
    file: serenity::Attachment,
) -> Result<(), Error> {
    info!("set_config_global called by {} with file {}", ctx.author().name, file.filename);

    // Validate file is JSON
    if !file.filename.ends_with(".json") {
        ctx.send(poise::CreateReply::default()
            .content("Please upload a `.json` file.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    // Download the file
    let content = match file.download().await {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to download attachment: {}", e);
            ctx.send(poise::CreateReply::default()
                .content(format!("Failed to download file: {}", e))
                .ephemeral(true))
                .await?;
            return Ok(());
        }
    };

    // Validate JSON
    let json: serde_json::Value = match serde_json::from_slice(&content) {
        Ok(j) => j,
        Err(e) => {
            error!("Invalid JSON in uploaded file: {}", e);
            ctx.send(poise::CreateReply::default()
                .content(format!("Invalid JSON file: {}", e))
                .ephemeral(true))
                .await?;
            return Ok(());
        }
    };

    // Stage the config
    {
        let mut config_manager = ctx.data().config_manager.write().await;
        config_manager.stage_raw_config("global", None, content.clone());
    }

    // Show preview with commit/cancel buttons
    let preview = serde_json::to_string_pretty(&json)
        .unwrap_or_else(|_| "Failed to format JSON".to_string());
    let preview_truncated = if preview.len() > 1000 {
        format!("{}...\n(truncated)", &preview[..1000])
    } else {
        preview
    };

    let embed = serenity::CreateEmbed::new()
        .title("Staged global.json")
        .description(format!(
            "**File:** `{}`\n\n**Preview:**\n```json\n{}\n```\n\nClick **Commit** to apply or **Cancel** to discard.",
            file.filename,
            preview_truncated
        ))
        .color(0x2ecc71);

    let buttons = vec![
        serenity::CreateButton::new("commit_config")
            .label("Commit Changes")
            .style(serenity::ButtonStyle::Success),
        serenity::CreateButton::new("cancel_config")
            .label("Cancel")
            .style(serenity::ButtonStyle::Danger),
    ];

    let reply = ctx.send(poise::CreateReply::default()
        .embed(embed)
        .components(vec![serenity::CreateActionRow::Buttons(buttons)])
        .ephemeral(true))
        .await?;

    // Wait for button interaction
    let message = reply.message().await?;

    if let Some(interaction) = message
        .await_component_interaction(ctx.serenity_context().shard.clone())
        .timeout(std::time::Duration::from_secs(120))
        .await
    {
        handle_commit_or_cancel(&ctx, &interaction).await;
    } else {
        // Timeout - clear staged
        let mut config_manager = ctx.data().config_manager.write().await;
        config_manager.clear_staged();
        info!("Config staging timed out for {}", ctx.author().name);
    }

    Ok(())
}

/// Upload season configuration (users.json for a season)
#[poise::command(
    slash_command,
    guild_only,
    default_member_permissions = "ADMINISTRATOR"
)]
pub async fn set_config_season(
    ctx: Context<'_>,
    #[description = "Season ID (e.g., 2025E, 2025F)"]
    season_id: String,
    #[description = "The users.json file for this season"]
    file: serenity::Attachment,
) -> Result<(), Error> {
    info!("set_config_season called by {} for season {} with file {}",
          ctx.author().name, season_id, file.filename);

    // Validate season_id
    if season_id.is_empty() || season_id.len() > 20 || season_id.contains(' ') {
        ctx.send(poise::CreateReply::default()
            .content("Invalid season ID. Use a short identifier like `2025F` without spaces.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    // Validate file is JSON
    if !file.filename.ends_with(".json") {
        ctx.send(poise::CreateReply::default()
            .content("Please upload a `.json` file.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    // Download the file
    let content = match file.download().await {
        Ok(data) => data,
        Err(e) => {
            error!("Failed to download attachment: {}", e);
            ctx.send(poise::CreateReply::default()
                .content(format!("Failed to download file: {}", e))
                .ephemeral(true))
                .await?;
            return Ok(());
        }
    };

    // Validate JSON
    let json: serde_json::Value = match serde_json::from_slice(&content) {
        Ok(j) => j,
        Err(e) => {
            error!("Invalid JSON in uploaded file: {}", e);
            ctx.send(poise::CreateReply::default()
                .content(format!("Invalid JSON file: {}", e))
                .ephemeral(true))
                .await?;
            return Ok(());
        }
    };

    // Stage the config using the method that supports both array and object formats
    let staging_result = {
        let mut config_manager = ctx.data().config_manager.write().await;
        config_manager.stage_season_from_bytes_with_id(
            &content,
            &file.filename,
            Some(&season_id),
            Some(ctx.author().name.clone()),
        )
    };

    // Check if staging failed
    if let Err(e) = staging_result {
        error!("Failed to stage season config: {}", e);
        ctx.send(poise::CreateReply::default()
            .content(format!("Failed to parse users.json: {}\n\nExpected format:\n```json\n[\n  {{ \"Name\": \"...\", \"DiscordId\": \"uuid-here\" }},\n  ...\n]\n```", e))
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    // Show preview with commit/cancel buttons
    let preview = serde_json::to_string_pretty(&json)
        .unwrap_or_else(|_| "Failed to format JSON".to_string());
    let preview_truncated = if preview.len() > 1000 {
        format!("{}...\n(truncated)", &preview[..1000])
    } else {
        preview
    };

    let embed = serenity::CreateEmbed::new()
        .title(format!("Staged users.json for Season {}", season_id))
        .description(format!(
            "**File:** `{}`\n\n**Preview:**\n```json\n{}\n```\n\nClick **Commit** to apply or **Cancel** to discard.",
            file.filename,
            preview_truncated
        ))
        .color(0x2ecc71);

    let buttons = vec![
        serenity::CreateButton::new("commit_config")
            .label("Commit Changes")
            .style(serenity::ButtonStyle::Success),
        serenity::CreateButton::new("cancel_config")
            .label("Cancel")
            .style(serenity::ButtonStyle::Danger),
    ];

    let reply = ctx.send(poise::CreateReply::default()
        .embed(embed)
        .components(vec![serenity::CreateActionRow::Buttons(buttons)])
        .ephemeral(true))
        .await?;

    // Wait for button interaction
    let message = reply.message().await?;

    if let Some(interaction) = message
        .await_component_interaction(ctx.serenity_context().shard.clone())
        .timeout(std::time::Duration::from_secs(120))
        .await
    {
        handle_commit_or_cancel(&ctx, &interaction).await;
    } else {
        // Timeout - clear staged
        let mut config_manager = ctx.data().config_manager.write().await;
        config_manager.clear_staged();
        info!("Config staging timed out for {}", ctx.author().name);
    }

    Ok(())
}

/// Helper to handle commit/cancel button clicks
async fn handle_commit_or_cancel(
    ctx: &Context<'_>,
    interaction: &serenity::ComponentInteraction,
) {
    let custom_id = &interaction.data.custom_id;

    if custom_id == "commit_config" {
        let mut config_manager = ctx.data().config_manager.write().await;
        match config_manager.commit_staged().await {
            Ok(changes) => {
                let change_summary = if changes.is_empty() {
                    "Configuration saved.".to_string()
                } else {
                    changes
                        .iter()
                        .map(|c| format!("✅ {} ({}): {}", c.entity_type, c.entity_name, c.details))
                        .collect::<Vec<_>>()
                        .join("\n")
                };

                // Check for missing files for any committed seasons
                let mut missing_files_info = String::new();
                let mut next_steps = Vec::new();

                for change in &changes {
                    if change.entity_type == "season" {
                        let (_, missing) = config_manager.get_season_file_status(&change.entity_name);
                        for file in &missing {
                            if file.contains("category.json") {
                                missing_files_info.push_str(&format!("⚠️ Missing: `{}`\n", file));
                                next_steps.push(format!(
                                    "Upload category.json for {} to define channels",
                                    change.entity_name
                                ));
                            }
                        }
                    }
                }

                // Check global.json
                if config_manager.get_global_structure().is_none() {
                    missing_files_info.push_str("⚠️ Missing: `global.json`\n");
                    next_steps.push("Upload global.json with `/set_config_global` to define roles".to_string());
                }

                let mut description = format!("**Changes applied:**\n{}", change_summary);

                if !missing_files_info.is_empty() {
                    description.push_str(&format!("\n\n**Missing files:**\n{}", missing_files_info));
                }

                if !next_steps.is_empty() {
                    description.push_str("\n\n**Next steps:**\n");
                    for (i, step) in next_steps.iter().enumerate() {
                        description.push_str(&format!("{}. {}\n", i + 1, step));
                    }
                }

                let embed = serenity::CreateEmbed::new()
                    .title("Configuration Committed!")
                    .description(description)
                    .color(0x2ecc71);

                let _ = interaction.create_response(
                    ctx.http(),
                    serenity::CreateInteractionResponse::UpdateMessage(
                        serenity::CreateInteractionResponseMessage::new()
                            .embed(embed)
                            .components(vec![])
                    )
                ).await;
                info!("Configuration committed by {}", ctx.author().name);
            }
            Err(e) => {
                let embed = serenity::CreateEmbed::new()
                    .title("Commit Failed")
                    .description(format!("Error: {}", e))
                    .color(0xe74c3c);

                let _ = interaction.create_response(
                    ctx.http(),
                    serenity::CreateInteractionResponse::UpdateMessage(
                        serenity::CreateInteractionResponseMessage::new()
                            .embed(embed)
                            .components(vec![])
                    )
                ).await;
                error!("Configuration commit failed: {}", e);
            }
        }
    } else if custom_id == "cancel_config" {
        let mut config_manager = ctx.data().config_manager.write().await;
        config_manager.clear_staged();

        let embed = serenity::CreateEmbed::new()
            .title("Cancelled")
            .description("Configuration update cancelled.")
            .color(0xe74c3c);

        let _ = interaction.create_response(
            ctx.http(),
            serenity::CreateInteractionResponse::UpdateMessage(
                serenity::CreateInteractionResponseMessage::new()
                    .embed(embed)
                    .components(vec![])
            )
        ).await;
        info!("User {} cancelled config update", ctx.author().name);
    }
}
