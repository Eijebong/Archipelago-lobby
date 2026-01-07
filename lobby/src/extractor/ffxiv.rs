use super::{is_trueish, Extractor, FeatureExtractor, Value, YamlFeature};
use crate::error::Result;

#[allow(clippy::upper_case_acronyms)]
pub struct FFXIV;

impl FeatureExtractor for FFXIV {
    fn game(&self) -> &'static str {
        "Manual_FFXIV_Silasary"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        extractor.register_feature_with_custom_truth(
            YamlFeature::FishSanity,
            "fishsanity",
            fishsanity_trueish,
        )?;

        Ok(())
    }
}

fn fishsanity_trueish(value: &Value) -> bool {
    if is_trueish(value) {
        return true;
    }

    if let Some(v) = value.as_str() {
        return v.ends_with("_fish");
    }

    false
}
