use std::cell::RefCell;
use std::collections::HashMap;

use anyhow::Result;
use regex::{Regex, RegexBuilder};
use saphyr::YamlOwned as Value;
use serde::{Deserialize, Serialize};

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Severity {
    Info,
    Warning,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Outcome {
    Pass,
    Fail,
    Skipped,
    NotPresent,
    Error,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum RuleCheck {
    Truthy,
    Equals { value: String },
    NotEquals { value: String },
    GreaterThan { value: i64 },
    LessThan { value: i64 },
    Range { min: i64, max: i64 },
    Regex { pattern: String },
    Contains { value: String },
    Exists,
    NotExists,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum Predicate {
    Check {
        path: String,
        check: RuleCheck,
    },
    And {
        predicates: Vec<Predicate>,
    },
    Or {
        predicates: Vec<Predicate>,
    },
    Not {
        predicate: Box<Predicate>,
    },
}

impl Predicate {
    pub fn validate(&self) -> Result<(), &'static str> {
        match self {
            Predicate::Check { path, check } => {
                if path.trim().is_empty() {
                    return Err("Check predicate has an empty path");
                }
                if let RuleCheck::Regex { pattern } = check {
                    if Regex::new(pattern).is_err() {
                        return Err("Invalid regex pattern");
                    }
                }
                Ok(())
            }
            Predicate::And { predicates } | Predicate::Or { predicates } => {
                for p in predicates {
                    p.validate()?;
                }
                Ok(())
            }
            Predicate::Not { predicate } => predicate.validate(),
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Rule {
    pub name: String,
    pub game: Option<String>,
    pub when: Option<Predicate>,
    pub then: Predicate,
    pub severity: Severity,
}

impl Rule {
    pub fn validate(&self) -> Result<(), &'static str> {
        if let Some(ref when) = self.when {
            when.validate()?;
        }
        self.then.validate()
    }

    fn result(&self, outcome: Outcome, detail: Option<String>) -> RuleResult {
        RuleResult {
            rule_name: self.name.clone(),
            outcome,
            severity: self.severity.clone(),
            detail,
        }
    }
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct RuleResult {
    pub rule_name: String,
    pub outcome: Outcome,
    pub severity: Severity,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub detail: Option<String>,
}

fn is_trueish(option: &Value) -> bool {
    if let Some(value) = option.as_bool() {
        return value;
    }

    if let Some(value) = option.as_integer() {
        return value != 0;
    }

    let option_str = option.as_str();
    if let Some(value) = option_str.and_then(|v| v.parse::<i64>().ok()) {
        return value != 0;
    }

    if let Some(value) = option_str {
        let value = value.to_lowercase();
        return value == "true"
            || value == "all"
            || value == "full"
            || value.starts_with("random")
            || value.starts_with("enable");
    }

    false
}

fn yaml_value_as_string(val: &Value) -> String {
    super::yaml_value_as_string(val).unwrap_or_default()
}

fn yaml_value_as_i64(val: &Value) -> Option<i64> {
    if let Some(i) = val.as_integer() {
        return Some(i);
    }
    val.as_str().and_then(|s| s.parse::<i64>().ok())
}

fn navigate_path<'a>(yaml: &'a Value, path: &str) -> Option<&'a Value> {
    let mut current = yaml;
    for part in path.split('.') {
        current = current.as_mapping_get(part)?;
    }
    Some(current)
}

thread_local! {
    static REGEX_CACHE: RefCell<HashMap<String, Regex>> = RefCell::new(HashMap::new());
}

const REGEX_CACHE_MAX_ENTRIES: usize = 256;
const REGEX_SIZE_LIMIT: usize = 1_000_000;

fn cached_regex(pattern: &str) -> Result<Regex> {
    REGEX_CACHE.with(|cache| {
        let mut cache = cache.borrow_mut();
        if let Some(re) = cache.get(pattern) {
            return Ok(re.clone());
        }
        let re = RegexBuilder::new(pattern)
            .size_limit(REGEX_SIZE_LIMIT)
            .build()?;
        if cache.len() >= REGEX_CACHE_MAX_ENTRIES {
            cache.clear();
        }
        cache.insert(pattern.to_string(), re.clone());
        Ok(re)
    })
}

fn check_single_value(val: &Value, check: &RuleCheck) -> Result<bool> {
    match check {
        RuleCheck::Truthy => Ok(is_trueish(val)),
        RuleCheck::Equals { value } => Ok(yaml_value_as_string(val) == *value),
        RuleCheck::NotEquals { value } => Ok(yaml_value_as_string(val) != *value),
        RuleCheck::GreaterThan { value } => {
            Ok(yaml_value_as_i64(val).is_some_and(|v| v > *value))
        }
        RuleCheck::LessThan { value } => Ok(yaml_value_as_i64(val).is_some_and(|v| v < *value)),
        RuleCheck::Range { min, max } => {
            Ok(yaml_value_as_i64(val).is_some_and(|v| v >= *min && v <= *max))
        }
        RuleCheck::Regex { pattern } => {
            let re = cached_regex(pattern)?;
            Ok(re.is_match(&yaml_value_as_string(val)))
        }
        RuleCheck::Contains { value } => {
            if let Some(seq) = val.as_sequence() {
                return Ok(seq.iter().any(|item| yaml_value_as_string(item) == *value));
            }
            Ok(false)
        }
        RuleCheck::Exists | RuleCheck::NotExists => {
            unreachable!("Exists/NotExists handled before calling check_single_value")
        }
    }
}

fn evaluate_check(val: &Value, check: &RuleCheck) -> Result<bool> {
    if val.is_mapping() {
        let map = val.as_mapping().unwrap();
        // Check if this looks like a weighted map (keys are values, values are weights)
        let looks_like_weighted = map
            .iter()
            .all(|(_, v)| v.as_integer().is_some());

        if looks_like_weighted && !map.is_empty() {
            // "Can roll" semantics: check passes if ANY key with non-zero weight satisfies it
            for (key, weight) in map.iter() {
                let w = weight.as_integer().unwrap_or(0);
                if w == 0 {
                    continue;
                }
                if check_single_value(key, check)? {
                    return Ok(true);
                }
            }
            return Ok(false);
        }
    }

    check_single_value(val, check)
}

pub fn evaluate_predicate(predicate: &Predicate, game_yaml: &Value) -> Result<bool> {
    match predicate {
        Predicate::Check { path, check } => {
            match check {
                RuleCheck::Exists => Ok(navigate_path(game_yaml, path).is_some()),
                RuleCheck::NotExists => Ok(navigate_path(game_yaml, path).is_none()),
                _ => {
                    let Some(val) = navigate_path(game_yaml, path) else {
                        return Ok(false);
                    };
                    evaluate_check(val, check)
                }
            }
        }
        Predicate::And { predicates } => {
            for p in predicates {
                if !evaluate_predicate(p, game_yaml)? {
                    return Ok(false);
                }
            }
            Ok(true)
        }
        Predicate::Or { predicates } => {
            for p in predicates {
                if evaluate_predicate(p, game_yaml)? {
                    return Ok(true);
                }
            }
            Ok(false)
        }
        Predicate::Not { predicate } => Ok(!evaluate_predicate(predicate, game_yaml)?),
    }
}

pub fn evaluate_rule(rule: &Rule, yaml: &Value, game_name: &str) -> RuleResult {
    if let Some(ref game_filter) = rule.game
        && game_filter != game_name {
            return rule.result(Outcome::Skipped, None);
        }

    let Some(game_yaml) = yaml.as_mapping_get(game_name) else {
        return rule.result(Outcome::NotPresent, Some(format!("Game section '{}' not found", game_name)));
    };

    if let Some(ref when) = rule.when {
        match evaluate_predicate(when, game_yaml) {
            Ok(true) => {}
            Ok(false) => return rule.result(Outcome::Skipped, None),
            Err(e) => return rule.result(Outcome::Error, Some(format!("Error evaluating condition: {}", e))),
        }
    }

    match evaluate_predicate(&rule.then, game_yaml) {
        Ok(true) => rule.result(Outcome::Pass, None),
        Ok(false) => rule.result(Outcome::Fail, None),
        Err(e) => rule.result(Outcome::Error, Some(format!("Error evaluating rule: {}", e))),
    }
}

pub fn evaluate_rules_for_yaml(
    rules: &[Rule],
    yaml: &Value,
    game_name: &str,
) -> Vec<RuleResult> {
    rules
        .iter()
        .map(|rule| evaluate_rule(rule, yaml, game_name))
        .collect()
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
    fn test_truthy_bool() {
        let yaml = parse_yaml("Test:\n  death_link: true\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_truthy_false() {
        let yaml = parse_yaml("Test:\n  death_link: false\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_truthy_int() {
        let yaml = parse_yaml("Test:\n  death_link: 1\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_weighted_map_can_roll() {
        let yaml = parse_yaml("Test:\n  death_link:\n    true: 50\n    false: 50\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_weighted_map_cannot_roll() {
        let yaml = parse_yaml("Test:\n  death_link:\n    true: 0\n    false: 50\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_equals() {
        let yaml = parse_yaml("Test:\n  goal: ganon\n");
        let rule = Rule {
            name: "Goal check".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "goal".into(),
                check: RuleCheck::Equals {
                    value: "ganon".into(),
                },
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_not_equals() {
        let yaml = parse_yaml("Test:\n  goal: ganon\n");
        let rule = Rule {
            name: "Goal check".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "goal".into(),
                check: RuleCheck::NotEquals {
                    value: "triforce".into(),
                },
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_range() {
        let yaml = parse_yaml("Test:\n  starting_money: 50\n");
        let rule = Rule {
            name: "Money range".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "starting_money".into(),
                check: RuleCheck::Range { min: 10, max: 100 },
            },
            severity: Severity::Warning,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_range_out() {
        let yaml = parse_yaml("Test:\n  starting_money: 200\n");
        let rule = Rule {
            name: "Money range".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "starting_money".into(),
                check: RuleCheck::Range { min: 10, max: 100 },
            },
            severity: Severity::Warning,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_greater_than() {
        let yaml = parse_yaml("Test:\n  count: 5\n");
        let rule = Rule {
            name: "Count".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "count".into(),
                check: RuleCheck::GreaterThan { value: 3 },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_less_than() {
        let yaml = parse_yaml("Test:\n  count: 5\n");
        let rule = Rule {
            name: "Count".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "count".into(),
                check: RuleCheck::LessThan { value: 3 },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_regex() {
        let yaml = parse_yaml("Test:\n  mode: random_high\n");
        let rule = Rule {
            name: "Random mode".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "mode".into(),
                check: RuleCheck::Regex {
                    pattern: "^random.*".into(),
                },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_contains() {
        let yaml = parse_yaml("Test:\n  start_inventory:\n    - Sword\n    - Shield\n");
        let rule = Rule {
            name: "Has sword".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "start_inventory".into(),
                check: RuleCheck::Contains {
                    value: "Sword".into(),
                },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_contains_missing() {
        let yaml = parse_yaml("Test:\n  start_inventory:\n    - Sword\n    - Shield\n");
        let rule = Rule {
            name: "Has bow".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "start_inventory".into(),
                check: RuleCheck::Contains {
                    value: "Bow".into(),
                },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_exists() {
        let yaml = parse_yaml("Test:\n  death_link: true\n");
        let rule = Rule {
            name: "Has deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Exists,
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_not_exists() {
        let yaml = parse_yaml("Test:\n  death_link: true\n");
        let rule = Rule {
            name: "No plando".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "plando".into(),
                check: RuleCheck::NotExists,
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_nested_path() {
        let yaml = parse_yaml("Test:\n  accessibility:\n    trap_fill: junk\n");
        let rule = Rule {
            name: "Trap fill".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "accessibility.trap_fill".into(),
                check: RuleCheck::Equals {
                    value: "junk".into(),
                },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_and_predicate() {
        let yaml = parse_yaml("Test:\n  death_link: true\n  trainersanity: true\n");
        let rule = Rule {
            name: "Both on".into(),
            game: None,
            when: None,
            then: Predicate::And {
                predicates: vec![
                    Predicate::Check {
                        path: "death_link".into(),
                        check: RuleCheck::Truthy,
                    },
                    Predicate::Check {
                        path: "trainersanity".into(),
                        check: RuleCheck::Truthy,
                    },
                ],
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_or_predicate() {
        let yaml = parse_yaml("Test:\n  death_link: false\n  trainersanity: true\n");
        let rule = Rule {
            name: "Either on".into(),
            game: None,
            when: None,
            then: Predicate::Or {
                predicates: vec![
                    Predicate::Check {
                        path: "death_link".into(),
                        check: RuleCheck::Truthy,
                    },
                    Predicate::Check {
                        path: "trainersanity".into(),
                        check: RuleCheck::Truthy,
                    },
                ],
            },
            severity: Severity::Warning,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_not_predicate() {
        let yaml = parse_yaml("Test:\n  death_link: false\n");
        let rule = Rule {
            name: "No deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Not {
                predicate: Box::new(Predicate::Check {
                    path: "death_link".into(),
                    check: RuleCheck::Truthy,
                }),
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_when_condition_matches() {
        let yaml = parse_yaml("Test:\n  trainersanity: true\n  dexsanity: true\n");
        let rule = Rule {
            name: "Dex with trainers".into(),
            game: None,
            when: Some(Predicate::Check {
                path: "trainersanity".into(),
                check: RuleCheck::Truthy,
            }),
            then: Predicate::Check {
                path: "dexsanity".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Warning,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_when_condition_not_met_skips() {
        let yaml = parse_yaml("Test:\n  trainersanity: false\n  dexsanity: false\n");
        let rule = Rule {
            name: "Dex with trainers".into(),
            game: None,
            when: Some(Predicate::Check {
                path: "trainersanity".into(),
                check: RuleCheck::Truthy,
            }),
            then: Predicate::Check {
                path: "dexsanity".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Warning,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Skipped);
    }

    #[test]
    fn test_game_filter_match() {
        let yaml = parse_yaml("Pokemon Emerald:\n  death_link: true\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: Some("Pokemon Emerald".into()),
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Pokemon Emerald");
        assert_eq!(result.outcome, Outcome::Pass);
    }

    #[test]
    fn test_game_filter_mismatch_skips() {
        let yaml = parse_yaml("Tunic:\n  death_link: true\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: Some("Pokemon Emerald".into()),
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Tunic");
        assert_eq!(result.outcome, Outcome::Skipped);
    }

    #[test]
    fn test_missing_path_returns_fail() {
        let yaml = parse_yaml("Test:\n  other: true\n");
        let rule = Rule {
            name: "Deathlink".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "death_link".into(),
                check: RuleCheck::Truthy,
            },
            severity: Severity::Error,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Fail);
    }

    #[test]
    fn test_serde_roundtrip() {
        let rule = Rule {
            name: "Test rule".into(),
            game: Some("Pokemon Emerald".into()),
            when: Some(Predicate::Or {
                predicates: vec![
                    Predicate::Check {
                        path: "trainersanity".into(),
                        check: RuleCheck::Truthy,
                    },
                    Predicate::Check {
                        path: "dexsanity".into(),
                        check: RuleCheck::Truthy,
                    },
                ],
            }),
            then: Predicate::Not {
                predicate: Box::new(Predicate::Check {
                    path: "death_link".into(),
                    check: RuleCheck::Truthy,
                }),
            },
            severity: Severity::Error,
        };

        let json = serde_json::to_string(&rule).unwrap();
        let deserialized: Rule = serde_json::from_str(&json).unwrap();
        assert_eq!(deserialized.name, "Test rule");
        assert_eq!(deserialized.game, Some("Pokemon Emerald".into()));
    }

    #[test]
    fn test_weighted_map_equals_can_roll() {
        let yaml = parse_yaml("Test:\n  goal:\n    ganon: 30\n    triforce: 70\n");
        let rule = Rule {
            name: "Can roll ganon".into(),
            game: None,
            when: None,
            then: Predicate::Check {
                path: "goal".into(),
                check: RuleCheck::Equals {
                    value: "ganon".into(),
                },
            },
            severity: Severity::Info,
        };
        let result = evaluate_rule(&rule, &yaml, "Test");
        assert_eq!(result.outcome, Outcome::Pass);
    }
}
