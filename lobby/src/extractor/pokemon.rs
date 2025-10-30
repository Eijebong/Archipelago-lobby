use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct PokemonRB;
pub struct PokemonEmerald;
pub struct PokemonCrystal;
pub struct PokemonFrLg;

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
        extractor.register_feature(YamlFeature::TrainerSanity, "kanto_trainersanity")?;
        extractor.register_feature(YamlFeature::TrainerSanity, "johto_trainersanity")?;
        extractor.register_feature(YamlFeature::DexSanity, "dexsanity")?;
        extractor.register_feature(YamlFeature::DexSanity, "dexcountsanity")?;
        extractor.register_feature_with_custom_truth(
            YamlFeature::GrassSanity,
            "grasssanity",
            |value| {
                let Some(grasssanity) = value.as_str() else {
                    return false;
                };
                grasssanity != "off"
            },
        )?;
        Ok(())
    }
}

impl FeatureExtractor for PokemonFrLg {
    fn game(&self) -> &'static str {
        "Pokemon FireRed and LeafGreen"
    }
    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
        extractor.register_feature(YamlFeature::DexSanity, "dexsanity")?;
        Ok(())
    }
}

impl FeatureExtractor for PokemonRB {
    fn game(&self) -> &'static str {
        "Pokemon Red and Blue"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::TrainerSanity, "trainersanity")?;
        extractor.register_feature(YamlFeature::DexSanity, "dexsanity")?;
        Ok(())
    }
}
