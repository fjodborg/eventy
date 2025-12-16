use serde::{Deserialize, Serialize};
use std::collections::HashMap;

// Re-export PermissionSet from global_structure
pub use super::global_structure::PermissionSet;

/// Global permissions configuration - defines permission presets
/// Loaded from data/global/permissions.json
#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalPermissionsConfig {
    /// Permission definitions (e.g., "read", "readwrite", "admin")
    pub definitions: HashMap<String, PermissionSet>,
}

impl GlobalPermissionsConfig {
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

    /// Get a permission definition by name
    pub fn get_definition(&self, name: &str) -> Option<&PermissionSet> {
        self.definitions.get(name)
    }
}

impl Default for GlobalPermissionsConfig {
    fn default() -> Self {
        let mut definitions = HashMap::new();
        definitions.insert(
            "none".to_string(),
            PermissionSet {
                allow: vec![],
                deny: vec!["VIEW_CHANNEL".to_string(), "CONNECT".to_string()],
            },
        );
        definitions.insert(
            "read".to_string(),
            PermissionSet {
                allow: vec![
                    "VIEW_CHANNEL".to_string(),
                    "READ_MESSAGE_HISTORY".to_string(),
                ],
                deny: vec!["SEND_MESSAGES".to_string()],
            },
        );
        definitions.insert(
            "readwrite".to_string(),
            PermissionSet {
                allow: vec![
                    "VIEW_CHANNEL".to_string(),
                    "READ_MESSAGE_HISTORY".to_string(),
                    "SEND_MESSAGES".to_string(),
                    "ATTACH_FILES".to_string(),
                    "ADD_REACTIONS".to_string(),
                ],
                deny: vec![],
            },
        );
        definitions.insert(
            "admin".to_string(),
            PermissionSet {
                allow: vec![
                    "VIEW_CHANNEL".to_string(),
                    "READ_MESSAGE_HISTORY".to_string(),
                    "SEND_MESSAGES".to_string(),
                    "MANAGE_MESSAGES".to_string(),
                    "MANAGE_CHANNELS".to_string(),
                ],
                deny: vec![],
            },
        );
        Self { definitions }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_parse_permissions() {
        let json = r#"{
            "definitions": {
                "read": {
                    "allow": ["VIEW_CHANNEL", "READ_MESSAGE_HISTORY"],
                    "deny": ["SEND_MESSAGES"]
                }
            }
        }"#;

        let config: GlobalPermissionsConfig = serde_json::from_str(json).unwrap();
        assert!(config.get_definition("read").is_some());
    }
}
