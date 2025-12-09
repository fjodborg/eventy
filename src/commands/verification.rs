use poise::serenity_prelude as serenity;
use tracing::{debug, error, info};

use crate::{Context, Data, Error};

/// Start the verification process
///
/// Use this command to verify your identity. You'll need your personal verification ID.
#[poise::command(prefix_command, slash_command)]
pub async fn verify(
    ctx: Context<'_>,
    #[description = "Your personal verification ID (UUID)"] verification_id: Option<String>,
) -> Result<(), Error> {
    let user_id = ctx.author().id;
    let verification_manager = &ctx.data().verification_manager;

    // Check if already verified
    if verification_manager.is_verified(user_id).await {
        ctx.send(
            poise::CreateReply::default()
                .content("You are already verified!")
                .ephemeral(true),
        )
        .await?;
        return Ok(());
    }

    // Check if OAuth web server is configured
    let base_url = std::env::var("WEB_BASE_URL").ok();

    match (verification_id, base_url) {
        // If user provided a verification ID and we have a web URL, give them the direct link
        (Some(uuid), Some(url)) => {
            // Validate the UUID exists in our config
            let user_exists = {
                let config = ctx.data().config_manager.read().await;
                config.find_user_by_verification_id(&uuid).is_some()
            };

            if !user_exists {
                ctx.send(poise::CreateReply::default()
                    .content("**Invalid Verification ID**\n\nThis ID was not found in our records. Please check your ID and try again.")
                    .ephemeral(true))
                    .await?;
                return Ok(());
            }

            let verify_url = format!("{}/verify/{}", url, uuid);

            let embed = serenity::CreateEmbed::new()
                .title("Verify Your Account")
                .description("Click the link below to verify with Discord:")
                .field("Verification Link", &verify_url, false)
                .field("Instructions", "1. Click the link\n2. Login with Discord\n3. Authorize the bot\n4. You're verified!", false)
                .color(0x5865F2);

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;

            info!(
                "Sent OAuth verification link to {} for UUID {}",
                ctx.author().name,
                uuid
            );
        }

        // If we have a web URL but no ID provided, tell them to use the link they received
        (None, Some(_url)) => {
            let embed = serenity::CreateEmbed::new()
                .title("Verification Required")
                .description("To verify your account, you need your personal verification ID.")
                .field("Option 1: Use Your Link", "If you received a verification link, click it to verify.", false)
                .field("Option 2: Provide Your ID", "Run `/verify <your-id>` with the UUID you were given.\n\nExample: `/verify 12345678-1234-1234-1234-123456789abc`", false)
                .color(0x5865F2);

            ctx.send(poise::CreateReply::default().embed(embed).ephemeral(true))
                .await?;
        }

        // Fallback to DM-based verification if web server not configured
        (uuid_opt, None) => {
            // Use the old DM-based verification flow
            if let Some(uuid) = uuid_opt {
                // User provided UUID, attempt verification directly
                let result = verification_manager
                    .attempt_verification(user_id, &uuid)
                    .await;

                if result.success {
                    ctx.send(
                        poise::CreateReply::default()
                            .content(success_message(&result.display_name))
                            .ephemeral(true),
                    )
                    .await?;

                    // Apply roles in current guild
                    if let Some(guild_id) = ctx.guild_id() {
                        if let Ok(member) = guild_id.member(&ctx.http(), user_id).await {
                            // Set nickname
                            if let Err(e) = member
                                .clone()
                                .edit(
                                    &ctx.http(),
                                    serenity::EditMember::new().nickname(&result.display_name),
                                )
                                .await
                            {
                                error!("Failed to set nickname: {}", e);
                            }

                            // Assign roles
                            let role_manager = ctx.data().role_manager.read().await;
                            for role_name in &result.roles_to_assign {
                                if let Err(e) = role_manager
                                    .assign_role_to_user(&ctx.http(), guild_id, user_id, role_name)
                                    .await
                                {
                                    error!("Failed to assign role '{}': {}", role_name, e);
                                }
                            }
                        }
                    }

                    // Save database
                    if let Err(e) = verification_manager
                        .save_database("state/user_database.json")
                        .await
                    {
                        error!("Failed to save user database: {}", e);
                    }

                    info!("User {} verified as '{}'", user_id, result.display_name);
                } else {
                    let error_msg = result.error.unwrap_or_else(|| "Unknown error".to_string());
                    ctx.send(
                        poise::CreateReply::default()
                            .content(error_message(&error_msg))
                            .ephemeral(true),
                    )
                    .await?;
                }
            } else {
                // No UUID provided, start DM-based flow
                // Check if already pending
                if verification_manager.is_pending(user_id) {
                    ctx.send(
                        poise::CreateReply::default()
                            .content(
                                "You already have a pending verification. Please check your DMs.",
                            )
                            .ephemeral(true),
                    )
                    .await?;
                    return Ok(());
                }

                if ctx.guild_id().is_none() {
                    verification_manager.start_verification(user_id, ctx.channel_id(), None);
                    ctx.say(verification_message(&ctx.author().name)).await?;
                    return Ok(());
                }

                ctx.send(poise::CreateReply::default()
                    .content("**Verification Process Started**\n\nI've sent you a private message with verification instructions.")
                    .ephemeral(true))
                    .await?;

                match ctx.author().create_dm_channel(&ctx.http()).await {
                    Ok(dm_channel) => {
                        verification_manager.start_verification(
                            user_id,
                            dm_channel.id,
                            ctx.guild_id(),
                        );
                        dm_channel
                            .send_message(
                                &ctx.http(),
                                serenity::CreateMessage::new()
                                    .content(verification_message(&ctx.author().name)),
                            )
                            .await?;
                        info!("Sent verification DM to {}", ctx.author().name);
                    }
                    Err(e) => {
                        error!(
                            "Failed to create DM channel for {}: {}",
                            ctx.author().name,
                            e
                        );
                        ctx.send(poise::CreateReply::default()
                            .content("**Verification Failed**\n\nI couldn't send you a private message. Please enable DMs from server members and try again.")
                            .ephemeral(true))
                            .await?;
                    }
                }
            }
        }
    }

    Ok(())
}

/// Handle a DM message for verification
pub async fn handle_dm_verification(
    ctx: &serenity::Context,
    msg: &serenity::Message,
    data: &Data,
) -> Result<(), Error> {
    let user_id = msg.author.id;
    let verification_manager = &data.verification_manager;

    // Check if user has pending verification
    if !verification_manager.is_pending(user_id) {
        return Ok(());
    }

    let provided_id = msg.content.trim();
    debug!(
        "Processing verification attempt from {} with ID: {}",
        msg.author.name, provided_id
    );

    let result = verification_manager
        .attempt_verification(user_id, provided_id)
        .await;

    if result.success {
        // Send success message
        msg.channel_id
            .send_message(
                &ctx.http,
                serenity::CreateMessage::new().content(success_message(&result.display_name)),
            )
            .await?;

        // Apply roles in all guilds the bot shares with the user
        let guild_ids: Vec<serenity::GuildId> = ctx.cache.guilds();
        for guild_id in guild_ids {
            if let Ok(member) = guild_id.member(&ctx.http, user_id).await {
                // Try to set nickname
                if let Err(e) = member
                    .clone()
                    .edit(
                        &ctx.http,
                        serenity::EditMember::new().nickname(&result.display_name),
                    )
                    .await
                {
                    error!(
                        "Failed to set nickname for {} in guild {}: {}",
                        user_id, guild_id, e
                    );
                }

                // Assign roles
                let role_manager = data.role_manager.read().await;
                for role_name in &result.roles_to_assign {
                    if let Err(e) = role_manager
                        .assign_role_to_user(&ctx.http, guild_id, user_id, role_name)
                        .await
                    {
                        error!(
                            "Failed to assign role '{}' to {} in guild {}: {}",
                            role_name, user_id, guild_id, e
                        );
                    }
                }
            }
        }

        // Save the user database
        if let Err(e) = verification_manager
            .save_database("state/user_database.json")
            .await
        {
            error!("Failed to save user database: {}", e);
        }

        info!("User {} verified as '{}'", user_id, result.display_name);
    } else {
        // Send error message
        let error_msg = result.error.unwrap_or_else(|| "Unknown error".to_string());
        msg.channel_id
            .send_message(
                &ctx.http,
                serenity::CreateMessage::new().content(error_message(&error_msg)),
            )
            .await?;
    }

    Ok(())
}

fn verification_message(username: &str) -> String {
    format!(
        "**Welcome, {}!**\n\n\
        To verify your identity, please reply with your **personal ID** that was provided to you.\n\n\
        This ID is unique to you and allows us to confirm your membership.\n\n\
        Just type the ID and send it as a message here.",
        username
    )
}

fn success_message(name: &str) -> String {
    format!(
        "**Verification Successful!**\n\n\
        Welcome, **{}**! Your identity has been confirmed.\n\n\
        You now have access to the server. Your nickname and roles have been updated.",
        name
    )
}

fn error_message(error: &str) -> String {
    format!(
        "**Verification Failed**\n\n\
        {}\n\n\
        Please check your ID and try again. If you continue to have issues, please contact an administrator.",
        error
    )
}
