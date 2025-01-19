use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct StardewValley;

impl FeatureExtractor for StardewValley {
    fn game(&self) -> &'static str {
        "Stardew Valley"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature_with_custom_truth(
            YamlFeature::FishSanity,
            "fishsanity",
            |value| {
                let Some(fishsanity) = value.as_str() else {
                    return false;
                };
                fishsanity != "none"
            },
        )?;

        Ok(())
    }
}
