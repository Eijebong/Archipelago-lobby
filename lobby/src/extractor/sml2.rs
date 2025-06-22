use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct Sml2;

impl FeatureExtractor for Sml2 {
    fn game(&self) -> &'static str {
        "Super Mario Land 2"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::CoinSanity, "coinsanity")
    }
}
