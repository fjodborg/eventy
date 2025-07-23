// src/permissions/types.rs
use poise::serenity_prelude as serenity;
use serde::{Deserialize, Serialize};

use crate::role_manager::DiscordRoleConfig as RoleConfig;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ChannelPermissionConfig {
    pub name: String,
    pub permission: ChannelPermissionType,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum ChannelPermissionType {
    Read,
    ReadWrite,
    Admin,
    None,
}

#[derive(Debug, Clone)]
pub struct PermissionGrant {
    pub user_id: serenity::UserId,
    pub guild_id: serenity::GuildId,
    pub role_config: RoleConfig,
    pub granted_at: u64,
    pub granted_by: serenity::UserId,
}
