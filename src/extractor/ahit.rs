use super::{is_trueish, FeatureExtractor, YamlFeature, MAX_WEIGHT};

pub struct Ahit;

impl FeatureExtractor for Ahit {
    fn game(&self) -> &'static str {
        "A Hat in Time"
    }

    fn extract_features(&self, extractor: &mut super::Extractor) -> crate::error::Result<()> {
        let deathwish_probability =
            extractor.get_option_probability("EnableDeathWish", is_trueish)?;

        extractor.with_weight(deathwish_probability, |extractor| {
            extractor.add_feature_to_current_game(YamlFeature::DeathWish, MAX_WEIGHT);
            extractor.register_feature(YamlFeature::DeathWishWithBonus, "DWEnableBonus")
        })?;

        Ok(())
    }
}
