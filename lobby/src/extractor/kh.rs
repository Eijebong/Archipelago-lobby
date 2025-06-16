use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct KingdomHearts;

impl FeatureExtractor for KingdomHearts {
    fn game(&self) -> &'static str {
        "Kingdom Hearts"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::DeathLink, "donald_death_link")?;
        extractor.register_feature(YamlFeature::DeathLink, "goofy_death_link")?;
        Ok(())
    }
}
