use anyhow::Result;
use clap::Parser;
use dotenv::dotenv;
use poise::serenity_prelude as serenity;
use tracing::{error, info, warn};

/// Discord bot for choir member verification and management
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Force re-sync of slash commands to all guilds (use when commands aren't showing up)
    #[arg(long, short = 's')]
    sync_commands: bool,

    /// Register commands per-guild instead of globally (faster for testing)
    #[arg(long)]
    guild_commands: bool,

    /// Specific guild ID to sync commands to (for testing)
    #[arg(long)]
    guild_id: Option<u64>,
}

mod commands;
mod config;
mod error;
mod events;
mod managers;

mod state;
mod web;

use commands::{
    get_config, help, ping, set_config_global, set_config_season, update_category, verify,
};
use events::message::handle_message;
use events::{handle_guild_create, handle_member_add};
use managers::{
    create_shared_channel_manager, create_shared_config_manager, create_shared_maintainers_manager,
    create_shared_role_manager, create_shared_verification_manager, SharedChannelManager,
    SharedConfigManager, SharedMaintainersManager, SharedRoleManager, SharedVerificationManager,
};
use state::{
    create_shared_channel_state, create_shared_user_database, ChannelState, SharedChannelState,
    SharedUserDatabase, UserDatabase,
};

type Error = Box<dyn std::error::Error + Send + Sync>;
type Context<'a> = poise::Context<'a, Data, Error>;

/// Shared application state
pub struct Data {
    pub config_manager: SharedConfigManager,
    pub channel_state: SharedChannelState,
    pub user_database: SharedUserDatabase,
    pub role_manager: SharedRoleManager,
    pub channel_manager: SharedChannelManager,
    pub verification_manager: SharedVerificationManager,
    pub maintainers_manager: SharedMaintainersManager,
}

async fn event_handler(
    ctx: &serenity::Context,
    event: &serenity::FullEvent,
    _framework: poise::FrameworkContext<'_, Data, Error>,
    data: &Data,
) -> Result<(), Error> {
    match event {
        serenity::FullEvent::Message { new_message } => {
            if let Err(e) = handle_message(ctx, new_message, data).await {
                error!("Failed to handle message: {}", e);
            }
        }
        serenity::FullEvent::GuildMemberAddition { new_member } => {
            if let Err(e) = handle_member_add(ctx, new_member, data).await {
                error!("Failed to handle new member: {}", e);
            }
        }
        serenity::FullEvent::GuildCreate { guild, .. } => {
            if let Err(e) = handle_guild_create(ctx, guild, data).await {
                error!("Failed to handle guild create: {}", e);
            }
        }
        _ => {}
    }
    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    dotenv().ok();
    let args = Args::parse();

    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    let token = std::env::var("DISCORD_TOKEN").expect("Missing DISCORD_TOKEN environment variable");

    // Extract bot/application ID from token (first part before the dot, base64 encoded)
    if let Some(bot_id_b64) = token.split('.').next() {
        // Discord tokens use URL-safe base64 without padding
        use base64::Engine;
        match base64::engine::general_purpose::STANDARD_NO_PAD.decode(bot_id_b64) {
            Ok(decoded) => {
                if let Ok(id_str) = String::from_utf8(decoded) {
                    info!("Bot ID: {} (configure intents at https://discord.com/developers/applications/{}/bot)", id_str, id_str);
                }
            }
            Err(_) => {
                // Try URL-safe variant
                if let Ok(decoded) =
                    base64::engine::general_purpose::URL_SAFE_NO_PAD.decode(bot_id_b64)
                {
                    if let Ok(id_str) = String::from_utf8(decoded) {
                        info!("Bot ID: {} (configure intents at https://discord.com/developers/applications/{}/bot)", id_str, id_str);
                    }
                }
            }
        }
    }

    let data_path = std::env::var("DATA_PATH").unwrap_or_else(|_| "data".to_string());
    let state_path = std::env::var("STATE_PATH").unwrap_or_else(|_| "state".to_string());

    // Ensure state directory exists
    tokio::fs::create_dir_all(&state_path).await.ok();

    // Load state
    info!("Loading channel state...");
    let channel_state_path = format!("{}/channel_state.json", state_path);
    let channel_state = ChannelState::load(&channel_state_path)
        .await
        .unwrap_or_else(|e| {
            warn!("Could not load channel state: {}, using empty state", e);
            ChannelState::new()
        });
    let shared_channel_state = create_shared_channel_state(channel_state);

    info!("Loading user database...");
    let user_db_path = format!("{}/user_database.json", state_path);
    let user_database = UserDatabase::load(&user_db_path).await.unwrap_or_else(|e| {
        warn!("Could not load user database: {}, using empty database", e);
        UserDatabase::new()
    });
    let shared_user_database = create_shared_user_database(user_database);

    // Create config manager and load configs
    info!("Loading configurations from {}...", data_path);
    let config_manager = create_shared_config_manager(&data_path);
    {
        let mut cm: tokio::sync::RwLockWriteGuard<'_, managers::config_manager::ConfigManager> =
            config_manager.write().await;
        if let Err(e) = cm.load_all().await {
            error!("Failed to load configurations: {}", e);
        }
    }

    // Create managers
    let role_manager = create_shared_role_manager(shared_channel_state.clone());
    let channel_manager = create_shared_channel_manager(
        shared_channel_state.clone(),
        role_manager.clone(),
        config_manager.clone(),
    );
    let verification_manager =
        create_shared_verification_manager(shared_user_database.clone(), config_manager.clone());
    let maintainers_manager =
        create_shared_maintainers_manager(config_manager.clone(), channel_manager.clone());

    // Extract CLI flags for use in setup
    let sync_commands = args.sync_commands;
    let guild_commands = args.guild_commands;
    let target_guild_id = args.guild_id;

    if sync_commands {
        info!("--sync-commands: Will force re-register slash commands");
    }
    if guild_commands {
        info!("--guild-commands: Will register commands per-guild (faster for testing)");
    } else {
        info!("Registering commands globally by default (takes up to 1 hour to propagate)");
    }
    if let Some(gid) = target_guild_id {
        info!("--guild-id: Targeting specific guild {}", gid);
    }

    // Build framework
    let framework = poise::Framework::builder()
        .options(poise::FrameworkOptions {
            commands: vec![
                ping(),
                help(),
                verify(),
                get_config(),
                set_config_global(),
                set_config_season(),
                update_category(),
            ],
            event_handler: |ctx, event, framework, data| {
                Box::pin(event_handler(ctx, event, framework, data))
            },
            pre_command: |ctx| {
                Box::pin(async move {
                    info!(
                        "Command '{}' invoked by {} (ID: {}) in {}",
                        ctx.command().qualified_name,
                        ctx.author().name,
                        ctx.author().id,
                        ctx.guild_id().map(|g| g.to_string()).unwrap_or_else(|| "DM".to_string())
                    );
                })
            },
            post_command: |ctx| {
                Box::pin(async move {
                    info!(
                        "Command '{}' completed for {}",
                        ctx.command().qualified_name,
                        ctx.author().name
                    );
                })
            },
            on_error: |error| {
                Box::pin(async move {
                    match error {
                        poise::FrameworkError::Command { error, ctx, .. } => {
                            error!("Error in command '{}': {}", ctx.command().qualified_name, error);
                            let _ = ctx.say(format!("An error occurred: {}", error)).await;
                        }
                        poise::FrameworkError::ArgumentParse { error, input, ctx, .. } => {
                            error!("Argument parse error in '{}': {} (input: {:?})", ctx.command().qualified_name, error, input);
                        }
                        poise::FrameworkError::MissingBotPermissions { missing_permissions, ctx, .. } => {
                            error!("Bot missing permissions for '{}': {:?}", ctx.command().qualified_name, missing_permissions);
                            let _ = ctx.say(format!("Bot is missing permissions: {:?}", missing_permissions)).await;
                        }
                        poise::FrameworkError::MissingUserPermissions { missing_permissions, ctx, .. } => {
                            error!("User {} missing permissions for '{}': {:?}", ctx.author().name, ctx.command().qualified_name, missing_permissions);
                        }
                        poise::FrameworkError::NotAnOwner { ctx, .. } => {
                            error!("User {} tried to use owner command '{}'", ctx.author().name, ctx.command().qualified_name);
                        }
                        poise::FrameworkError::GuildOnly { ctx, .. } => {
                            error!("Command '{}' is guild-only, used in DM by {}", ctx.command().qualified_name, ctx.author().name);
                        }
                        other => {
                            error!("Other framework error: {}", other);
                        }
                    }
                })
            },
            ..Default::default()
        })
        .setup(move |ctx, ready, framework| {
            let config_manager = config_manager.clone();
            let shared_channel_state = shared_channel_state.clone();
            let shared_user_database = shared_user_database.clone();
            let role_manager = role_manager.clone();
            let channel_manager = channel_manager.clone();
            let verification_manager = verification_manager.clone();
            let maintainers_manager = maintainers_manager.clone();

            Box::pin(async move {
                info!("Bot logged in as: {}", ready.user.name);

                // Determine which guilds to register commands for
                let guilds_to_register: Vec<serenity::GuildId> = if let Some(gid) = target_guild_id {
                    // Only register to specific guild
                    vec![serenity::GuildId::new(gid)]
                } else {
                    // Register to all guilds the bot is in
                    ready.guilds.iter().map(|g| g.id).collect()
                };

                if guild_commands || sync_commands {
                    // Register commands per-guild (faster for testing)
                    for guild_id in &guilds_to_register {
                        info!("Registering commands to guild: {}", guild_id);
                        if let Err(e) = poise::builtins::register_in_guild(
                            ctx,
                            &framework.options().commands,
                            *guild_id,
                        ).await {
                            error!("Failed to register commands for guild {}: {}", guild_id, e);
                        } else {
                            info!("Successfully registered {} commands for guild {}",
                                  framework.options().commands.len(), guild_id);
                        }
                    }
                } else {
                    // Default: Register commands globally
                    info!("Registering commands globally...");
                    if let Err(e) = poise::builtins::register_globally(
                        ctx,
                        &framework.options().commands,
                    ).await {
                        error!("Failed to register commands globally: {}", e);
                    } else {
                        info!("Successfully registered {} commands globally (may take up to 1 hour to propagate)",
                              framework.options().commands.len());
                    }
                }

                // Start web server for OAuth verification if configured
                if let Some(oauth_state) = web::OAuthState::from_env() {
                    let web_config = web::WebServerConfig::default();
                    let serenity_http = ctx.http.clone();

                    let web_config_manager = config_manager.clone();
                    let web_role_manager = role_manager.clone();
                    let web_verification_manager = verification_manager.clone();

                    tokio::spawn(async move {
                        info!("Starting OAuth web server on port {}...", web_config.port);
                        if let Err(e) = web::start_web_server(
                            web_config,
                            oauth_state,
                            web_config_manager,
                            web_role_manager,
                            web_verification_manager,
                            serenity_http,
                        ).await {
                            error!("Web server error: {}", e);
                        }
                    });
                } else {
                    warn!("OAuth web server not started: DISCORD_CLIENT_ID or DISCORD_CLIENT_SECRET not set");
                }

                Ok(Data {
                    config_manager,
                    channel_state: shared_channel_state,
                    user_database: shared_user_database,
                    role_manager,
                    channel_manager,
                    verification_manager,
                    maintainers_manager,
                })
            })
        })
        .build();

    let intents = serenity::GatewayIntents::non_privileged()
        | serenity::GatewayIntents::MESSAGE_CONTENT
        | serenity::GatewayIntents::GUILD_MEMBERS;

    // Log which privileged intents we're requesting
    let privileged_intents: Vec<&str> = vec![
        if intents.contains(serenity::GatewayIntents::MESSAGE_CONTENT) {
            Some("MESSAGE_CONTENT")
        } else {
            None
        },
        if intents.contains(serenity::GatewayIntents::GUILD_MEMBERS) {
            Some("GUILD_MEMBERS")
        } else {
            None
        },
        if intents.contains(serenity::GatewayIntents::GUILD_PRESENCES) {
            Some("GUILD_PRESENCES")
        } else {
            None
        },
    ]
    .into_iter()
    .flatten()
    .collect();

    info!("Requesting privileged intents: {:?}", privileged_intents);

    let mut client = serenity::ClientBuilder::new(token, intents)
        .framework(framework)
        .await?;

    info!("Starting bot...");
    if let Err(e) = client.start().await {
        // Check if it's a disallowed intents error
        let err_str = e.to_string();
        if err_str.contains("Disallowed") || err_str.contains("intents") {
            error!("Failed to start bot: {}", e);
            error!("The following privileged intents need to be enabled in the Discord Developer Portal:");
            for intent in &privileged_intents {
                error!("  - {}", intent);
            }
            error!("Go to https://discord.com/developers/applications -> Your App -> Bot -> Privileged Gateway Intents");
            return Err(anyhow::anyhow!(
                "Disallowed gateway intents. Enable these in Discord Developer Portal: {:?}",
                privileged_intents
            ));
        }
        return Err(e.into());
    }
    warn!("Bot ended.");

    Ok(())
}
