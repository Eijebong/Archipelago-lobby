use std::collections::HashMap;

use crate::error::Result;
use anyhow::anyhow;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::db::YamlFile;

mod jd;
mod kh;
mod pokemon;
mod sv;
mod tunic;

#[derive(Debug, Serialize, Deserialize, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum YamlFeature {
    DeathLink,
    TrainerSanity,
    DexSanity,
    OrbSanity,
    GrassSanity,
    FishSanity,
}

pub type YamlFeatures = HashMap<YamlFeature, u32>;
pub type YamlGameFeatures<'a> = HashMap<&'a str, YamlFeatures>;

pub trait FeatureExtractor {
    fn game(&self) -> &'static str;
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()>;
}

const MAX_WEIGHT: u32 = 10000;

pub struct Extractor<'a> {
    game_features: YamlGameFeatures<'a>,
    current_game: Option<(&'a str, &'a Value, u32)>,
    current_weight: u32,
    yaml: &'a Value,
}

fn is_trueish(option: &Value) -> bool {
    if let Some(value) = option.as_bool() {
        return value;
    }

    if let Some(value) = option.as_i64() {
        return value != 0;
    }

    let option_str = option.as_str();
    if let Some(value) = option_str.and_then(|v| v.parse::<i64>().ok()) {
        return value != 0;
    }

    option_str == Some("true")
}

impl<'a> Extractor<'a> {
    pub fn new(yaml: &'a Value) -> Result<Extractor<'a>> {
        let Some(_) = yaml.as_mapping() else {
            Err(anyhow!("The main body of the YAML should be a map"))?
        };

        Ok(Self {
            game_features: YamlGameFeatures::new(),
            yaml,
            current_game: None,
            current_weight: MAX_WEIGHT,
        })
    }

    pub fn register_feature(&mut self, feature: YamlFeature, path: &str) -> Result<()> {
        let option_probability = self.get_option_probability(path, is_trueish)?;
        let feature_probability = self.get_weighted_probality(option_probability);

        self.add_feature_to_current_game(feature, feature_probability);

        Ok(())
    }

    pub fn register_feature_with_custom_truth(
        &mut self,
        feature: YamlFeature,
        path: &str,
        is_trueish: fn(&Value) -> bool,
    ) -> Result<()> {
        let option_probability = self.get_option_probability(path, is_trueish)?;
        let feature_probability = self.get_weighted_probality(option_probability);

        self.add_feature_to_current_game(feature, feature_probability);

        Ok(())
    }

    pub fn get_option_probability(
        &mut self,
        path: &str,
        is_true_callback: fn(&Value) -> bool,
    ) -> Result<u32> {
        let Some((_, game_yaml, _)) = self.current_game else {
            panic!("You should call set_game before")
        };

        let Some(option) = game_yaml.get(path) else {
            return Ok(0);
        };

        get_option_probability(option, is_true_callback)
    }

    pub fn register_ranged_feature(
        &mut self,
        feature: YamlFeature,
        path: &str,
        min: u64,
        max: u64,
        transform: fn(&Value) -> Result<u64>,
    ) -> Result<()> {
        let Some((_, game_yaml, _)) = self.current_game else {
            panic!("You should call set_game before")
        };

        let Some(option) = game_yaml.get(path) else {
            return Ok(());
        };

        let map = get_option_map::<u64>(option, transform)?;

        let total: u32 = map.values().sum();
        let probability: u32 = map
            .iter()
            .filter_map(|(option, probability)| {
                if *option < min || *option > max {
                    return Some(probability);
                }

                None
            })
            .sum();
        let actual_probability = ((probability as f64 / total as f64) * MAX_WEIGHT as f64) as u32;
        let feature_probability = self.get_weighted_probality(actual_probability);

        self.add_feature_to_current_game(feature, feature_probability);

        Ok(())
    }

    pub fn with_weight(&mut self, weight: u32, inner: fn(&mut Self) -> Result<()>) -> Result<()> {
        assert!(
            weight <= MAX_WEIGHT,
            "Maximum weight: {}, supplied weight: {}",
            MAX_WEIGHT,
            weight
        );

        self.current_weight = weight;
        inner(self)?;
        self.current_weight = MAX_WEIGHT;

        Ok(())
    }

    pub fn set_game(&mut self, game_name: &'a str, probability: u32) -> Result<()> {
        let Some(map) = self.yaml.as_mapping() else {
            Err(anyhow!("The main body of the YAML should be a map"))?
        };

        let Some(game_yaml) = map.get(game_name) else {
            Err(anyhow!(format!(
                "The requested game isn't present in the YAML: {}",
                game_name
            )))?
        };
        self.current_game = Some((game_name, game_yaml, probability));

        Ok(())
    }

    fn get_weighted_probality(&self, probability: u32) -> u32 {
        let Some((_, _, game_probability)) = self.current_game else {
            panic!("You should call set_game before")
        };

        (probability as f64
            * (game_probability as f64 / MAX_WEIGHT as f64)
            * (self.current_weight as f64 / MAX_WEIGHT as f64)) as u32
    }

    fn add_feature_to_current_game(&mut self, feature: YamlFeature, feature_probability: u32) {
        let Some((game_name, _, _)) = self.current_game else {
            panic!("You should call set_game before")
        };

        if feature_probability != 0 {
            let current_game_features = self.game_features.entry(game_name).or_default();
            let current_value = current_game_features.entry(feature).or_default();
            let not_current = MAX_WEIGHT - *current_value;
            let not_new = MAX_WEIGHT - feature_probability;
            *current_value = MAX_WEIGHT - ((not_current * not_new) / MAX_WEIGHT);
        }
    }

    fn finalize(self) -> YamlFeatures {
        let mut finalized_features = YamlFeatures::new();

        for (_, features) in self.game_features {
            for (feature, probability) in features {
                let current_value = finalized_features.entry(feature).or_default();
                *current_value += probability
            }
        }

        finalized_features
    }
}

fn get_option_map<K: for<'a> Deserialize<'a> + std::hash::Hash + Eq>(
    option: &serde_yaml::Value,
    transform: fn(&Value) -> Result<K>,
) -> Result<HashMap<K, u32>> {
    if let Ok(value) = transform(option) {
        return Ok(HashMap::from([(value, MAX_WEIGHT)]));
    }

    let Some(map) = option.as_mapping() else {
        Err(anyhow!(
            "Option should either be value or a mapping of the same type"
        ))?
    };

    let mut ret = HashMap::new();
    for (key, value) in map.iter() {
        let Ok(key) = transform(key) else { continue };
        if value != 0 {
            ret.insert(key, value.as_u64().unwrap() as u32);
        }
    }

    Ok(ret)
}

fn get_option_probability(
    option: &serde_yaml::Value,
    is_true_callback: fn(&Value) -> bool,
) -> Result<u32> {
    if is_true_callback(option) {
        return Ok(MAX_WEIGHT);
    }

    if option.is_mapping() {
        let map = option.as_mapping().unwrap();
        let total: u64 = map.values().filter_map(|v| v.as_u64()).sum();
        let mut on_count = 0;
        for (key, value) in map.iter() {
            if !value.is_u64() {
                continue;
            }

            if value != 0 && get_option_probability(key, is_true_callback)? != 0 {
                on_count += value.as_u64().unwrap();
            }
        }

        return Ok(((on_count as f64 / total as f64) * MAX_WEIGHT as f64) as u32);
    }

    Ok(0)
}

struct DefaultExtractor;

impl FeatureExtractor for DefaultExtractor {
    fn game(&self) -> &'static str {
        "None"
    }
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::DeathLink, "death_link")?;
        extractor.register_feature(YamlFeature::DeathLink, "deathlink")?;
        extractor.register_feature(YamlFeature::DeathLink, "DeathLink")?;
        Ok(())
    }
}

pub static EXTRACTORS: Lazy<HashMap<&'static str, Box<dyn FeatureExtractor + Send + Sync>>> =
    Lazy::new(|| {
        let mut extractors: HashMap<&'static str, Box<dyn FeatureExtractor + Send + Sync>> =
            HashMap::new();
        macro_rules! register {
            ($($ty: ident)::+) => {
                let obj = $($ty)::+ {};
                extractors.insert(obj.game(), Box::new(obj));
            };
        }

        register!(pokemon::PokemonRB);
        register!(pokemon::PokemonEmerald);
        register!(pokemon::PokemonCrystal);
        register!(pokemon::PokemonFrLg);
        register!(jd::JakAndDaxter);
        register!(tunic::Tunic);
        register!(kh::KingdomHearts);
        register!(sv::StardewValley);

        extractors
    });

pub fn extract_features(parsed: &YamlFile, raw_yaml: &str) -> Result<YamlFeatures> {
    let yaml: Value = serde_yaml::from_str(raw_yaml)?;
    let mut extractor = Extractor::new(&yaml)?;

    match &parsed.game {
        crate::db::YamlGame::Name(name) => {
            extract_features_from_yaml(&mut extractor, name.as_str(), MAX_WEIGHT)?;
        }
        crate::db::YamlGame::Map(map) => {
            let total: f64 = map.values().sum();
            for (game, weight) in map {
                if *weight == 0. {
                    continue;
                }
                let probability = (weight / total) * MAX_WEIGHT as f64;
                extract_features_from_yaml(&mut extractor, game.as_str(), probability as u32)?;
            }
        }
    }

    Ok(extractor.finalize())
}

fn extract_features_from_yaml<'a>(
    extractor: &mut Extractor<'a>,
    game_name: &'a str,
    probability: u32,
) -> Result<()> {
    extractor.set_game(game_name, probability)?;

    let default_extractor = DefaultExtractor {};
    default_extractor.extract_features(extractor)?;

    let Some(game_extractor) = EXTRACTORS.get(game_name) else {
        return Ok(());
    };

    game_extractor.extract_features(extractor)?;

    Ok(())
}

#[cfg(test)]
mod tests {
    use std::collections::HashMap;

    use crate::{error::Result, extractor::DefaultExtractor};
    use anyhow::anyhow;
    use serde_yaml::Value;

    use super::{Extractor, FeatureExtractor, YamlFeature};

    struct TestExtractor;
    impl FeatureExtractor for TestExtractor {
        fn game(&self) -> &'static str {
            "Test"
        }

        fn extract_features(&self, extractor: &mut super::Extractor) -> crate::error::Result<()> {
            extractor.register_feature(YamlFeature::DeathLink, "deathlink")?;
            extractor.register_feature(YamlFeature::DeathLink, "death_link")?;
            extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
            extractor.register_ranged_feature(YamlFeature::OrbSanity, "orbulons", 2, 5, |v| {
                Ok(v.as_u64().ok_or_else(|| anyhow!("Nope"))?)
            })?;
            extractor.with_weight(5000, |extractor: &mut Extractor| -> Result<()> {
                extractor.register_feature(YamlFeature::DeathLink, "half_deathlink")?;

                Ok(())
            })?;

            Ok(())
        }
    }

    #[test]
    fn test_extract_bool() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink: true
  trainersanity: false
  other_option: false
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_str_bool() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink: 'true'
  trainersanity: 'false'
  other_option: false
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_int() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink: 0
  trainersanity: 1
  other_option: 100
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::TrainerSanity, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_str_int() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink: 0
  trainersanity: '1'
  other_option: 100
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::TrainerSanity, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_decisive_map() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink:
    true: 50
    false: 0
  trainersanity:
    true: 0
    false: 50
  other_option: 100
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_indecisive_map() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink:
    true: 50
    false: 50
  trainersanity:
    true: 20
    false: 80
  other_option: 100
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([
            (YamlFeature::DeathLink, 5000),
            (YamlFeature::TrainerSanity, 2000),
        ]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_with_multiple_games() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 90
  Other: 10
Test:
  deathlink:
    true: 50
    false: 50
  trainersanity:
    true: 20
    false: 80
  other_option: 100
Other:
  death_link:
    true: 90
    false: 10
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 9000)?;
        game_extractor.extract_features(&mut extractor)?;
        extractor.set_game("Other", 1000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([
            (YamlFeature::DeathLink, 5400),
            (YamlFeature::TrainerSanity, 1800),
        ]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_ranged_feature_low() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 100
Test:
  orbulons:
    1: 50
    2: 0
    3: 0
    4: 0
    5: 0
    6: 0
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::OrbSanity, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_ranged_feature_high() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 100
Test:
  orbulons:
    1: 0
    2: 0
    3: 0
    4: 0
    5: 0
    6: 50
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::OrbSanity, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_ranged_feature_both() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 100
Test:
  orbulons:
    1: 50
    2: 0
    3: 0
    4: 0
    5: 0
    6: 50
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::OrbSanity, 10000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_ranged_feature_off() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 100
Test:
  orbulons:
    1: 0
    2: 0
    3: 50
    4: 0
    5: 0
    6: 0
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::new();
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_ranged_feature_prob() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
  Test: 100
Test:
  orbulons:
    1: 50
    2: 50
    3: 50
    4: 50
    5: 50
    6: 50
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::OrbSanity, 3333)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_extract_weighted() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  half_deathlink: true
  trainersanity: false
  other_option: false
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 5000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_double_deathlink() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
Test:
  deathlink:
    true: 50
    false: 50
  death_link:
    true: 50
    false: 50
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 10000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 7500)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }

    #[test]
    fn test_double_deathlink_in_games() -> Result<()> {
        let game_extractor = TestExtractor {};
        let raw_yaml = r#"
games:
    Test: 50
    Other: 50
Test:
  deathlink:
    true: 50
    false: 50
Other:
  deathlink:
    true: 50
    false: 50
        "#;
        let yaml: Value = serde_yaml::from_str(raw_yaml)?;
        let mut extractor = Extractor::new(&yaml)?;
        extractor.set_game("Test", 5000)?;
        game_extractor.extract_features(&mut extractor)?;
        extractor.set_game("Other", 5000)?;
        game_extractor.extract_features(&mut extractor)?;

        let expected = HashMap::from([(YamlFeature::DeathLink, 5000)]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }
}
