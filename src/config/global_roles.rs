use serde::{Deserialize, Serialize};

// Re-export RoleDefinition from global_structure
pub use super::global_structure::RoleDefinition;

/// Global roles configuration - defines Discord roles
/// Loaded from data/global/roles.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalRolesConfig {
    /// Role definitions
    pub roles: Vec<RoleDefinition>,
}

impl GlobalRolesConfig {
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
        self.roles.iter().find(|r| r.is_default_member_role)
    }

    /// Get a role by name
    pub fn get_role(&self, name: &str) -> Option<&RoleDefinition> {
        self.roles.iter().find(|r| r.name == name)
    }
}

impl Default for GlobalRolesConfig {
    fn default() -> Self {
        Self {
            roles: vec![RoleDefinition {
                name: "Medlem".to_string(),
                color: Some("#2ecc71".to_string()),
                hoist: false,
                mentionable: true,
                position: None,
                is_default_member_role: true,
                permissions: vec![],
                skip_permission_sync: false,
            }],
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_roles() {
        let json = r##"{
            "roles": [
                {
                    "name": "Medlem",
                    "color": "#2ecc71",
                    "is_default_member_role": true
                },
                {
                    "name": "Admin",
                    "color": "#e74c3c",
                    "hoist": true
                }
            ]
        }"##;

        let config: GlobalRolesConfig = serde_json::from_str(json).unwrap();
        assert_eq!(config.roles.len(), 2);
        assert!(config.get_default_member_role().is_some());
    }
}
