use poise::serenity_prelude::{GuildId, Http, Permissions};
use tracing::{error, info, warn};

/// A single permission with its status
#[derive(Debug, Clone)]
pub struct PermissionStatus {
    pub name: &'static str,
    pub description: &'static str,
    pub has_permission: bool,
}

/// All required permissions for the bot
pub fn get_required_permissions() -> Vec<(&'static str, &'static str, Permissions)> {
    vec![
        ("VIEW_CHANNEL", "See channels and categories", Permissions::VIEW_CHANNEL),
        ("SEND_MESSAGES", "Send messages in channels", Permissions::SEND_MESSAGES),
        ("EMBED_LINKS", "Send rich embeds in messages", Permissions::EMBED_LINKS),
        ("MANAGE_ROLES", "Create and assign roles to members", Permissions::MANAGE_ROLES),
        ("MANAGE_CHANNELS", "Create and modify channels/categories", Permissions::MANAGE_CHANNELS),
        ("MANAGE_NICKNAMES", "Set member nicknames after verification", Permissions::MANAGE_NICKNAMES),
    ]
}

/// Result of a permission check for a single guild
#[derive(Debug)]
pub struct GuildPermissionCheck {
    pub guild_id: GuildId,
    pub guild_name: String,
    pub permission_statuses: Vec<PermissionStatus>,
    pub has_all_permissions: bool,
    pub bot_role_position: Option<u16>,
    pub bot_role_name: Option<String>,
    pub highest_managed_role_position: Option<u16>,
    pub role_hierarchy_ok: bool,
    /// Roles that are above the bot and cannot be managed
    pub roles_above_bot: Vec<(String, u16)>,
}

/// Check bot permissions for a specific guild
pub async fn check_guild_permissions(
    http: &Http,
    guild_id: GuildId,
) -> Result<GuildPermissionCheck, String> {
    // Fetch the guild
    let guild = guild_id
        .to_partial_guild(http)
        .await
        .map_err(|e| format!("Failed to fetch guild {}: {}", guild_id, e))?;

    let guild_name = guild.name.clone();

    // Get the bot's member info
    let bot_user = http
        .get_current_user()
        .await
        .map_err(|e| format!("Failed to get bot user: {}", e))?;

    let bot_member = guild
        .member(http, bot_user.id)
        .await
        .map_err(|e| format!("Failed to get bot member in guild {}: {}", guild_id, e))?;

    // Calculate bot's permissions (use base permissions for server-wide permission check)
    #[allow(deprecated)]
    let bot_permissions = guild.member_permissions(&bot_member);

    // Check each required permission
    let required = get_required_permissions();
    let mut permission_statuses = Vec::new();
    let mut has_all_permissions = true;

    for (name, description, permission) in &required {
        let has_perm = bot_permissions.contains(*permission);
        if !has_perm {
            has_all_permissions = false;
        }
        permission_statuses.push(PermissionStatus {
            name,
            description,
            has_permission: has_perm,
        });
    }

    // Check role hierarchy
    let mut bot_role_position: Option<u16> = None;
    let mut bot_role_name: Option<String> = None;
    let mut highest_managed_role_position: Option<u16> = None;
    let mut roles_above_bot: Vec<(String, u16)> = Vec::new();

    // Find bot's highest role position and name
    for role_id in &bot_member.roles {
        if let Some(role) = guild.roles.get(role_id) {
            let pos = role.position;
            if bot_role_position.is_none() || pos > bot_role_position.unwrap() {
                bot_role_position = Some(pos);
                bot_role_name = Some(role.name.clone());
            }
        }
    }

    // Find highest position of roles we might need to manage (excluding @everyone and bot's own roles)
    let everyone_role_id = guild_id.everyone_role();
    for (role_id, role) in &guild.roles {
        // Skip @everyone
        if *role_id == everyone_role_id {
            continue;
        }
        // Skip bot's own roles
        if bot_member.roles.contains(role_id) {
            continue;
        }
        // Skip managed/integration roles (these are typically bot roles that can't be assigned)
        if role.managed {
            continue;
        }

        let pos = role.position;
        if highest_managed_role_position.is_none() || pos > highest_managed_role_position.unwrap() {
            highest_managed_role_position = Some(pos);
        }

        // Track roles that are above the bot's highest role
        if let Some(bot_pos) = bot_role_position {
            if pos >= bot_pos {
                roles_above_bot.push((role.name.clone(), pos));
            }
        }
    }

    // Sort roles_above_bot by position (descending)
    roles_above_bot.sort_by(|a, b| b.1.cmp(&a.1));

    // Role hierarchy is OK if bot's position is higher than the highest role it needs to manage
    let role_hierarchy_ok = match (bot_role_position, highest_managed_role_position) {
        (Some(bot_pos), Some(managed_pos)) => bot_pos > managed_pos,
        (Some(_), None) => true, // No roles to manage
        _ => false,              // Can't determine
    };

    Ok(GuildPermissionCheck {
        guild_id,
        guild_name,
        permission_statuses,
        has_all_permissions,
        bot_role_position,
        bot_role_name,
        highest_managed_role_position,
        role_hierarchy_ok,
        roles_above_bot,
    })
}

/// Check permissions for all guilds the bot is in
pub async fn check_all_guild_permissions(
    http: &Http,
    guild_ids: &[GuildId],
) -> Vec<GuildPermissionCheck> {
    let mut results = Vec::new();

    for guild_id in guild_ids {
        match check_guild_permissions(http, *guild_id).await {
            Ok(check) => results.push(check),
            Err(e) => {
                error!("Failed to check permissions for guild {}: {}", guild_id, e);
            }
        }
    }

    results
}

/// Log permission check results with appropriate log levels
pub fn log_permission_check_results(results: &[GuildPermissionCheck]) {
    info!("========================================");
    info!("       BOT PERMISSION CHECK");
    info!("========================================");
    info!("");

    for check in results {
        info!("Guild: '{}' (ID: {})", check.guild_name, check.guild_id);
        info!("----------------------------------------");

        // Show bot role info
        if let Some(ref role_name) = check.bot_role_name {
            info!("Bot's highest role: '{}' (position {})", role_name, check.bot_role_position.unwrap_or(0));
        } else {
            warn!("Bot has no roles assigned!");
        }
        info!("");

        // Show each permission with yes/no
        info!("Server Permissions:");
        for status in &check.permission_statuses {
            let symbol = if status.has_permission { "[YES]" } else { "[NO] " };

            if status.has_permission {
                info!("  {} {:<18} - {}", symbol, status.name, status.description);
            } else {
                error!("  {} {:<18} - {}", symbol, status.name, status.description);
            }
        }
        info!("");

        // Show role hierarchy status
        info!("Role Hierarchy:");
        if check.role_hierarchy_ok {
            if check.roles_above_bot.is_empty() {
                info!("  [OK] Bot role is above all other roles");
            } else {
                info!("  [OK] Bot role is above all manageable roles");
            }
        } else {
            error!("  [FAIL] Bot role is NOT above all roles it needs to manage!");
            if let Some(managed_pos) = check.highest_managed_role_position {
                error!("         Bot position: {}, Highest managed role position: {}",
                    check.bot_role_position.unwrap_or(0), managed_pos);
            }
        }

        if !check.roles_above_bot.is_empty() {
            warn!("");
            warn!("  Roles the bot CANNOT manage (at or above bot's position):");
            for (role_name, pos) in &check.roles_above_bot {
                warn!("    - '{}' (position {})", role_name, pos);
            }
        }

        info!("");

        // Summary for this guild
        if check.has_all_permissions && check.role_hierarchy_ok {
            info!("Status: ALL CHECKS PASSED");
        } else {
            error!("Status: ISSUES DETECTED - Some operations may fail!");
            if !check.has_all_permissions {
                let missing: Vec<_> = check.permission_statuses
                    .iter()
                    .filter(|s| !s.has_permission)
                    .map(|s| s.name)
                    .collect();
                error!("  Missing permissions: {}", missing.join(", "));
                error!("  Fix: Go to Discord Server Settings > Roles > Bot's role > enable missing permissions");
            }
            if !check.role_hierarchy_ok {
                error!("  Role hierarchy issue: Bot's role must be above all roles it manages");
                error!("  Fix: Go to Discord Server Settings > Roles > drag bot's role higher");
            }
        }
        info!("========================================");
        info!("");
    }

    // Overall summary
    let all_ok = results.iter().all(|r| r.has_all_permissions && r.role_hierarchy_ok);
    if all_ok {
        info!("OVERALL: All permission checks passed for all guilds");
    } else {
        warn!("");
        warn!("OVERALL: Permission issues detected in one or more guilds!");
        warn!("Please fix the issues above to ensure the bot functions correctly.");
    }
}

/// Run a full permission check and log results
/// Returns true if all permissions are OK, false otherwise
pub async fn run_startup_permission_check(http: &Http, guild_ids: &[GuildId]) -> bool {
    let results = check_all_guild_permissions(http, guild_ids).await;
    log_permission_check_results(&results);

    results
        .iter()
        .all(|r| r.has_all_permissions && r.role_hierarchy_ok)
}

/// Check if the bot can modify permissions on other roles
/// This checks if the bot has all permissions that exist on other roles
pub async fn check_role_permission_management(
    http: &Http,
    guild_id: GuildId,
    role_names_to_manage: &[String],
) -> Result<RolePermissionManagementCheck, String> {
    let guild = guild_id
        .to_partial_guild(http)
        .await
        .map_err(|e| format!("Failed to fetch guild: {}", e))?;

    let bot_user = http
        .get_current_user()
        .await
        .map_err(|e| format!("Failed to get bot user: {}", e))?;

    let bot_member = guild
        .member(http, bot_user.id)
        .await
        .map_err(|e| format!("Failed to get bot member: {}", e))?;

    #[allow(deprecated)]
    let bot_permissions = guild.member_permissions(&bot_member);

    // If bot has administrator, it can do everything
    if bot_permissions.contains(Permissions::ADMINISTRATOR) {
        return Ok(RolePermissionManagementCheck {
            can_manage_all: true,
            bot_permissions,
            roles_with_issues: vec![],
            missing_permissions: Permissions::empty(),
        });
    }

    let mut roles_with_issues = Vec::new();
    let mut all_missing = Permissions::empty();

    for (_, role) in &guild.roles {
        // Skip if not in our list to manage
        if !role_names_to_manage.contains(&role.name) {
            continue;
        }

        // Find permissions on this role that the bot doesn't have
        let role_perms = role.permissions;
        let missing = role_perms - bot_permissions;

        if !missing.is_empty() {
            roles_with_issues.push(RolePermissionIssue {
                role_name: role.name.clone(),
                role_permissions: role_perms,
                missing_from_bot: missing,
            });
            all_missing |= missing;
        }
    }

    Ok(RolePermissionManagementCheck {
        can_manage_all: roles_with_issues.is_empty(),
        bot_permissions,
        roles_with_issues,
        missing_permissions: all_missing,
    })
}

/// Result of checking if bot can manage role permissions
#[derive(Debug)]
pub struct RolePermissionManagementCheck {
    pub can_manage_all: bool,
    pub bot_permissions: Permissions,
    pub roles_with_issues: Vec<RolePermissionIssue>,
    pub missing_permissions: Permissions,
}

/// A role that has permissions the bot can't manage
#[derive(Debug)]
pub struct RolePermissionIssue {
    pub role_name: String,
    pub role_permissions: Permissions,
    pub missing_from_bot: Permissions,
}

/// Log role permission management check results
pub fn log_role_permission_management_check(check: &RolePermissionManagementCheck) {
    info!("");
    info!("========================================");
    info!("   ROLE PERMISSION MANAGEMENT CHECK");
    info!("========================================");

    if check.can_manage_all {
        info!("[OK] Bot can manage permissions on all configured roles");
    } else {
        warn!("[WARNING] Bot CANNOT fully manage permissions on some roles!");
        warn!("");
        warn!("The bot can only REMOVE permissions that IT HAS.");
        warn!("To fix this, either:");
        warn!("  1. Give the Bot role 'Administrator' permission, OR");
        warn!("  2. Give the Bot role all the permissions listed below, OR");
        warn!("  3. Set 'skip_permission_sync: true' for these roles in roles.json");
        warn!("");

        for issue in &check.roles_with_issues {
            warn!("Role '{}' has permissions the bot lacks:", issue.role_name);
            log_permission_flags("    ", issue.missing_from_bot);
        }

        warn!("");
        warn!("All missing permissions (union):");
        log_permission_flags("  ", check.missing_permissions);
    }

    info!("========================================");
    info!("");
}

/// Helper to log individual permission flags
/// Maps Discord permission bits to human-readable names shown in Discord's role settings
fn log_permission_flags(prefix: &str, perms: Permissions) {
    // Common permissions visible in Discord's server role settings
    let flag_names = [
        // General Server Permissions
        (Permissions::VIEW_CHANNEL, "View Channels"),
        (Permissions::MANAGE_CHANNELS, "Manage Channels"),
        (Permissions::MANAGE_ROLES, "Manage Roles"),
        (Permissions::MANAGE_GUILD_EXPRESSIONS, "Manage Expressions (Emoji/Stickers)"),
        (Permissions::VIEW_AUDIT_LOG, "View Audit Log"),
        (Permissions::VIEW_GUILD_INSIGHTS, "View Server Insights"),
        (Permissions::MANAGE_WEBHOOKS, "Manage Webhooks"),
        (Permissions::MANAGE_GUILD, "Manage Server"),
        // Membership Permissions
        (Permissions::CREATE_INSTANT_INVITE, "Create Invite"),
        (Permissions::CHANGE_NICKNAME, "Change Nickname"),
        (Permissions::MANAGE_NICKNAMES, "Manage Nicknames"),
        (Permissions::KICK_MEMBERS, "Kick Members"),
        (Permissions::BAN_MEMBERS, "Ban Members"),
        (Permissions::MODERATE_MEMBERS, "Timeout Members"),
        // Text Channel Permissions
        (Permissions::SEND_MESSAGES, "Send Messages"),
        (Permissions::SEND_MESSAGES_IN_THREADS, "Send Messages in Threads"),
        (Permissions::CREATE_PUBLIC_THREADS, "Create Public Threads"),
        (Permissions::CREATE_PRIVATE_THREADS, "Create Private Threads"),
        (Permissions::EMBED_LINKS, "Embed Links"),
        (Permissions::ATTACH_FILES, "Attach Files"),
        (Permissions::ADD_REACTIONS, "Add Reactions"),
        (Permissions::USE_EXTERNAL_EMOJIS, "Use External Emoji"),
        (Permissions::USE_EXTERNAL_STICKERS, "Use External Stickers"),
        (Permissions::MENTION_EVERYONE, "Mention @everyone, @here, and All Roles"),
        (Permissions::MANAGE_MESSAGES, "Manage Messages"),
        (Permissions::MANAGE_THREADS, "Manage Threads"),
        (Permissions::READ_MESSAGE_HISTORY, "Read Message History"),
        (Permissions::SEND_TTS_MESSAGES, "Send Text-to-Speech Messages"),
        (Permissions::USE_APPLICATION_COMMANDS, "Use Application Commands"),
        // Voice Channel Permissions
        (Permissions::CONNECT, "Connect"),
        (Permissions::SPEAK, "Speak"),
        (Permissions::STREAM, "Video"),
        (Permissions::USE_EMBEDDED_ACTIVITIES, "Use Activities"),
        (Permissions::USE_VAD, "Use Voice Activity"),
        (Permissions::PRIORITY_SPEAKER, "Priority Speaker"),
        (Permissions::MUTE_MEMBERS, "Mute Members"),
        (Permissions::DEAFEN_MEMBERS, "Deafen Members"),
        (Permissions::MOVE_MEMBERS, "Move Members"),
        // Events
        (Permissions::MANAGE_EVENTS, "Manage Events"),
        // Special
        (Permissions::ADMINISTRATOR, "Administrator"),
        // Stage Channels (only visible if server has stage channels)
        (Permissions::REQUEST_TO_SPEAK, "Request to Speak (Stage)"),
    ];

    for (flag, name) in flag_names {
        if perms.contains(flag) {
            warn!("{}- {}", prefix, name);
        }
    }
}
