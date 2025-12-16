use poise::serenity_prelude as serenity;
use tracing::{error, info};

use crate::{Context, Error};

/// Handle a role operation error and return a user-friendly message
fn format_role_error(role_name: &str, e: &serenity::Error) -> String {
    let err_str = e.to_string();
    if err_str.contains("Missing Permissions") || err_str.contains("50013") {
        let msg = format!(
            "{}: Role hierarchy issue - move bot's role above '{}' in Discord server settings",
            role_name, role_name
        );
        error!("{}", msg);
        msg
    } else {
        error!("Failed to sync role '{}': {}", role_name, e);
        format!("{}: {}", role_name, e)
    }
}

/// Sync Discord roles with the global roles configuration
#[poise::command(slash_command, guild_only)]
pub async fn update_roles(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("This command must be used in a guild")?;

    // Defer the response since role operations can take a while
    ctx.defer().await?;

    let config_manager = ctx.data().config_manager.read().await;

    // Check if we have global roles configured
    let global_roles = match config_manager.get_global_roles() {
        Some(r) => r.clone(),
        None => {
            ctx.say("No global roles configured. Add roles to `data/global/roles.json`.").await?;
            return Ok(());
        }
    };
    drop(config_manager);

    let http = ctx.serenity_context().http.as_ref();

    // Get existing roles in the guild
    let existing_roles = guild_id.roles(http).await?;

    let mut created = Vec::new();
    let mut updated = Vec::new();
    let mut unchanged = Vec::new();
    let mut errors = Vec::new();

    for role_def in &global_roles.roles {
        // Check if role already exists
        if let Some((role_id, existing_role)) = existing_roles.iter().find(|(_, r)| r.name == role_def.name) {
            // Role exists - check if it needs updating
            let target_color = role_def
                .color
                .as_ref()
                .and_then(|c| {
                    let hex = c.trim_start_matches('#');
                    u32::from_str_radix(hex, 16).ok()
                })
                .unwrap_or(0);

            let needs_update = existing_role.colour.0 != target_color
                || existing_role.hoist != role_def.hoist
                || existing_role.mentionable != role_def.mentionable;

            if needs_update {
                // Update the role using guild_id.edit_role
                match guild_id
                    .edit_role(
                        http,
                        *role_id,
                        serenity::EditRole::new()
                            .colour(target_color as u64)
                            .hoist(role_def.hoist)
                            .mentionable(role_def.mentionable),
                    )
                    .await
                {
                    Ok(_) => {
                        info!("Updated role '{}'", role_def.name);
                        updated.push(role_def.name.clone());
                    }
                    Err(e) => {
                        errors.push(format_role_error(&role_def.name, &e));
                    }
                }
            } else {
                unchanged.push(role_def.name.clone());
            }
        } else {
            // Create new role
            let color = role_def
                .color
                .as_ref()
                .and_then(|c| {
                    let hex = c.trim_start_matches('#');
                    u32::from_str_radix(hex, 16).ok().map(serenity::Colour::new)
                })
                .unwrap_or(serenity::Colour::default());

            match guild_id
                .create_role(
                    http,
                    serenity::EditRole::new()
                        .name(&role_def.name)
                        .colour(color)
                        .hoist(role_def.hoist)
                        .mentionable(role_def.mentionable),
                )
                .await
            {
                Ok(role) => {
                    info!("Created role '{}' (ID: {})", role_def.name, role.id);
                    created.push(role_def.name.clone());
                }
                Err(e) => {
                    errors.push(format_role_error(&role_def.name, &e));
                }
            }
        }
    }

    // Build response message
    let mut response = String::from("**Role sync complete:**\n\n");

    if !created.is_empty() {
        response.push_str(&format!("Created ({}):\n", created.len()));
        for name in &created {
            response.push_str(&format!("  - @{}\n", name));
        }
        response.push('\n');
    }

    if !updated.is_empty() {
        response.push_str(&format!("Updated ({}):\n", updated.len()));
        for name in &updated {
            response.push_str(&format!("  - @{}\n", name));
        }
        response.push('\n');
    }

    if !unchanged.is_empty() {
        response.push_str(&format!("Unchanged ({}):\n", unchanged.len()));
        for name in &unchanged {
            response.push_str(&format!("  - @{}\n", name));
        }
        response.push('\n');
    }

    if !errors.is_empty() {
        response.push_str(&format!("Errors ({}):\n", errors.len()));
        for err in &errors {
            response.push_str(&format!("  - {}\n", err));
        }
    }

    if created.is_empty() && updated.is_empty() && errors.is_empty() {
        response = String::from("All roles are already in sync with the configuration.");
    }

    ctx.say(response).await?;

    Ok(())
}
