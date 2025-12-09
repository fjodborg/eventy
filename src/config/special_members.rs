use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for special member roles (Bestyrelse, Korleder, etc.)
/// Maps role names to lists of user verification IDs
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
pub struct SpecialMembersConfig {
    /// Maps role name to list of user IDs (verification UUIDs)
    /// e.g., {"Bestyrelse": ["uuid1", "uuid2"], "Korleder": ["uuid3"]}
    pub roles: HashMap<String, Vec<String>>,
}

impl SpecialMembersConfig {
    /// Create an empty special members config
    pub fn new() -> Self {
        Self {
            roles: HashMap::new(),
        }
    }

    /// Load from a JSON file
    pub fn load_from_file(path: &str) -> crate::error::Result<Self> {
        let content = std::fs::read_to_string(path).map_err(|e| crate::error::BotError::ConfigLoad {
            path: path.to_string(),
            source: e,
        })?;

        serde_json::from_str(&content).map_err(|e| crate::error::BotError::ConfigParse {
            path: path.to_string(),
            source: e,
        })
    }

    /// Get all special roles for a user by their verification ID
    pub fn get_roles_for_user(&self, verification_id: &str) -> Vec<String> {
        self.roles
            .iter()
            .filter_map(|(role_name, user_ids)| {
                if user_ids.contains(&verification_id.to_string()) {
                    Some(role_name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if a user has a specific role
    pub fn has_role(&self, verification_id: &str, role_name: &str) -> bool {
        self.roles
            .get(role_name)
            .map(|ids| ids.contains(&verification_id.to_string()))
            .unwrap_or(false)
    }

    /// Get all role names
    pub fn get_role_names(&self) -> Vec<&String> {
        self.roles.keys().collect()
    }

    /// Add a user to a role
    pub fn add_user_to_role(&mut self, role_name: &str, verification_id: &str) {
        self.roles
            .entry(role_name.to_string())
            .or_insert_with(Vec::new)
            .push(verification_id.to_string());
    }

    /// Remove a user from a role
    pub fn remove_user_from_role(&mut self, role_name: &str, verification_id: &str) {
        if let Some(ids) = self.roles.get_mut(role_name) {
            ids.retain(|id| id != verification_id);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_roles_for_user() {
        let json = r#"{
            "roles": {
                "Bestyrelse": ["user1", "user2"],
                "Korleder": ["user1", "user3"]
            }
        }"#;

        let config: SpecialMembersConfig = serde_json::from_str(json).unwrap();

        let roles = config.get_roles_for_user("user1");
        assert!(roles.contains(&"Bestyrelse".to_string()));
        assert!(roles.contains(&"Korleder".to_string()));
        assert_eq!(roles.len(), 2);

        let roles = config.get_roles_for_user("user2");
        assert_eq!(roles, vec!["Bestyrelse".to_string()]);

        let roles = config.get_roles_for_user("unknown");
        assert!(roles.is_empty());
    }
}
