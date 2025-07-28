// src/main/rs
use anyhow::Result;
use dotenv::dotenv;
use messages::welcome_message;
use poise::serenity_prelude as serenity;
use std::env;
use tracing::{debug, error, info, warn};

mod commands;
mod guild_manager;
mod messages;
mod permissions;
mod verification;
mod role_manager;
mod channel_manager;

use channel_manager::SharedChannelManager;
use commands::*;
use guild_manager::GuildManager;
use permissions::PermissionManager;
use verification::VerificationManager;
use role_manager::SharedRoleManager;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Debug)]
pub struct Data {
    pub guild_manager: GuildManager,
    pub role_manager: SharedRoleManager,
    pub channel_manager: SharedChannelManager,
}

async fn clear_all_commands(
    ctx: &serenity::Context,
    guild_id: serenity::GuildId,
) -> Result<(), Error> {
    info!("Clearing all existing commands for guild: {}", guild_id);
    
    // Get all existing commands
    let existing_commands = match guild_id.get_commands(&ctx.http).await {
        Ok(commands) => commands,
        Err(e) => {
            error!("Failed to fetch existing commands for guild {}: {}", guild_id, e);
            return Err(e.into());
        }
    };
    
    info!("Found {} existing commands in guild {}", existing_commands.len(), guild_id);
    
    // Delete each command individually
    for command in existing_commands {
        info!("Deleting command: {} (ID: {})", command.name, command.id);
        if let Err(e) = guild_id.delete_command(&ctx.http, command.id).await {
            error!("Failed to delete command {} ({}): {}", command.name, command.id, e);
        } else {
            info!("Successfully deleted command: {}", command.name);
        }
    }
    
    // Also clear global commands if any exist (usually not needed for guild bots)
    let global_commands = match ctx.http.get_global_commands().await {
        Ok(commands) => commands,
        Err(e) => {
            warn!("Failed to fetch global commands: {}", e);
            Vec::new()
        }
    };
    
    if !global_commands.is_empty() {
        info!("Found {} global commands, clearing them", global_commands.len());
        for command in global_commands {
            info!("Deleting global command: {} (ID: {})", command.name, command.id);
            if let Err(e) = ctx.http.delete_global_command(command.id).await {
                error!("Failed to delete global command {} ({}): {}", command.name, command.id, e);
            } else {
                info!("Successfully deleted global command: {}", command.name);
            }
        }
    }
    
    info!("Finished clearing commands for guild: {}", guild_id);
    Ok(())
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            if new_message.guild_id.is_none() && !new_message.author.bot {
                debug!("Processing DM message from user: {}", new_message.author.name);
                if let Err(e) = handle_dm_message(ctx, new_message, &data.guild_manager).await {
                    error!("Failed to handle DM message: {}", e);
                }
            }
        }
        serenity::FullEvent::GuildMemberAddition { new_member } => {
            // Find welcome channel
            match find_welcome_channel(ctx, new_member.guild_id).await {
                Ok(welcome_channel) => {
                    debug!("Found welcome channel: {} ({})", welcome_channel.name, welcome_channel.id);
                    let msg= "TODO:Welcome message commented out since i didn't get single person messages working. 
                                            I just sent a single message that everyone can read in welcome ";

                    // TODO: make a single welcome message that get's modified.
                    // TODO: Reenable welcome message once everything is up and running.
                    // Send welcome message in welcome channel (user-only visible)
                    // let welcome_embed = serenity::CreateEmbed::new()
                    //     .title("🎉 Welcome to the Server!")
                    //     .description(welcome_message(&new_member.user.name))
                    //     .color(0x00ff00);
                    //     .author(author);
                    // let welcome_message = serenity::CreateMessage::new()
                    //     .embed(welcome_embed)
                    //     .allowed_mentions(serenity::CreateAllowedMentions::new().users([new_member.user.id]));
                    // match welcome_channel.send_message(&ctx.http, welcome_message).await {
                    //     Ok(_) => {
                    //         info!("Sent welcome message to {} in welcome channel", new_member.user.name);
                    //     }
                    //     Err(e) => {
                    //         error!("Failed to send welcome message to {} in welcome channel: {}", new_member.user.name, e);
                    //     }
                    // }
                }
                Err(e) => {
                    error!("Failed to find welcome channel in guild {}: {}", new_member.guild_id, e);
                }
            };
            if let Err(e) = data.guild_manager.handle_new_member(ctx, new_member).await {
                error!("Failed to handle new member: {}", e);
            }
        }
        serenity::FullEvent::GuildCreate { guild, .. } => {
            // Auto-setup roles when bot joins a new guild or on startup
            // info!("Setting up roles for guild: {} ({})", guild.name, guild.id);
            warn!("Implement startup functionality for guild: {} ({})", guild.name, guild.id);
            
            // let channel_manager = data.channel_manager.read().await;
            // if let Err(e) = channel_manager.ensure_channels_exist(&ctx.http, guild.id).await {
            //     error!("Failed to setup channels for guild {}: {}", guild.id, e);
            // } else {
            //     info!("Successfully setup channels for guild: {}", guild.id);
            // }
            
            // // Use the role manager to setup roles
            // let role_manager = data.role_manager.read().await;
            // if let Err(e) = role_manager.create_roles_in_guild(&ctx.http, guild.id).await {
            //     error!("Failed to setup roles for guild {}: {}", guild.id, e);
            // } else {
            //     info!("Successfully setup roles for guild: {}", guild.id);
            // }
            
            // Also call the existing guild manager setup if needed
            if let Err(e) = data.guild_manager.setup_guild_roles(&ctx.http, guild.id).await {
                error!("Failed to setup guild roles via guild manager for guild {}: {}", guild.id, e);
            }
        }
        _ => {}
    }
    Ok(())
}

async fn find_welcome_channel(
    ctx: &serenity::Context,
    guild_id: serenity::GuildId,
) -> Result<serenity::GuildChannel, Error> {
    debug!("Looking for welcome channel in guild: {}", guild_id);
    
    let channels = guild_id.channels(&ctx.http).await?;
    
    // Look for channels with welcome-related names
    let welcome_name = "welcome";
    
    if let Some((_, channel)) = channels.iter().find(|(_, ch)| {
        ch.kind == serenity::ChannelType::Text && ch.name.to_lowercase() == welcome_name
    }) {
        debug!("Found welcome channel: {} ({})", channel.name, channel.id);
        return Ok(channel.clone());
    }
    
    // If no specific welcome channel found, try to find the first text channel
    if let Some((_, channel)) = channels.iter().find(|(_, ch)| {
        ch.kind == serenity::ChannelType::Text
    }) {
        warn!("No welcome channel found, using first text channel: {} ({})", channel.name, channel.id);
        return Ok(channel.clone());
    }
    
    Err(anyhow::anyhow!("No suitable welcome channel found").into())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let token = env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN environment variable");

    // Load role configuration
    let role_config_file = env::var("ROLES_CONFIG")
        .unwrap_or_else(|_| "data/roles.json".to_string());
    
    info!("Loading role configuration from: {}", role_config_file);
    let role_manager = role_manager::create_shared_role_manager(&role_config_file).await?;
    
    // Load channel configuration
    let channel_config_file = env::var("CHANNELS_CONFIG")
        .unwrap_or_else(|_| "data/channels.json".to_string());
    
    info!("Loading channel configuration from: {}", channel_config_file);
    let channel_manager = channel_manager::create_shared_channel_manager(&channel_config_file).await?;
    
    // Initialize other managers
    let verification_manager = VerificationManager::new();
    verification_manager.load_database().await?;

    let permission_manager = PermissionManager::new().await?;
    let guild_manager = GuildManager::new(verification_manager, permission_manager).await?;

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                ping(), 
                verify(), 
                // list_users(),
                // setup_roles(),
                // update_roles(),
                // reload_role_config(),
                // list_role_configs(),
                // setup_channels(),
                // reload_channel_config(),
                // list_channel_configs(),
            ],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(|ctx, ready, framework| {
            Box::pin(async move {
                info!("Bot logged in as: {}", ready.user.name);
                
                for guild in &ready.guilds {
                    info!("Processing guild: {} ({})", guild.id, guild.id);
                    
                    // Clear all existing commands first
                    if let Err(e) = clear_all_commands(ctx, guild.id).await {
                        error!("Failed to clear commands for guild {}: {}", guild.id, e);
                        // Continue anyway to try registering new commands
                    }
                    
                    // Wait a bit for Discord to process the deletions
                    tokio::time::sleep(tokio::time::Duration::from_millis(1000)).await;
                    
                    // Register new commands
                    info!("Registering new commands for guild: {}", guild.id);
                    if let Err(e) = poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        guild.id,
                    ).await {
                        error!("Failed to register commands for guild {}: {}", guild.id, e);
                    } else {
                        info!("Successfully registered {} commands for guild: {}", 
                              framework.options().commands.len(), guild.id);
                    }
                }

                Ok(Data { 
                    guild_manager,
                    role_manager: role_manager.clone(),
                    channel_manager: channel_manager.clone(),
                })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    info!("Starting bot...");
    client.start().await?;

    Ok(())
}