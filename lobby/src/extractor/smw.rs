use super::{FeatureExtractor, YamlFeature};

pub struct Smw;

impl FeatureExtractor for Smw {
    fn game(&self) -> &'static str {
        "Super Mario World"
    }

    fn extract_features(&self, extractor: &mut super::Extractor) -> crate::error::Result<()> {
        extractor.register_feature(YamlFeature::BlockSanity, "blocksanity")?;

        Ok(())
    }
}
