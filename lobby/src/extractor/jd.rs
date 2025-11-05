use anyhow::anyhow;
use serde_yaml::Value;

use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct JakAndDaxter;

impl FeatureExtractor for JakAndDaxter {
    fn game(&self) -> &'static str {
        "Jak and Daxter: The Precursor Legacy"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        let global_orbsanity_weight = extractor
            .get_option_probability("enable_orbsanity", |value| value.as_str() == Some("global"))?;
        extractor.with_weight(
            global_orbsanity_weight,
            |extractor: &mut Extractor| -> Result<()> {
                extractor.register_ranged_feature(
                    YamlFeature::OrbSanity,
                    "global_orbsanity_bundle_size",
                    10,
                    200,
                    orb_value_to_u64,
                )?;

                Ok(())
            },
        )?;

        let per_level_weight = extractor.get_option_probability("enable_orbsanity", |value| {
            value.as_str() == Some("per_level")
        })?;
        extractor.with_weight(
            per_level_weight,
            |extractor: &mut Extractor| -> Result<()> {
                extractor.register_ranged_feature(
                    YamlFeature::OrbSanity,
                    "level_orbsanity_bundle_size",
                    10,
                    200,
                    orb_value_to_u64,
                )?;

                Ok(())
            },
        )?;
        Ok(())
    }
}

fn orb_value_to_u64(value: &Value) -> Result<u64> {
    let Some(value) = value.as_str() else {
        Err(anyhow!("Invalid orb option value"))?
    };

    let Some(value) = value.split('_').next() else {
        Err(anyhow!("Invalid orb option value. Should be x_orb(s)"))?
    };

    Ok(value.parse::<u64>()?)
}
