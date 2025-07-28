// src/commands.rs
use anyhow::Result;
use poise::serenity_prelude as serenity;
use tracing::{error, info};

use crate::{messages::*, Context, Error};


#[poise::command(prefix_command, slash_command)]
pub async fn ping(ctx: Context<'_>) -> Result<(), Error> {
    info!("Ping command called by {}", ctx.author().name);
    ctx.send(poise::CreateReply::default()
        .content("🏓 Pong! Bot is working!")
        .ephemeral(true))
        .await?;
    Ok(())
}


#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MANAGE_ROLES"
)]
pub async fn setup_roles(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("This command can only be used in a guild")?;
    
    ctx.defer().await?;
    
    let role_manager = ctx.data().role_manager.read().await;
    
    match role_manager.create_roles_in_guild(&ctx.serenity_context().http, guild_id).await {
        Ok(created_roles) => {
            let role_list = created_roles
                .iter()
                .map(|(name, id)| format!("• {} ({})", name, id))
                .collect::<Vec<_>>()
                .join("\n");
            
            let embed = serenity::CreateEmbed::new()
                .title("✅ Role Setup Complete")
                .description(format!("Successfully processed {} roles:\n\n{}", created_roles.len(), role_list))
                .color(0x00ff00);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            info!("Setup {} roles for guild {}", created_roles.len(), guild_id);
        }
        Err(e) => {
            let embed = serenity::CreateEmbed::new()
                .title("❌ Role Setup Failed")
                .description(format!("Failed to setup roles: {}", e))
                .color(0xff0000);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            error!("Failed to setup roles for guild {}: {}", guild_id, e);
        }
    }
    
    Ok(())
}

#[poise::command(
    slash_command,
    guild_only,
    required_permissions = "MANAGE_ROLES",
)]
pub async fn update_roles(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("This command can only be used in a guild")?;
    
    ctx.defer().await?;
    
    let role_manager = ctx.data().role_manager.read().await;
    
    match role_manager.update_roles_in_guild(&ctx.serenity_context().http, guild_id).await {
        Ok(updated_roles) => {
            let role_list = updated_roles
                .iter()
                .map(|(name, id)| format!("• {} ({})", name, id))
                .collect::<Vec<_>>()
                .join("\n");
            
            let embed = serenity::CreateEmbed::new()
                .title("✅ Roles Updated")
                .description(format!("Successfully updated {} roles:\n\n{}", updated_roles.len(), role_list))
                .color(0x00ff00);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            info!("Updated {} roles for guild {}", updated_roles.len(), guild_id);
        }
        Err(e) => {
            let embed = serenity::CreateEmbed::new()
                .title("❌ Role Update Failed")
                .description(format!("Failed to update roles: {}", e))
                .color(0xff0000);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            error!("Failed to update roles for guild {}: {}", guild_id, e);
        }
    }
    
    Ok(())
}

#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
)]
pub async fn reload_role_config(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;
    
    let mut role_manager = ctx.data().role_manager.write().await;
    
    match role_manager.reload_config().await {
        Ok(true) => {
            let embed = serenity::CreateEmbed::new()
                .title("✅ Configuration Reloaded")
                .description("Role configuration has been successfully reloaded from file.")
                .color(0x00ff00);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            info!("Role configuration reloaded successfully");
        }
        Ok(false) => {
            let embed = serenity::CreateEmbed::new()
                .title("⚠️ No Config File")
                .description("No configuration file path available for reload.")
                .color(0xffaa00);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
        }
        Err(e) => {
            let embed = serenity::CreateEmbed::new()
                .title("❌ Reload Failed")
                .description(format!("Failed to reload configuration: {}", e))
                .color(0xff0000);
            
            ctx.send(poise::CreateReply::default().embed(embed)).await?;
            error!("Failed to reload role configuration: {}", e);
        }
    }
    
    Ok(())
}

#[poise::command(
    slash_command,
    required_permissions = "ADMINISTRATOR",
)]
pub async fn list_role_configs(ctx: Context<'_>) -> Result<(), Error> {
    let role_manager = ctx.data().role_manager.read().await;
    let configs = role_manager.get_role_configs();
    
    if configs.is_empty() {
        let embed = serenity::CreateEmbed::new()
            .title("📋 Role Configurations")
            .description("No role configurations found.")
            .color(0xffaa00);
        
        ctx.send(poise::CreateReply::default().embed(embed)).await?;
        return Ok(());
    }
    
    let config_list = configs
        .iter()
        .map(|config| {
            let category = if config.category.is_empty() {
                "No category".to_string()
            } else {
                config.category.clone()
            };
            
            format!(
                "**{}**\n├ Category: {}\n├ Channels: {}\n├ Color: {}\n└ Mentionable: {}\n",
                config.role,
                category,
                config.channels.len(),
                config.color.as_ref().unwrap_or(&"Default".to_string()),
                if config.mentionable { "Yes" } else { "No" }
            )
        })
        .collect::<Vec<_>>()
        .join("\n");
    
    // Split into chunks if too long for Discord
    let chunks = if config_list.len() > 4000 {
        let mut chunks = Vec::new();
        let mut current_chunk = String::new();
        
        for config in configs {
            let config_str = format!(
                "**{}** - Category: {}, Channels: {}\n",
                config.role,
                if config.category.is_empty() { "None" } else { &config.category },
                config.channels.len()
            );
            
            if current_chunk.len() + config_str.len() > 4000 {
                chunks.push(current_chunk);
                current_chunk = config_str;
            } else {
                current_chunk.push_str(&config_str);
            }
        }
        
        if !current_chunk.is_empty() {
            chunks.push(current_chunk);
        }
        
        chunks
    } else {
        vec![config_list]
    };
    
    for (i, chunk) in chunks.iter().enumerate() {
        let title = if chunks.len() > 1 {
            format!("📋 Role Configurations ({}/{})", i + 1, chunks.len())
        } else {
            "📋 Role Configurations".to_string()
        };
        
        let embed = serenity::CreateEmbed::new()
            .title(title)
            .description(chunk)
            .color(0x3498db)
            .footer(serenity::CreateEmbedFooter::new(format!("Total: {} roles", configs.len())));
        
        ctx.send(poise::CreateReply::default().embed(embed)).await?;
    }
    
    Ok(())
}
#[poise::command(prefix_command, slash_command)]
pub async fn verify(ctx: Context<'_>) -> Result<(), Error> {
    let discord_id = ctx.author().id;
    let verification_manager = ctx.data().guild_manager.get_verification_manager();

    if ctx.guild_id().is_none() {
        info!("Verify command called in DM by {}", ctx.author().name);
        // This is a DM - handle verification directly
        return handle_dm_verify_command(ctx).await;
    }

    if verification_manager.is_user_verified(discord_id) {
        ctx.send(poise::CreateReply::default()
            .content("✅ You are already verified!")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    if verification_manager.is_pending_verification(discord_id) {
        ctx.send(poise::CreateReply::default()
            .content("🔄 You already have a pending verification. Please check your DMs.")
            .ephemeral(true))
            .await?;
        return Ok(());
    }


    start_verification_for_user(ctx).await
}

#[poise::command(prefix_command, slash_command, guild_only)]
pub async fn list_users(ctx: Context<'_>) -> Result<(), Error> {
    if !has_admin_permissions(ctx).await? {
        ctx.send(poise::CreateReply::default()
            .content("❌ You don't have permission to use this command!")
            .ephemeral(true))
            .await?;
        return Ok(());
    }

    let verification_manager = ctx.data().guild_manager.get_verification_manager();
    let users = verification_manager.get_all_users();

    if users.is_empty() {
        ctx.say("No users found in database.").await?;
        return Ok(());
    }

    let mut description = String::new();
    for user in users.iter().take(20) {
        description.push_str(&format!("**{}** (ID: {})\n", user.name, user.id));
    }

    let embed = serenity::CreateEmbed::new()
        .title("User Database")
        .description(description)
        .color(0x0099ff);

    ctx.send(poise::CreateReply::default().embed(embed)).await?;
    Ok(())
}

#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
pub async fn setup_channels(ctx: Context<'_>) -> Result<(), Error> {
    let guild_id = ctx.guild_id().ok_or("This command can only be used in a guild")?;
    
    ctx.defer().await?;
    
    let channel_manager = ctx.data().channel_manager.read().await;
    
    match channel_manager.ensure_channels_exist(&ctx.serenity_context().http, guild_id).await {
        Ok(_) => {
            ctx.say("✅ Successfully setup all configured channels and categories!").await?;
        }
        Err(e) => {
            error!("Failed to setup channels: {}", e);
            ctx.say(format!("❌ Failed to setup channels: {}", e)).await?;
        }
    }
    
    Ok(())
}

/// Reload channel configuration from file
#[poise::command(slash_command, guild_only, required_permissions = "MANAGE_CHANNELS")]
pub async fn reload_channel_config(ctx: Context<'_>) -> Result<(), Error> {
    ctx.defer().await?;
    
    let channel_config_file = std::env::var("CHANNELS_CONFIG")
        .unwrap_or_else(|_| "data/channels.json".to_string());
    
    let mut channel_manager = ctx.data().channel_manager.write().await;
    
    match channel_manager.reload_config(&channel_config_file).await {
        Ok(_) => {
            ctx.say("✅ Channel configuration reloaded successfully!").await?;
        }
        Err(e) => {
            error!("Failed to reload channel config: {}", e);
            ctx.say(format!("❌ Failed to reload channel config: {}", e)).await?;
        }
    }
    
    Ok(())
}

/// List all configured channels and categories
#[poise::command(slash_command, guild_only)]
pub async fn list_channel_configs(ctx: Context<'_>) -> Result<(), Error> {
    let channel_manager = ctx.data().channel_manager.read().await;
    let configs = channel_manager.get_configs();
    
    if configs.is_empty() {
        ctx.say("No channel configurations found.").await?;
        return Ok(());
    }
    
    let mut response = String::from("**Configured Channels:**\n");
    
    for config in configs {
        response.push_str(&format!("• **{}** ({})\n", config.name, format!("{:?}", config.channel_type)));
        
        if !config.channels.is_empty() {
            for sub_channel in &config.channels {
                response.push_str(&format!("  ├─ {} ({})\n", sub_channel.name, format!("{:?}", sub_channel.channel_type)));
            }
        }
    }
    
    ctx.say(response).await?;
    Ok(())
}

async fn has_admin_permissions(ctx: Context<'_>) -> Result<bool> {
    let guild_id = match ctx.guild_id() {
        Some(id) => id,
        None => return Ok(false),
    };

    let member = guild_id.member(&ctx.http(), ctx.author().id).await?;
    let guild = guild_id.to_partial_guild(&ctx.http()).await?;
    let permissions = guild.member_permissions(&member);

    Ok(permissions.administrator())
}

// Helper functions (simplified versions of your original functions)
async fn handle_dm_verify_command(ctx: Context<'_>) -> Result<(), Error> {
    let verification_manager = ctx.data().guild_manager.get_verification_manager();
    
    if verification_manager.is_user_verified(ctx.author().id) {
        ctx.say("✅ You are already verified!").await?;
        return Ok(());
    }

    verification_manager.add_pending_verification(ctx.author().id, ctx.channel_id());
    ctx.say(verification_message(&ctx.author().name)).await?;
    Ok(())
}

async fn start_verification_for_user(ctx: Context<'_>) -> Result<(), Error> {
    let reply = ctx.send(poise::CreateReply::default()
        .content("📨 **Verification Process Started**\n\nI've sent you a private message with verification instructions.")
        .ephemeral(true))
        .await?;

    match ctx.author().create_dm_channel(&ctx.http()).await {
        Ok(dm_channel) => {
            ctx.data().guild_manager.get_verification_manager()
                .add_pending_verification(ctx.author().id, dm_channel.id);

            dm_channel
                .send_message(&ctx.http(), serenity::CreateMessage::new()
                    .content(verification_message(&ctx.author().name)))
                .await?;
        }
        Err(e) => {
            reply.edit(ctx, poise::CreateReply::default()
                .content("❌ **Verification Failed**\n\nI couldn't send you a private message.")
                .ephemeral(true))
                .await?;
            return Err(e.into());
        }
    }

    Ok(())
}

pub async fn handle_dm_message(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    guild_manager: &crate::guild_manager::GuildManager,
) -> Result<(), Error> {
    let verification_manager = guild_manager.get_verification_manager();
    
    if !verification_manager.is_pending_verification(msg.author.id) {
        return Ok(());
    }

    let user_id = msg.content.trim();
    
    match verification_manager.find_user_by_id(user_id) {
        Some(user_data) => {
            verification_manager.complete_verification(msg.author.id, user_data.clone());
            
            msg.channel_id
                .send_message(&ctx.http, serenity::CreateMessage::new()
                    .content(success_message(&user_data.name)))
                .await?;

            // Update user in all guilds
            let guild_ids: Vec<serenity::GuildId> = ctx.cache.guilds().into_iter().collect();
            for guild_id in guild_ids {
                if let Ok(_member) = guild_id.member(&ctx.http, msg.author.id).await {
                    if let Err(e) = guild_manager
                        .update_user_in_guild(&ctx.http, &ctx.cache, guild_id, msg.author.id, &user_data)
                        .await
                    {
                        error!("Failed to update user {} in guild {}: {}", msg.author.name, guild_id, e);
                    }
                }
            }
        }
        None => {
            msg.channel_id
                .send_message(&ctx.http, serenity::CreateMessage::new()
                    .content(error_message(user_id)))
                .await?;
        }
    }

    Ok(())
}
