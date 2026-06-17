//! Identity and repository configuration, stored as JSON at `.lore/config`.

use std::collections::BTreeMap;

use serde::{Deserialize, Serialize};

/// Who recorded an entry or commit: a git-style name and email.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(default)]
pub struct Identity {
    pub name: String,
    pub email: String,
}

impl Identity {
    pub fn new(name: impl Into<String>, email: impl Into<String>) -> Identity {
        Identity {
            name: name.into(),
            email: email.into(),
        }
    }

    /// `name <email>`, or just `name` when no email is set.
    pub fn label(&self) -> String {
        if self.email.is_empty() {
            self.name.clone()
        } else {
            format!("{} <{}>", self.name, self.email)
        }
    }
}

/// A named remote and where it lives.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Remote {
    pub url: String,
}

/// Repository configuration: the local identity and named remotes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    #[serde(default)]
    pub user: Identity,
    #[serde(default)]
    pub remotes: BTreeMap<String, Remote>,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn label_with_and_without_email() {
        assert_eq!(Identity::new("Ray", "ray@x.com").label(), "Ray <ray@x.com>");
        assert_eq!(Identity::new("Ray", "").label(), "Ray");
    }

    #[test]
    fn empty_json_is_default_config() {
        let c: Config = serde_json::from_str("{}").unwrap();
        assert_eq!(c, Config::default());
        assert!(c.remotes.is_empty());
    }

    #[test]
    fn partial_config_loads_with_defaults() {
        let c: Config = serde_json::from_str(r#"{"user":{"name":"Ray"}}"#).unwrap();
        assert_eq!(c.user.name, "Ray");
        assert_eq!(c.user.email, "");
        assert!(c.remotes.is_empty());
    }

    #[test]
    fn config_round_trips() {
        let c = Config {
            user: Identity::new("Ray", "ray@x.com"),
            remotes: BTreeMap::from([(
                "origin".to_string(),
                Remote {
                    url: "https://lorehub.com/r".into(),
                },
            )]),
        };
        let back: Config = serde_json::from_slice(&serde_json::to_vec(&c).unwrap()).unwrap();
        assert_eq!(c, back);
    }
}
