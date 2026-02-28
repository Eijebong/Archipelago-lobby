use std::fmt;
use std::str::FromStr;

use saphyr::YamlOwned as Value;

pub mod api;
pub mod builtin;
pub mod db;
pub mod page;
pub mod rules;
pub mod triggers;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
pub enum Role {
    Viewer,
    Reviewer,
    RuleEditor,
    Editor,
    Admin,
}

impl Role {
    pub fn as_str(&self) -> &'static str {
        match self {
            Role::Viewer => "viewer",
            Role::Reviewer => "reviewer",
            Role::RuleEditor => "rule_editor",
            Role::Editor => "editor",
            Role::Admin => "admin",
        }
    }
}

impl FromStr for Role {
    type Err = ();

    fn from_str(s: &str) -> Result<Role, ()> {
        match s {
            "viewer" => Ok(Role::Viewer),
            "reviewer" => Ok(Role::Reviewer),
            "rule_editor" => Ok(Role::RuleEditor),
            "editor" => Ok(Role::Editor),
            "admin" => Ok(Role::Admin),
            _ => Err(()),
        }
    }
}

impl fmt::Display for Role {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.as_str())
    }
}

pub fn yaml_value_as_string(val: &Value) -> Option<String> {
    if let Some(s) = val.as_str() {
        return Some(s.to_string());
    }
    if let Some(b) = val.as_bool() {
        return Some(b.to_string());
    }
    if let Some(i) = val.as_integer() {
        return Some(i.to_string());
    }
    None
}
