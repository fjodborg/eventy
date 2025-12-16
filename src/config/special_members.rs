use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Configuration for special member roles (Bestyrelse, Korleder, etc.)
/// Maps role names to lists of Discord usernames
#[derive(Debug, Clone, Serialize, Deserialize, Default)]
#[serde(deny_unknown_fields)]
pub struct SpecialMembersConfig {
    /// Maps role name to list of Discord usernames (the actual username, not nickname)
    /// e.g., {"Bestyrelse": ["john_doe", "alice"], "Korleder": ["bob_smith"]}
    pub discord_usernames_by_role: HashMap<String, Vec<String>>,

    /// List of Discord usernames who can access the admin panel
    /// These users get admin panel access without needing ADMINISTRATOR permission
    #[serde(default)]
    pub maintainers: Vec<String>,
}

impl SpecialMembersConfig {
    /// Create an empty special members config
    pub fn new() -> Self {
        Self {
            discord_usernames_by_role: HashMap::new(),
            maintainers: Vec::new(),
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

    /// Get all special roles for a user by their Discord username
    pub fn get_roles_for_user(&self, discord_username: &str) -> Vec<String> {
        // Normalize to lowercase for case-insensitive matching
        let username_lower = discord_username.to_lowercase();
        self.discord_usernames_by_role
            .iter()
            .filter_map(|(role_name, usernames)| {
                if usernames.iter().any(|u| u.to_lowercase() == username_lower) {
                    Some(role_name.clone())
                } else {
                    None
                }
            })
            .collect()
    }

    /// Check if a user has a specific role
    pub fn has_role(&self, discord_username: &str, role_name: &str) -> bool {
        let username_lower = discord_username.to_lowercase();
        self.discord_usernames_by_role
            .get(role_name)
            .map(|usernames| usernames.iter().any(|u| u.to_lowercase() == username_lower))
            .unwrap_or(false)
    }

    /// Get all role names
    pub fn get_role_names(&self) -> Vec<&String> {
        self.discord_usernames_by_role.keys().collect()
    }

    /// Get all role names as owned strings
    pub fn get_all_role_names(&self) -> Vec<String> {
        self.discord_usernames_by_role.keys().cloned().collect()
    }

    /// Add a user to a role by Discord username
    pub fn add_user_to_role(&mut self, role_name: &str, discord_username: &str) {
        self.discord_usernames_by_role
            .entry(role_name.to_string())
            .or_insert_with(Vec::new)
            .push(discord_username.to_string());
    }

    /// Remove a user from a role by Discord username
    pub fn remove_user_from_role(&mut self, role_name: &str, discord_username: &str) {
        let username_lower = discord_username.to_lowercase();
        if let Some(usernames) = self.discord_usernames_by_role.get_mut(role_name) {
            usernames.retain(|u| u.to_lowercase() != username_lower);
        }
    }

    /// Check if a user is a maintainer (can access admin panel)
    pub fn is_maintainer(&self, discord_username: &str) -> bool {
        let username_lower = discord_username.to_lowercase();
        self.maintainers
            .iter()
            .any(|u| u.to_lowercase() == username_lower)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_get_roles_for_user() {
        let json = r#"{
            "discord_usernames_by_role": {
                "Bestyrelse": ["john_doe", "alice"],
                "Korleder": ["john_doe", "bob_smith"]
            },
            "maintainers": []
        }"#;

        let config: SpecialMembersConfig = serde_json::from_str(json).unwrap();

        // john_doe has both roles
        let roles = config.get_roles_for_user("john_doe");
        assert!(roles.contains(&"Bestyrelse".to_string()));
        assert!(roles.contains(&"Korleder".to_string()));
        assert_eq!(roles.len(), 2);

        // alice only has Bestyrelse
        let roles = config.get_roles_for_user("alice");
        assert_eq!(roles, vec!["Bestyrelse".to_string()]);

        // Unknown username has no roles
        let roles = config.get_roles_for_user("unknown_user");
        assert!(roles.is_empty());
    }

    #[test]
    fn test_case_insensitive_matching() {
        let json = r#"{
            "discord_usernames_by_role": {
                "Bestyrelse": ["John_Doe"]
            }
        }"#;

        let config: SpecialMembersConfig = serde_json::from_str(json).unwrap();

        // Should match regardless of case
        assert!(!config.get_roles_for_user("john_doe").is_empty());
        assert!(!config.get_roles_for_user("JOHN_DOE").is_empty());
        assert!(!config.get_roles_for_user("John_Doe").is_empty());
    }

    #[test]
    fn test_is_maintainer() {
        let json = r#"{
            "discord_usernames_by_role": {},
            "maintainers": ["admin_user", "Another_Admin"]
        }"#;

        let config: SpecialMembersConfig = serde_json::from_str(json).unwrap();

        // Should match maintainers (case-insensitive)
        assert!(config.is_maintainer("admin_user"));
        assert!(config.is_maintainer("ADMIN_USER"));
        assert!(config.is_maintainer("another_admin"));

        // Should not match non-maintainers
        assert!(!config.is_maintainer("random_user"));
    }
}
