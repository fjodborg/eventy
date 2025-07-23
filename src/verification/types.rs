// src/verification/types.rs
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserData {
    #[serde(rename = "Name")]
    pub name: String,
    #[serde(rename = "DiscordId")]
    pub id: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserDatabase {
    #[serde(flatten)]
    pub users: Vec<UserData>,
}

#[derive(Debug)]
pub struct PendingVerification {
    pub user_id: serenity::UserId,
    pub channel_id: serenity::ChannelId,
    pub timestamp: u64,
}

#[derive(Debug, Clone)]
pub struct VerifiedUser {
    pub discord_id: serenity::UserId,
    pub user_data: UserData,
    pub verified_at: u64,
}