use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct FFXIV;

impl FeatureExtractor for FFXIV {
    fn game(&self) -> &'static str {
        "Manual_FFXIV_Silasary"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature(YamlFeature::FishSanity, "fishsanity")?;

        Ok(())
    }
}
