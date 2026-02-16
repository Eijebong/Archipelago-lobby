use saphyr::YamlOwned as Value;

pub mod api;
pub mod builtin;
pub mod db;
pub mod page;
pub mod rules;
pub mod triggers;

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
