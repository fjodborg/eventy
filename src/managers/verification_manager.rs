use poise::serenity_prelude::UserId;
use std::sync::Arc;
use tracing::info;

use crate::error::Result;
use crate::managers::{ConfigManager, SharedConfigManager};
use crate::state::{SharedUserDatabase, TrackedUser, UserDatabase};

/// Result of a verification attempt
#[derive(Debug, Clone)]
pub struct VerificationResult {
    pub success: bool,
    pub display_name: String,
    pub seasons: Vec<String>,
    pub roles_to_assign: Vec<String>,
    pub error: Option<String>,
}

/// Manages user verification flow
pub struct VerificationManager {
    /// User database
    user_db: SharedUserDatabase,

    /// Config manager for season lookups
    config_manager: SharedConfigManager,
}

impl VerificationManager {
    pub fn new(user_db: SharedUserDatabase, config_manager: SharedConfigManager) -> Self {
        Self {
            user_db,
            config_manager,
        }
    }

    /// Get access to the user database (for web server integration)
    pub fn user_db(&self) -> &SharedUserDatabase {
        &self.user_db
    }

    /// Check if a user is already verified (by Discord ID)
    pub async fn is_verified(&self, user_id: UserId) -> bool {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.is_verified(&user_id.to_string())
    }

    /// Get a verified user by Discord ID
    pub async fn get_verified_user(&self, user_id: UserId) -> Option<TrackedUser> {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.find_by_discord_id(&user_id.to_string()).cloned()
    }

    /// Attempt verification with a provided ID
    pub async fn attempt_verification(
        &self,
        user_id: UserId,
        provided_id: &str,
    ) -> VerificationResult {
        let provided_id = provided_id.trim();

        // Look up the user in seasons first to know which season this ID belongs to
        let config: tokio::sync::RwLockReadGuard<'_, ConfigManager> =
            self.config_manager.read().await;
        let (season_id, season_user) = match config.find_user_by_verification_id(provided_id) {
            Some((season, user)) => (season.season_id.clone(), user.clone()),
            None => {
                return VerificationResult {
                    success: false,
                    display_name: String::new(),
                    seasons: vec![],
                    roles_to_assign: vec![],
                    error: Some(format!(
                        "Could not find ID '{}' in our records. Please check your ID and try again.",
                        provided_id
                    )),
                };
            }
        };

        // Check if this verification ID was already used by someone else
        {
            let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
            if let Some(existing) = db.find_by_verification_id(provided_id) {
                if existing.discord_id != user_id.to_string() {
                    return VerificationResult {
                        success: false,
                        display_name: String::new(),
                        seasons: vec![],
                        roles_to_assign: vec![],
                        error: Some(
                            "This ID has already been used to verify another account.".to_string(),
                        ),
                    };
                }
            }

            // Check if this Discord user is already verified for THIS season
            if let Some(existing) = db.find_by_discord_id(&user_id.to_string()) {
                if existing.verification_ids.contains_key(&season_id) {
                    return VerificationResult {
                        success: false,
                        display_name: existing.display_name.clone(),
                        seasons: existing.verification_ids.keys().cloned().collect(),
                        roles_to_assign: vec![],
                        error: Some(format!(
                            "You are already verified for season {}!",
                            season_id
                        )),
                    };
                }
            }
        }

        // Found the user! Get their special roles too
        let special_roles = config.get_special_roles_for_user(provided_id);
        let default_role = config.get_default_member_role_name().to_string();

        let mut roles_to_assign = vec![default_role];
        roles_to_assign.extend(special_roles.clone());

        let display_name = season_user.name.clone();

        // Update or create tracked user and save to database
        {
            let mut db: tokio::sync::RwLockWriteGuard<'_, UserDatabase> =
                self.user_db.write().await;

            if let Some(existing) = db.find_by_discord_id(&user_id.to_string()) {
                let mut updated_user = existing.clone();
                updated_user.add_verification_id(&season_id, provided_id);
                // Merge special roles
                for role in &special_roles {
                    if !updated_user.special_roles.contains(role) {
                        updated_user.special_roles.push(role.clone());
                    }
                }
                // Update display name if it changed (optional, but good practice)
                updated_user.display_name = display_name.clone();
                updated_user.update_last_seen();

                db.upsert_user(updated_user);
            } else {
                let tracked_user = TrackedUser::new(
                    user_id.to_string(),
                    provided_id.to_string(),
                    season_id.clone(),
                    display_name.clone(),
                    special_roles,
                );
                db.upsert_user(tracked_user);
            }
        }

        info!(
            "User {} verified as '{}' for season {}",
            user_id, display_name, season_id
        );

        VerificationResult {
            success: true,
            display_name,
            seasons: vec![season_id],
            roles_to_assign,
            error: None,
        }
    }

    /// Save the user database to disk
    pub async fn save_database(&self, path: &str) -> Result<()> {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.save(path).await
    }

    /// Get all users from the database
    pub async fn get_all_users(&self) -> Vec<TrackedUser> {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.get_all_users().into_iter().cloned().collect()
    }

    /// Get user count
    pub async fn get_user_count(&self) -> usize {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.user_count()
    }

    /// Export database as JSON bytes
    pub async fn export_database(&self) -> Result<Vec<u8>> {
        let db: tokio::sync::RwLockReadGuard<'_, UserDatabase> = self.user_db.read().await;
        db.export()
    }
}

/// Shared verification manager type
pub type SharedVerificationManager = Arc<VerificationManager>;

pub fn create_shared_verification_manager(
    user_db: SharedUserDatabase,
    config_manager: SharedConfigManager,
) -> SharedVerificationManager {
    Arc::new(VerificationManager::new(user_db, config_manager))
}
