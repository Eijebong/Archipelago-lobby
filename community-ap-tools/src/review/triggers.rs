use std::collections::HashMap;

use saphyr::{LoadableYamlNode, YamlOwned as Value};

use super::yaml_value_as_string;

fn is_trueish_match(option_val: &Value, trigger_result: &str) -> bool {
    yaml_value_as_string(option_val).is_some_and(|s| s == trigger_result)
}

fn value_can_match(val: &Value, trigger_result: &str) -> bool {
    if is_trueish_match(val, trigger_result) {
        return true;
    }

    if let Some(map) = val.as_mapping() {
        for (key, weight) in map.iter() {
            let w = weight.as_integer().unwrap_or(0);
            if w != 0 && is_trueish_match(key, trigger_result) {
                return true;
            }
        }
    }

    false
}

struct Trigger {
    option_category: String,
    option_name: String,
    option_result: String,
    options: Vec<(String, String, Value, bool)>, // (game, key, value, is_additive)
}

fn parse_trigger(trigger_yaml: &Value) -> Option<Trigger> {
    let category = trigger_yaml.as_mapping_get("option_category")?.as_str()?;
    let name = trigger_yaml.as_mapping_get("option_name")?.as_str()?;
    let result_str = yaml_value_as_string(trigger_yaml.as_mapping_get("option_result")?)?;

    let options_map = trigger_yaml.as_mapping_get("options")?;
    let mut options = Vec::new();

    if let Some(outer) = options_map.as_mapping() {
        for (game_key, game_opts) in outer.iter() {
            let game_name = game_key.as_str()?;
            if let Some(inner) = game_opts.as_mapping() {
                for (opt_key, opt_val) in inner.iter() {
                    let raw_key = opt_key.as_str()?;
                    let is_additive = raw_key.starts_with('+');
                    let key = if is_additive { &raw_key[1..] } else { raw_key };
                    options.push((
                        game_name.to_string(),
                        key.to_string(),
                        opt_val.clone(),
                        is_additive,
                    ));
                }
            }
        }
    }

    Some(Trigger {
        option_category: category.to_string(),
        option_name: name.to_string(),
        option_result: result_str,
        options,
    })
}

fn parse_triggers_from(val: &Value) -> impl Iterator<Item = Trigger> + '_ {
    val.as_sequence()
        .into_iter()
        .flatten()
        .filter_map(parse_trigger)
}

fn collect_triggers(yaml: &Value) -> Vec<Trigger> {
    let mut triggers = Vec::new();

    if let Some(top_triggers) = yaml.as_mapping_get("triggers") {
        triggers.extend(parse_triggers_from(top_triggers));
    }

    if let Some(map) = yaml.as_mapping() {
        for (key, game_section) in map.iter() {
            let Some(key_str) = key.as_str() else {
                continue;
            };
            if matches!(key_str, "triggers" | "game" | "name" | "description") {
                continue;
            }
            if let Some(game_triggers) = game_section.as_mapping_get("triggers") {
                triggers.extend(parse_triggers_from(game_triggers));
            }
        }
    }

    triggers
}

pub fn resolve_triggers(yaml: &Value) -> Value {
    let triggers = collect_triggers(yaml);
    if triggers.is_empty() {
        return yaml.clone();
    }

    let mut current = yaml.clone();
    let mut iterations = 0;
    const MAX_ITERATIONS: usize = 50;

    loop {
        iterations += 1;
        if iterations > MAX_ITERATIONS {
            break;
        }

        let triggers = collect_triggers(&current);
        // game -> key -> value
        let mut changes: HashMap<(&str, &str), Value> = HashMap::new();

        for trigger in &triggers {
            let option_val = current
                .as_mapping_get(&trigger.option_category)
                .and_then(|game| game.as_mapping_get(&trigger.option_name));

            let Some(option_val) = option_val else {
                continue;
            };

            if !value_can_match(option_val, &trigger.option_result) {
                continue;
            }

            for (game, key, value, is_additive) in &trigger.options {
                let Some(game_section) = current.as_mapping_get(game) else {
                    continue;
                };

                if *is_additive {
                    let existing = game_section.as_mapping_get(key);
                    let merged = merge_additive(existing, value);
                    if existing.is_some_and(|e| e == &merged) {
                        continue;
                    }
                    changes.insert((game, key), merged);
                } else {
                    let existing = game_section.as_mapping_get(key);
                    if existing.is_some_and(|e| yaml_values_equal(e, value)) {
                        continue;
                    }
                    changes.insert((game, key), value.clone());
                }
            }
        }

        if changes.is_empty() {
            break;
        }

        current = set_yaml_values(&current, &changes);
    }

    current
}

fn yaml_values_equal(a: &Value, b: &Value) -> bool {
    if let (Some(a_str), Some(b_str)) = (a.as_str(), b.as_str()) {
        return a_str == b_str;
    }
    if let (Some(a_int), Some(b_int)) = (a.as_integer(), b.as_integer()) {
        return a_int == b_int;
    }
    if let (Some(a_bool), Some(b_bool)) = (a.as_bool(), b.as_bool()) {
        return a_bool == b_bool;
    }
    false
}

fn merge_additive(existing: Option<&Value>, addition: &Value) -> Value {
    // If existing is a list and addition is a list, concatenate
    // Otherwise just use the addition
    let mut items = Vec::new();

    if let Some(existing) = existing
        && let Some(seq) = existing.as_sequence()
    {
        for item in seq {
            items.push(format!("- {}", yaml_value_to_inline(item)));
        }
    }

    if let Some(seq) = addition.as_sequence() {
        for item in seq {
            items.push(format!("- {}", yaml_value_to_inline(item)));
        }
    } else {
        items.push(format!("- {}", yaml_value_to_inline(addition)));
    }

    let yaml_str = items.join("\n");
    Value::load_from_str(&yaml_str)
        .ok()
        .and_then(|mut docs| docs.pop())
        .unwrap_or_else(|| addition.clone())
}

fn yaml_value_to_inline(val: &Value) -> String {
    if let Some(s) = val.as_str() {
        let escaped = s.replace('\\', "\\\\").replace('"', "\\\"");
        return format!("\"{}\"", escaped);
    }
    if let Some(b) = val.as_bool() {
        return b.to_string();
    }
    if let Some(i) = val.as_integer() {
        return i.to_string();
    }
    if let Some(f) = val.as_floating_point() {
        return f.to_string();
    }
    "null".to_string()
}

fn set_yaml_values(yaml: &Value, changes: &HashMap<(&str, &str), Value>) -> Value {
    let mut lines = Vec::new();
    let Some(top_map) = yaml.as_mapping() else {
        return yaml.clone();
    };

    // Group changes by game for efficient lookup
    let mut by_game: HashMap<&str, HashMap<&str, &Value>> = HashMap::new();
    for ((game, key), value) in changes {
        by_game.entry(game).or_default().insert(key, value);
    }

    for (top_key, top_val) in top_map.iter() {
        let top_key_str = top_key.as_str().unwrap_or("");
        lines.push(format!("{}:", top_key_str));

        if let Some(game_changes) = by_game.get(top_key_str) {
            if let Some(game_map) = top_val.as_mapping() {
                let mut seen_keys = std::collections::HashSet::new();
                for (opt_key, opt_val) in game_map.iter() {
                    let opt_key_str = opt_key.as_str().unwrap_or("");
                    seen_keys.insert(opt_key_str);
                    if let Some(new_val) = game_changes.get(opt_key_str) {
                        serialize_yaml_entry(&mut lines, opt_key_str, new_val, 2);
                    } else {
                        serialize_yaml_entry(&mut lines, opt_key_str, opt_val, 2);
                    }
                }
                for (key, val) in game_changes {
                    if !seen_keys.contains(key) {
                        serialize_yaml_entry(&mut lines, key, val, 2);
                    }
                }
            } else {
                serialize_yaml_value(&mut lines, top_val, 2);
            }
        } else {
            serialize_yaml_value(&mut lines, top_val, 2);
        }
    }

    let rebuilt = lines.join("\n");
    Value::load_from_str(&rebuilt)
        .ok()
        .and_then(|mut docs| docs.pop())
        .unwrap_or_else(|| yaml.clone())
}

fn serialize_yaml_entry(lines: &mut Vec<String>, key: &str, value: &Value, indent: usize) {
    let prefix = " ".repeat(indent);
    if let Some(map) = value.as_mapping() {
        lines.push(format!("{}{}:", prefix, key));
        for (k, v) in map.iter() {
            let k_str = k.as_str().unwrap_or("");
            serialize_yaml_entry(lines, k_str, v, indent + 2);
        }
    } else if let Some(seq) = value.as_sequence() {
        lines.push(format!("{}{}:", prefix, key));
        for item in seq {
            lines.push(format!("{}  - {}", prefix, yaml_value_to_inline(item)));
        }
    } else {
        lines.push(format!(
            "{}{}: {}",
            prefix,
            key,
            yaml_value_to_inline(value)
        ));
    }
}

fn serialize_yaml_value(lines: &mut Vec<String>, value: &Value, indent: usize) {
    let prefix = " ".repeat(indent);
    if let Some(map) = value.as_mapping() {
        for (k, v) in map.iter() {
            let k_str = k.as_str().unwrap_or("");
            serialize_yaml_entry(lines, k_str, v, indent);
        }
    } else if let Some(seq) = value.as_sequence() {
        for item in seq {
            lines.push(format!("{}- {}", prefix, yaml_value_to_inline(item)));
        }
    } else {
        lines.push(format!("{}{}", prefix, yaml_value_to_inline(value)));
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use saphyr::LoadableYamlNode;

    fn parse_yaml(raw: &str) -> Value {
        Value::load_from_str(raw)
            .unwrap()
            .into_iter()
            .next()
            .unwrap()
    }

    #[test]
    fn test_no_triggers() {
        let yaml = parse_yaml("Test:\n  death_link: true\n");
        let resolved = resolve_triggers(&yaml);
        let val = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(val.as_bool().unwrap());
    }

    #[test]
    fn test_top_level_trigger_fires() {
        let raw = r#"
triggers:
  - option_category: Test
    option_name: mode
    option_result: hard
    options:
      Test:
        death_link: true
Test:
  mode: hard
  death_link: false
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let dl = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(dl.as_bool().unwrap());
    }

    #[test]
    fn test_trigger_does_not_fire() {
        let raw = r#"
triggers:
  - option_category: Test
    option_name: mode
    option_result: hard
    options:
      Test:
        death_link: true
Test:
  mode: easy
  death_link: false
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let dl = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(!dl.as_bool().unwrap());
    }

    #[test]
    fn test_trigger_within_game_block() {
        let raw = r#"
Test:
  mode: hard
  death_link: false
  triggers:
    - option_category: Test
      option_name: mode
      option_result: hard
      options:
        Test:
          death_link: true
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let dl = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(dl.as_bool().unwrap());
    }

    #[test]
    fn test_trigger_with_weighted_map() {
        let raw = r#"
triggers:
  - option_category: Test
    option_name: mode
    option_result: hard
    options:
      Test:
        death_link: true
Test:
  mode:
    hard: 50
    easy: 50
  death_link: false
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let dl = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(dl.as_bool().unwrap());
    }

    #[test]
    fn test_trigger_weighted_map_zero_weight() {
        let raw = r#"
triggers:
  - option_category: Test
    option_name: mode
    option_result: hard
    options:
      Test:
        death_link: true
Test:
  mode:
    hard: 0
    easy: 50
  death_link: false
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let dl = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("death_link")
            .unwrap();
        assert!(!dl.as_bool().unwrap());
    }

    #[test]
    fn test_trigger_sets_new_option() {
        let raw = r#"
triggers:
  - option_category: Test
    option_name: mode
    option_result: hard
    options:
      Test:
        extra_option: enabled
Test:
  mode: hard
"#;
        let yaml = parse_yaml(raw);
        let resolved = resolve_triggers(&yaml);
        let extra = resolved
            .as_mapping_get("Test")
            .unwrap()
            .as_mapping_get("extra_option")
            .unwrap();
        assert_eq!(extra.as_str().unwrap(), "enabled");
    }

    #[test]
    fn test_max_iteration_cap() {
        // Two triggers that fire each other forever
        let raw = r#"
triggers:
  - option_category: Test
    option_name: a
    option_result: "1"
    options:
      Test:
        b: "1"
  - option_category: Test
    option_name: b
    option_result: "1"
    options:
      Test:
        a: "1"
Test:
  a: "1"
  b: "0"
"#;
        let yaml = parse_yaml(raw);
        // Should not infinite loop — just terminates after MAX_ITERATIONS
        let _resolved = resolve_triggers(&yaml);
    }
}
