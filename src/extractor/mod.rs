use std::collections::HashMap;

use crate::error::Result;
use anyhow::anyhow;
use once_cell::sync::Lazy;
use serde::{Deserialize, Serialize};
use serde_yaml::Value;

use crate::db::YamlFile;

#[derive(Debug, Serialize, Deserialize, Hash, Eq, PartialEq, Ord, PartialOrd)]
pub enum YamlFeature {
    DeathLink,
    TrainerSanity,
    DexSanity,
}

pub type YamlFeatures = HashMap<YamlFeature, u32>;

pub trait FeatureExtractor {
    fn game(&self) -> &'static str;
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()>;
}

pub struct Extractor<'a> {
    features: YamlFeatures,
    current_game: Option<(&'a Value, u32)>,
    yaml: &'a Value,
}

impl<'a> Extractor<'a> {
    pub fn new(yaml: &'a Value) -> Result<Extractor<'a>> {
        let Some(_) = yaml.as_mapping() else {
            Err(anyhow!("The main body of the YAML should be a map"))?
        };

        Ok(Self {
            features: YamlFeatures::new(),
            yaml,
            current_game: None,
        })
    }

    pub fn register_feature(&mut self, feature: YamlFeature, path: &str) -> Result<()> {
        let Some((game_yaml, game_probability)) = self.current_game else {
            panic!("You should call set_game before")
        };

        let Some(option) = game_yaml.get(path) else {
            return Ok(());
        };

        let option_probability = get_option_probability(option)?;

        if option_probability != 0 {
            let current_value = self.features.entry(feature).or_default();
            *current_value +=
                (option_probability as f64 * (game_probability as f64 / 10000.)) as u32;
        }

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
        self.current_game = Some((game_yaml, probability));

        Ok(())
    }

    fn finalize(self) -> YamlFeatures {
        self.features
    }
}

fn get_option_probability(option: &serde_yaml::Value) -> Result<u32> {
    if option.is_bool() {
        return Ok(if option.as_bool().unwrap() { 10000 } else { 0 });
    }

    if option.is_number() {
        return Ok(if option.as_i64().unwrap() != 0 {
            10000
        } else {
            0
        });
    }

    if option.is_string() {
        if option.as_str() == Some("true") {
            return Ok(10000);
        }
        return Ok(0);
    }

    if option.is_mapping() {
        let map = option.as_mapping().unwrap();
        let total: u64 = map.values().filter_map(|v| v.as_u64()).sum();
        let mut on_count = 0;
        for (key, value) in map.iter() {
            if !value.is_u64() {
                continue;
            }

            if value != 0 && get_option_probability(key)? != 0 {
                on_count += value.as_u64().unwrap();
            }
        }

        return Ok(((on_count as f64 / total as f64) * 10000.) as u32);
    }

    Ok(0)
}

struct PokemonEmerald;
struct PokemonCrystal;
struct PokemonFrLg;
struct DefaultExtractor;

impl FeatureExtractor for PokemonEmerald {
    fn game(&self) -> &'static str {
        "Pokemon Emerald"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
        extractor.register_feature(YamlFeature::DexSanity, "dexsanity")?;

        Ok(())
    }
}

impl FeatureExtractor for PokemonCrystal {
    fn game(&self) -> &'static str {
        "Pokemon Crystal"
    }
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
        Ok(())
    }
}

impl FeatureExtractor for PokemonFrLg {
    fn game(&self) -> &'static str {
        "Pokemon FireRed and LeafGreen"
    }
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
        Ok(())
    }
}

impl FeatureExtractor for DefaultExtractor {
    fn game(&self) -> &'static str {
        "None"
    }
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::DeathLink, "death_link")?;
        extractor.register_feature(YamlFeature::DeathLink, "deathlink")?;
        Ok(())
    }
}

pub static EXTRACTORS: Lazy<HashMap<&'static str, Box<dyn FeatureExtractor + Send + Sync>>> =
    Lazy::new(|| {
        let mut extractors: HashMap<&'static str, Box<dyn FeatureExtractor + Send + Sync>> =
            HashMap::new();
        macro_rules! register {
            ($ty: ident) => {
                let obj = $ty {};
                extractors.insert(obj.game(), Box::new(obj));
            };
        }

        register!(PokemonEmerald);
        register!(PokemonCrystal);
        register!(PokemonFrLg);

        extractors
    });

pub fn extract_features(parsed: &YamlFile, raw_yaml: &str) -> Result<YamlFeatures> {
    let yaml: Value = serde_yaml::from_str(raw_yaml)?;
    let mut extractor = Extractor::new(&yaml)?;

    match &parsed.game {
        crate::db::YamlGame::Name(name) => {
            extract_features_from_yaml(&mut extractor, name.as_str(), 10000)?;
        }
        crate::db::YamlGame::Map(map) => {
            let total: f64 = map.values().sum();
            for (game, weight) in map {
                if *weight == 0. {
                    continue;
                }
                let probability = (weight / total) * 10000.;
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
    use serde_yaml::Value;

    use super::{Extractor, FeatureExtractor, YamlFeature};

    struct TestExtractor;
    impl FeatureExtractor for TestExtractor {
        fn game(&self) -> &'static str {
            "Test"
        }

        fn extract_features(&self, extractor: &mut super::Extractor) -> crate::error::Result<()> {
            extractor.register_feature(YamlFeature::DeathLink, "deathlink")?;
            extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;

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
        DefaultExtractor {}.extract_features(&mut extractor)?;

        let expected = HashMap::from([
            (YamlFeature::DeathLink, 5400),
            (YamlFeature::TrainerSanity, 1800),
        ]);
        assert_eq!(extractor.finalize(), expected);

        Ok(())
    }
}
