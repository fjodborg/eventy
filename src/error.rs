use thiserror::Error;

#[derive(Error, Debug)]
pub enum BotError {
    // Configuration errors
    #[error("Failed to load config file '{path}': {source}")]
    ConfigLoad {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to parse config file '{path}': {source}")]
    ConfigParse {
        path: String,
        #[source]
        source: serde_json::Error,
    },

    #[error("Invalid config: {message}")]
    ConfigValidation { message: String },

    #[error("Config not found: {config_type} '{name}'")]
    ConfigNotFound { config_type: String, name: String },

    // State errors
    #[error("Failed to save state to '{path}': {source}")]
    StateSave {
        path: String,
        #[source]
        source: std::io::Error,
    },

    #[error("Failed to load state from '{path}': {source}")]
    StateLoad {
        path: String,
        #[source]
        source: std::io::Error,
    },

    // Staging errors
    #[error("No staged configuration to commit")]
    NoStagedConfig,

    #[error("Failed to stage config: {message}")]
    StagingFailed { message: String },

    // Verification errors
    #[error("User not found in any season: {user_id}")]
    UserNotFound { user_id: String },

    #[error("User already verified: {discord_id}")]
    AlreadyVerified { discord_id: String },

    #[error("Verification pending for user: {discord_id}")]
    VerificationPending { discord_id: String },

    // Discord errors
    #[error("Discord API error: {message}")]
    Discord { message: String },

    #[error("Channel not found: {name}")]
    ChannelNotFound { name: String },

    #[error("Role not found: {name}")]
    RoleNotFound { name: String },

    #[error("Guild not found: {id}")]
    GuildNotFound { id: String },

    // Permission errors
    #[error("Permission denied: {message}")]
    PermissionDenied { message: String },

    // Generic errors
    #[error("Internal error: {message}")]
    Internal { message: String },
}

impl From<serenity::Error> for BotError {
    fn from(err: serenity::Error) -> Self {
        BotError::Discord {
            message: err.to_string(),
        }
    }
}

impl From<std::io::Error> for BotError {
    fn from(err: std::io::Error) -> Self {
        BotError::Internal {
            message: err.to_string(),
        }
    }
}

impl From<serde_json::Error> for BotError {
    fn from(err: serde_json::Error) -> Self {
        BotError::Internal {
            message: err.to_string(),
        }
    }
}

pub type Result<T> = std::result::Result<T, BotError>;

use poise::serenity_prelude as serenity;
