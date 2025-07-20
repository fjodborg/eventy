use anyhow::Result;
use dashmap::DashMap;
use poise::serenity_prelude as serenity;
use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::fs;
use tracing::{debug, info, warn};

use super::types::*;

const USER_DATABASE_FILE: &str = "data/users.json";

#[derive(Debug)]
pub struct VerificationManager {
    user_data: Arc<DashMap<String, UserData>>,
    pending_verifications: Arc<DashMap<serenity::UserId, PendingVerification>>,
    verified_users: Arc<DashMap<serenity::UserId, VerifiedUser>>,
}

impl VerificationManager {
    pub fn new() -> Self {
        Self {
            user_data: Arc::new(DashMap::new()),
            pending_verifications: Arc::new(DashMap::new()),
            verified_users: Arc::new(DashMap::new()),
        }
    }

    pub async fn load_database(&self) -> Result<()> {
        debug!("Loading user database from: {}", USER_DATABASE_FILE);

        let path = std::path::Path::new(USER_DATABASE_FILE);
        if !path.exists() {
            warn!("User database file not found at {}, starting with empty database", USER_DATABASE_FILE);
            return Ok(());
        }

        let content = fs::read_to_string(USER_DATABASE_FILE).await?;
        let user_db: UserDatabase = serde_json::from_str(&content).unwrap();

        for user in user_db.users {
            let id = user.id.clone();
            debug!("Loading user: {} ({})", user.name, user.id);
            self.user_data.insert(id, user);
        }

        info!("Successfully loaded {} users from database", self.user_data.len());
        Ok(())
    }

    pub fn find_user_by_id(&self, id: &str) -> Option<UserData> {
        self.user_data.get(id).map(|entry| entry.value().clone())
    }

    pub fn is_user_verified(&self, discord_id: serenity::UserId) -> bool {
        self.verified_users.contains_key(&discord_id)
    }

    pub fn get_verified_user(&self, discord_id: serenity::UserId) -> Option<VerifiedUser> {
        self.verified_users.get(&discord_id).map(|entry| entry.value().clone())
    }

    pub fn add_pending_verification(&self, user_id: serenity::UserId, channel_id: serenity::ChannelId) {
        let verification = PendingVerification {
            user_id,
            channel_id,
            timestamp: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        };
        self.pending_verifications.insert(user_id, verification);
        info!("Added pending verification for user: {}", user_id);
    }

    pub fn complete_verification(&self, discord_id: serenity::UserId, user_data: UserData) {
        let verified_user = VerifiedUser {
            discord_id,
            user_data,
            verified_at: SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs(),
        };

        self.verified_users.insert(discord_id, verified_user);
        self.pending_verifications.remove(&discord_id);
        info!("Completed verification for discord ID: {}", discord_id);
    }

    pub fn is_pending_verification(&self, discord_id: serenity::UserId) -> bool {
        self.pending_verifications.contains_key(&discord_id)
    }

    pub fn remove_pending_verification(&self, discord_id: serenity::UserId) {
        self.pending_verifications.remove(&discord_id);
    }

    pub fn get_all_users(&self) -> Vec<UserData> {
        self.user_data.iter().map(|entry| entry.value().clone()).collect()
    }
}