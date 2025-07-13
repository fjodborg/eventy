use anyhow::Result;
use dotenv::dotenv;
use messages::{verification_message, welcome_message};
use poise::serenity_prelude as serenity;
use std::env;
use tracing::{error, info, warn, debug};

mod commands;
mod models;
mod user_manager; 
mod messages;

use commands::*;
use user_manager::UserManager;

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

#[derive(Debug)]
pub struct Data {
    pub user_manager: UserManager,
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            // Handle DM messages for verification
            if new_message.guild_id.is_none() && !new_message.author.bot {
                debug!("Processing DM message from user: {}", new_message.author.name);
                if let Err(e) = handle_dm_message(ctx, new_message, &data.user_manager).await {
                    error!("Failed to handle DM message from {}: {}", new_message.author.name, e);
                }
            }
        }
        serenity::FullEvent::GuildMemberAddition { new_member } => {
            info!(
                "New member joined: {} ({}) in guild: {}",
                new_member.user.name, new_member.user.id, new_member.guild_id
            );

            // Check if user is already verified
            if data.user_manager.is_user_verified(new_member.user.id) {
                info!("User {} is already verified, updating access", new_member.user.name);
                // User is already verified, update their access immediately
                if let Some(user_data) = data.user_manager.get_verified_user_data(new_member.user.id) {
                    if let Err(e) = data.user_manager
                        .update_user_in_guild(
                            &ctx.http,
                            &ctx.cache,
                            new_member.guild_id,
                            new_member.user.id,
                            &user_data,
                        )
                        .await
                    {
                        error!("Failed to update verified user {} in guild {}: {}", 
                               new_member.user.name, new_member.guild_id, e);
                    }
                } else {
                    error!("User {} marked as verified but no user data found", new_member.user.name);
                }
            } else {
                info!("Starting verification process for new user: {}", new_member.user.name);
                // Set up unverified user permissions first
                if let Err(e) = data.user_manager
                    .setup_unverified_user_permissions(&ctx.http, new_member.guild_id, new_member.user.id)
                    .await
                {
                    error!("Failed to setup unverified permissions for {}: {}", new_member.user.name, e);
                }

                // Send welcome message in welcome channel and start verification
                if let Err(e) = handle_new_member_verification(ctx, new_member, &data.user_manager).await {
                    error!("Failed to handle new member verification for {}: {}", new_member.user.name, e);
                }
            }
        }
        _ => {}
    }
    Ok(())
}

async fn handle_new_member_verification(
    ctx: &serenity::Context,
    new_member: &serenity::Member,
    user_manager: &UserManager,
) -> Result<(), Error> {
    debug!("Handling new member verification for: {}", new_member.user.name);

    // Check if user is already in verification process
    if user_manager.is_pending_verification(new_member.user.id) {
        warn!("User {} already has pending verification", new_member.user.name);
        return Ok(());
    }

    // Find welcome channel
    let welcome_channel = match find_welcome_channel(ctx, new_member.guild_id).await {
        Ok(channel) => channel,
        Err(e) => {
            error!("Failed to find welcome channel in guild {}: {}", new_member.guild_id, e);
            // Continue with DM verification even if welcome channel not found
            return start_dm_verification_process(ctx, &new_member.user, user_manager).await;
        }
    };

    debug!("Found welcome channel: {} ({})", welcome_channel.name, welcome_channel.id);

    // Send welcome message in welcome channel (user-only visible)
    let welcome_embed = serenity::CreateEmbed::new()
        .title("🎉 Welcome to the Server!")
        .description(welcome_message(&new_member.user.name))
        .color(0x00ff00)
        // .footer(serenity::CreateEmbedFooter::new("This message will disappear after 5 minutes"))
        ;

    let welcome_message = serenity::CreateMessage::new()
        .embed(welcome_embed)
        .allowed_mentions(serenity::CreateAllowedMentions::new().users([new_member.user.id]));

    match welcome_channel.send_message(&ctx.http, welcome_message).await {
        Ok(_) => {
            info!("Sent welcome message to {} in welcome channel", new_member.user.name);

        }
        Err(e) => {
            error!("Failed to send welcome message to {} in welcome channel: {}", new_member.user.name, e);
        }
    }

    // Start DM verification process
    start_dm_verification_process(ctx, &new_member.user, user_manager).await
}

async fn find_welcome_channel(
    ctx: &serenity::Context,
    guild_id: serenity::GuildId,
) -> Result<serenity::GuildChannel, Error> {
    debug!("Looking for welcome channel in guild: {}", guild_id);
    
    let channels = guild_id.channels(&ctx.http).await?;
    
    // Look for channels with welcome-related names
    let welcome_names = ["welcome", "welcomes", "welcome-channel", "general"];
    
    for name in welcome_names {
        if let Some((_, channel)) = channels.iter().find(|(_, ch)| {
            ch.kind == serenity::ChannelType::Text && ch.name.to_lowercase() == name
        }) {
            debug!("Found welcome channel: {} ({})", channel.name, channel.id);
            return Ok(channel.clone());
        }
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

async fn start_dm_verification_process(
    ctx: &serenity::Context,
    user: &serenity::User,
    user_manager: &UserManager,
) -> Result<(), Error> {
    debug!("Starting DM verification process for: {}", user.name);

    // Start verification process via DM
    match user.create_dm_channel(&ctx.http).await {
        Ok(dm_channel) => {
            debug!("Created DM channel for {}: {}", user.name, dm_channel.id);
            user_manager.add_pending_verification(user.id, dm_channel.id);

            match dm_channel
                .send_message(
                    &ctx.http,
                    serenity::CreateMessage::new().content(verification_message(&user.name)),
                )
                .await
            {
                Ok(_) => {
                    info!("Successfully sent verification DM to {}", user.name);
                }
                Err(e) => {
                    error!("Failed to send verification DM to {}: {}", user.name, e);
                    user_manager.remove_pending_verification(user.id);
                    return Err(e.into());
                }
            }
        }
        Err(e) => {
            error!("Failed to create DM channel for {}: {}", user.name, e);
            return Err(e.into());
        }
    }

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();

    let token = env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN environment variable");

    let user_manager = UserManager::new();

    info!("Loading user database...");
    if let Err(e) = user_manager.load_database().await {
        error!("Failed to load user database: {}", e);
        return Err(e);
    }

    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![ping(), verify(), list_users()],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            ..Default::default()
        })
        .setup(|ctx, ready, framework| {
            Box::pin(async move {
                info!("Bot logged in as: {}", ready.user.name);
                info!("Registering commands...");

                if let Some(guild) = ready.guilds.first() {
                    info!("Registering commands for guild: {}", guild.id);
                    if let Err(e) = poise::builtins::register_in_guild(
                        ctx,
                        &framework.options().commands,
                        guild.id,
                    )
                    .await {
                        error!("Failed to register commands: {}", e);
                    } else {
                        info!("Successfully registered commands");
                    }
                }

                let data = Data { user_manager };

                Ok(data)
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

    if let Err(why) = client.start().await {
        error!("Client error: {:?}", why);
    }

    Ok(())
}