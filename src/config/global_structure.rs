use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Global structure configuration - defines default channel layout and role permissions
/// This is the base template that categories inherit from
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalStructureConfig {
    /// Default roles to create for each category
    #[serde(default)]
    pub default_roles: Vec<RoleDefinition>,

    /// Default channel structure for categories
    #[serde(default)]
    pub default_channels: Vec<ChannelDefinition>,

    /// Named permission presets that can be referenced
    #[serde(default)]
    pub permission_presets: HashMap<String, PermissionPreset>,

    /// Definitions for permission levels (e.g. "read", "readwrite", "admin")
    #[serde(default)]
    pub permission_definitions: HashMap<String, PermissionSet>,
}

impl Default for GlobalStructureConfig {
    fn default() -> Self {
        Self {
            default_roles: vec![RoleDefinition {
                name: "Medlem".to_string(),
                color: Some("#2ecc71".to_string()),
                hoist: false,
                mentionable: true,
                position: None,
                is_default_member_role: true,
                permissions: vec![],
                skip_permission_sync: false,
            }],
            default_channels: vec![ChannelDefinition {
                name: "general".to_string(),
                channel_type: ChannelType::Text,
                position: Some(0),
                role_permissions: {
                    let mut perms = HashMap::new();
                    perms.insert("Medlem".to_string(), ChannelPermissionLevel::ReadWrite);
                    perms
                },
                children: vec![],
            }],
            permission_presets: HashMap::new(),
            permission_definitions: HashMap::new(),
        }
    }
}

impl GlobalStructureConfig {
    /// Load from a JSON file
    pub fn load_from_file(path: &str) -> crate::error::Result<Self> {
        let content =
            std::fs::read_to_string(path).map_err(|e| crate::error::BotError::ConfigLoad {
                path: path.to_string(),
                source: e,
            })?;

        serde_json::from_str(&content).map_err(|e| crate::error::BotError::ConfigParse {
            path: path.to_string(),
            source: e,
        })
    }

    /// Get the default member role
    pub fn get_default_member_role(&self) -> Option<&RoleDefinition> {
        self.default_roles.iter().find(|r| r.is_default_member_role)
    }
}

/// Definition for a role
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RoleDefinition {
    /// Role name
    pub name: String,

    /// Hex color (e.g., "#ff0000")
    #[serde(skip_serializing_if = "Option::is_none")]
    pub color: Option<String>,

    /// Whether to display role members separately
    #[serde(default)]
    pub hoist: bool,

    /// Whether the role can be mentioned
    #[serde(default)]
    pub mentionable: bool,

    /// Position in the role hierarchy
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u16>,

    /// Whether this is the default role for new members
    #[serde(default)]
    pub is_default_member_role: bool,

    /// Server-level permissions for this role (e.g., ["CHANGE_NICKNAME", "CREATE_INSTANT_INVITE", "ADMINISTRATOR"])
    /// If not specified, defaults to no permissions
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub permissions: Vec<String>,

    /// If true, skip syncing permissions for this role (useful for roles managed manually in Discord)
    #[serde(default)]
    pub skip_permission_sync: bool,
}

impl RoleDefinition {
    /// Check if permissions are explicitly specified in the config
    /// Returns true if permissions should be synced (non-empty list or skip_permission_sync is false)
    pub fn has_explicit_permissions(&self) -> bool {
        !self.permissions.is_empty()
    }

    /// Parse permission strings into Discord Permissions
    pub fn get_permissions(&self) -> poise::serenity_prelude::Permissions {
        use poise::serenity_prelude::Permissions;

        let mut perms = Permissions::empty();
        for name in &self.permissions {
            match name.to_uppercase().as_str() {
                "CREATE_INSTANT_INVITE" => perms |= Permissions::CREATE_INSTANT_INVITE,
                "KICK_MEMBERS" => perms |= Permissions::KICK_MEMBERS,
                "BAN_MEMBERS" => perms |= Permissions::BAN_MEMBERS,
                "ADMINISTRATOR" => perms |= Permissions::ADMINISTRATOR,
                "MANAGE_CHANNELS" => perms |= Permissions::MANAGE_CHANNELS,
                "MANAGE_GUILD" => perms |= Permissions::MANAGE_GUILD,
                "ADD_REACTIONS" => perms |= Permissions::ADD_REACTIONS,
                "VIEW_AUDIT_LOG" => perms |= Permissions::VIEW_AUDIT_LOG,
                "PRIORITY_SPEAKER" => perms |= Permissions::PRIORITY_SPEAKER,
                "STREAM" => perms |= Permissions::STREAM,
                "VIEW_CHANNEL" => perms |= Permissions::VIEW_CHANNEL,
                "SEND_MESSAGES" => perms |= Permissions::SEND_MESSAGES,
                "SEND_TTS_MESSAGES" => perms |= Permissions::SEND_TTS_MESSAGES,
                "MANAGE_MESSAGES" => perms |= Permissions::MANAGE_MESSAGES,
                "EMBED_LINKS" => perms |= Permissions::EMBED_LINKS,
                "ATTACH_FILES" => perms |= Permissions::ATTACH_FILES,
                "READ_MESSAGE_HISTORY" => perms |= Permissions::READ_MESSAGE_HISTORY,
                "MENTION_EVERYONE" => perms |= Permissions::MENTION_EVERYONE,
                "USE_EXTERNAL_EMOJIS" => perms |= Permissions::USE_EXTERNAL_EMOJIS,
                "VIEW_GUILD_INSIGHTS" => perms |= Permissions::VIEW_GUILD_INSIGHTS,
                "CONNECT" => perms |= Permissions::CONNECT,
                "SPEAK" => perms |= Permissions::SPEAK,
                "MUTE_MEMBERS" => perms |= Permissions::MUTE_MEMBERS,
                "DEAFEN_MEMBERS" => perms |= Permissions::DEAFEN_MEMBERS,
                "MOVE_MEMBERS" => perms |= Permissions::MOVE_MEMBERS,
                "USE_VAD" => perms |= Permissions::USE_VAD,
                "CHANGE_NICKNAME" => perms |= Permissions::CHANGE_NICKNAME,
                "MANAGE_NICKNAMES" => perms |= Permissions::MANAGE_NICKNAMES,
                "MANAGE_ROLES" => perms |= Permissions::MANAGE_ROLES,
                "MANAGE_WEBHOOKS" => perms |= Permissions::MANAGE_WEBHOOKS,
                "MANAGE_EMOJIS" => perms |= Permissions::MANAGE_GUILD_EXPRESSIONS,
                "USE_APPLICATION_COMMANDS" => perms |= Permissions::USE_APPLICATION_COMMANDS,
                "REQUEST_TO_SPEAK" => perms |= Permissions::REQUEST_TO_SPEAK,
                "MANAGE_EVENTS" => perms |= Permissions::MANAGE_EVENTS,
                "MANAGE_THREADS" => perms |= Permissions::MANAGE_THREADS,
                "CREATE_PUBLIC_THREADS" => perms |= Permissions::CREATE_PUBLIC_THREADS,
                "CREATE_PRIVATE_THREADS" => perms |= Permissions::CREATE_PRIVATE_THREADS,
                "USE_EXTERNAL_STICKERS" => perms |= Permissions::USE_EXTERNAL_STICKERS,
                "SEND_MESSAGES_IN_THREADS" => perms |= Permissions::SEND_MESSAGES_IN_THREADS,
                "USE_EMBEDDED_ACTIVITIES" => perms |= Permissions::USE_EMBEDDED_ACTIVITIES,
                "MODERATE_MEMBERS" => perms |= Permissions::MODERATE_MEMBERS,
                "MANAGE_GUILD_EXPRESSIONS" => perms |= Permissions::MANAGE_GUILD_EXPRESSIONS,
                unknown => {
                    tracing::warn!(
                        "Unknown permission string '{}' in role '{}', ignoring",
                        unknown,
                        self.name
                    );
                }
            }
        }
        perms
    }
}

/// Definition for a channel (or category)
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ChannelDefinition {
    /// Channel name
    pub name: String,

    /// Channel type
    #[serde(rename = "type")]
    pub channel_type: ChannelType,

    /// Position in the channel list
    #[serde(skip_serializing_if = "Option::is_none")]
    pub position: Option<u16>,

    /// Role-based permissions for this channel
    #[serde(default)]
    pub role_permissions: HashMap<String, ChannelPermissionLevel>,

    /// Child channels (for categories only)
    #[serde(default)]
    pub children: Vec<ChannelDefinition>,
}

/// Channel types
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelType {
    Category,
    Text,
    Voice,
    Forum,
    Stage,
    News,
}

impl ChannelType {
    /// Convert to serenity ChannelType
    pub fn to_serenity(&self) -> poise::serenity_prelude::ChannelType {
        use poise::serenity_prelude::ChannelType as SerenityChannelType;
        match self {
            ChannelType::Category => SerenityChannelType::Category,
            ChannelType::Text => SerenityChannelType::Text,
            ChannelType::Voice => SerenityChannelType::Voice,
            ChannelType::Forum => SerenityChannelType::Forum,
            ChannelType::Stage => SerenityChannelType::Stage,
            ChannelType::News => SerenityChannelType::News,
        }
    }
}

/// Permission levels for channels
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum ChannelPermissionLevel {
    /// Cannot view the channel
    None,
    /// Can view and read, but not send messages
    Read,
    /// Can view, read, and send messages
    ReadWrite,
    /// Full channel management
    Admin,
}

impl ChannelPermissionLevel {
    /// Convert to Discord permission overwrites
    pub fn to_permissions(
        &self,
        channel_type: &ChannelType,
        config: &GlobalStructureConfig,
    ) -> (
        poise::serenity_prelude::Permissions,
        poise::serenity_prelude::Permissions,
    ) {
        use poise::serenity_prelude::Permissions;

        // Helper to parse permission strings
        let parse_perms = |perm_names: &[String]| -> Permissions {
            let mut p = Permissions::empty();
            for name in perm_names {
                match name.to_uppercase().as_str() {
                    "CREATE_INSTANT_INVITE" => p |= Permissions::CREATE_INSTANT_INVITE,
                    "KICK_MEMBERS" => p |= Permissions::KICK_MEMBERS,
                    "BAN_MEMBERS" => p |= Permissions::BAN_MEMBERS,
                    "ADMINISTRATOR" => p |= Permissions::ADMINISTRATOR,
                    "MANAGE_CHANNELS" => p |= Permissions::MANAGE_CHANNELS,
                    "MANAGE_GUILD" => p |= Permissions::MANAGE_GUILD,
                    "ADD_REACTIONS" => p |= Permissions::ADD_REACTIONS,
                    "VIEW_AUDIT_LOG" => p |= Permissions::VIEW_AUDIT_LOG,
                    "PRIORITY_SPEAKER" => p |= Permissions::PRIORITY_SPEAKER,
                    "STREAM" => p |= Permissions::STREAM,
                    "VIEW_CHANNEL" => p |= Permissions::VIEW_CHANNEL,
                    "SEND_MESSAGES" => p |= Permissions::SEND_MESSAGES,
                    "SEND_TTS_MESSAGES" => p |= Permissions::SEND_TTS_MESSAGES,
                    "MANAGE_MESSAGES" => p |= Permissions::MANAGE_MESSAGES,
                    "EMBED_LINKS" => p |= Permissions::EMBED_LINKS,
                    "ATTACH_FILES" => p |= Permissions::ATTACH_FILES,
                    "READ_MESSAGE_HISTORY" => p |= Permissions::READ_MESSAGE_HISTORY,
                    "MENTION_EVERYONE" => p |= Permissions::MENTION_EVERYONE,
                    "USE_EXTERNAL_EMOJIS" => p |= Permissions::USE_EXTERNAL_EMOJIS,
                    "VIEW_GUILD_INSIGHTS" => p |= Permissions::VIEW_GUILD_INSIGHTS,
                    "CONNECT" => p |= Permissions::CONNECT,
                    "SPEAK" => p |= Permissions::SPEAK,
                    "MUTE_MEMBERS" => p |= Permissions::MUTE_MEMBERS,
                    "DEAFEN_MEMBERS" => p |= Permissions::DEAFEN_MEMBERS,
                    "MOVE_MEMBERS" => p |= Permissions::MOVE_MEMBERS,
                    "USE_VAD" => p |= Permissions::USE_VAD,
                    "CHANGE_NICKNAME" => p |= Permissions::CHANGE_NICKNAME,
                    "MANAGE_NICKNAMES" => p |= Permissions::MANAGE_NICKNAMES,
                    "MANAGE_ROLES" => p |= Permissions::MANAGE_ROLES,
                    "MANAGE_WEBHOOKS" => p |= Permissions::MANAGE_WEBHOOKS,
                    "MANAGE_EMOJIS" => p |= Permissions::MANAGE_GUILD_EXPRESSIONS,
                    "USE_APPLICATION_COMMANDS" => p |= Permissions::USE_APPLICATION_COMMANDS,
                    "REQUEST_TO_SPEAK" => p |= Permissions::REQUEST_TO_SPEAK,
                    "MANAGE_EVENTS" => p |= Permissions::MANAGE_EVENTS,
                    "MANAGE_THREADS" => p |= Permissions::MANAGE_THREADS,
                    "CREATE_PUBLIC_THREADS" => p |= Permissions::CREATE_PUBLIC_THREADS,
                    "CREATE_PRIVATE_THREADS" => p |= Permissions::CREATE_PRIVATE_THREADS,
                    "USE_EXTERNAL_STICKERS" => p |= Permissions::USE_EXTERNAL_STICKERS,
                    "SEND_MESSAGES_IN_THREADS" => p |= Permissions::SEND_MESSAGES_IN_THREADS,
                    "USE_EMBEDDED_ACTIVITIES" => p |= Permissions::USE_EMBEDDED_ACTIVITIES,
                    "MODERATE_MEMBERS" => p |= Permissions::MODERATE_MEMBERS,
                    "MANAGE_GUILD_EXPRESSIONS" => p |= Permissions::MANAGE_GUILD_EXPRESSIONS,
                    unknown => {
                        tracing::warn!(
                            "Unknown permission string '{}' in permission_definitions, ignoring",
                            unknown
                        );
                    }
                }
            }
            p
        };

        // First check if we have a custom definition for this level
        // For voice channels, try voice-specific keys first (e.g., "read_voice")
        let is_voice = matches!(channel_type, ChannelType::Voice | ChannelType::Stage);

        let level_name = match self {
            ChannelPermissionLevel::None => "none",
            ChannelPermissionLevel::Read => "read",
            ChannelPermissionLevel::ReadWrite => "readwrite",
            ChannelPermissionLevel::Admin => "admin",
        };

        // Try voice-specific definition first for voice channels
        if is_voice {
            let voice_key = format!("{}_voice", level_name);
            if let Some(def) = config.permission_definitions.get(&voice_key) {
                return (parse_perms(&def.allow), parse_perms(&def.deny));
            }
        }

        // Then try the standard definition
        if let Some(def) = config.permission_definitions.get(level_name) {
            return (parse_perms(&def.allow), parse_perms(&def.deny));
        }

        // Fallback to hardcoded defaults if not defined
        match self {
            ChannelPermissionLevel::None => (
                Permissions::empty(),
                Permissions::VIEW_CHANNEL | Permissions::CONNECT,
            ),
            ChannelPermissionLevel::Read => match channel_type {
                ChannelType::Voice | ChannelType::Stage => (
                    Permissions::VIEW_CHANNEL | Permissions::CONNECT | Permissions::SPEAK,
                    Permissions::empty(),
                ),
                _ => (
                    Permissions::VIEW_CHANNEL | Permissions::READ_MESSAGE_HISTORY,
                    Permissions::SEND_MESSAGES,
                ),
            },
            ChannelPermissionLevel::ReadWrite => match channel_type {
                ChannelType::Voice | ChannelType::Stage => (
                    Permissions::VIEW_CHANNEL
                        | Permissions::CONNECT
                        | Permissions::SPEAK
                        | Permissions::STREAM
                        | Permissions::USE_VAD,
                    Permissions::empty(),
                ),
                _ => (
                    Permissions::VIEW_CHANNEL
                        | Permissions::READ_MESSAGE_HISTORY
                        | Permissions::SEND_MESSAGES
                        | Permissions::ATTACH_FILES
                        | Permissions::ADD_REACTIONS,
                    Permissions::empty(),
                ),
            },
            ChannelPermissionLevel::Admin => (
                Permissions::VIEW_CHANNEL
                    | Permissions::READ_MESSAGE_HISTORY
                    | Permissions::SEND_MESSAGES
                    | Permissions::ATTACH_FILES
                    | Permissions::ADD_REACTIONS
                    | Permissions::MANAGE_MESSAGES
                    | Permissions::MANAGE_CHANNELS
                    | Permissions::MANAGE_WEBHOOKS
                    | Permissions::CONNECT
                    | Permissions::SPEAK
                    | Permissions::MUTE_MEMBERS
                    | Permissions::DEAFEN_MEMBERS
                    | Permissions::MOVE_MEMBERS,
                Permissions::empty(),
            ),
        }
    }
}

/// Named permission preset
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionPreset {
    pub name: String,
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

/// Set of allowed and denied permissions
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PermissionSet {
    #[serde(default)]
    pub allow: Vec<String>,
    #[serde(default)]
    pub deny: Vec<String>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_global_structure() {
        let json = r##"{
            "default_roles": [
                {
                    "name": "Medlem",
                    "color": "#00ff00",
                    "is_default_member_role": true
                }
            ],
            "default_channels": [
                {
                    "name": "general",
                    "type": "text",
                    "role_permissions": {
                        "Medlem": "readwrite"
                    },
                    "children": []
                }
            ]
        }"##;

        let config: GlobalStructureConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.default_roles.len(), 1);
        assert_eq!(config.default_channels.len(), 1);
        assert!(config.default_roles[0].is_default_member_role);
    }
}
