// src/models.rs
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct UserData {
    #[serde(rename="Name")]
    pub name: String,
    #[serde(rename="DiscordId")]
    pub id: String,
    // pub seasons: Vec<String>,z
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(transparent)]
pub struct UserDatabase {
    #[serde(flatten)]
    pub users: Vec<UserData>
}

#[derive(Debug)]
pub struct PendingVerification {
    pub user_id: serenity::UserId,
    pub channel_id: serenity::ChannelId,
    pub timestamp: u64,
}
