use anyhow::Result;
use serde::{Deserialize, Serialize};
use tokio::fs;
use tracing::{debug, error, info};

use crate::role_manager::DiscordRoleConfig as RoleConfig;

const ROLES_CONFIG_FILE: &str = "data/roles.json";

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PermissionConfig {
    pub roles: Vec<RoleConfig>,
}

impl PermissionConfig {
    pub async fn load() -> Result<Self> {
        debug!(
            "Loading permission configuration from: {}",
            ROLES_CONFIG_FILE
        );

        let path = std::path::Path::new(ROLES_CONFIG_FILE);
        if !path.exists() {
            error!(
                "Permission configuration file not found at {}",
                ROLES_CONFIG_FILE
            );
            return Err(anyhow::anyhow!("Permission configuration file not found"));
        }

        let content = fs::read_to_string(ROLES_CONFIG_FILE).await?;
        let roles: Vec<RoleConfig> = serde_json::from_str(&content).unwrap();

        info!("Successfully loaded {} role configurations", roles.len());
        Ok(Self { roles })
    }

    pub fn get_role_config(&self, role_name: &str) -> Option<&RoleConfig> {
        self.roles
            .iter()
            .find(|r| r.role.eq_ignore_ascii_case(role_name))
    }

    pub fn get_roles_for_category(&self, category: &str) -> Vec<&RoleConfig> {
        self.roles
            .iter()
            .filter(|r| r.category.eq_ignore_ascii_case(category))
            .collect()
    }

    pub fn get_all_categories(&self) -> Vec<String> {
        let mut categories: Vec<String> = self.roles.iter().map(|r| r.category.clone()).collect();
        categories.sort();
        categories.dedup();
        categories
    }
}
