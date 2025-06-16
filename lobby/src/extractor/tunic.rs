use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct Tunic;

impl FeatureExtractor for Tunic {
    fn game(&self) -> &'static str {
        "TUNIC"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::GrassSanity, "grass_randomizer")?;
        Ok(())
    }
}
