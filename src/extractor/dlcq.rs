use serde_yaml::Value;

use super::{Extractor, FeatureExtractor, YamlFeature};
use crate::error::Result;

pub struct DlcQuest;

impl FeatureExtractor for DlcQuest {
    fn game(&self) -> &'static str {
        "DLCQuest"
    }

    fn extract_features(&self, extractor: &mut Extractor) -> Result<()> {
        let coinsanity_weight = extractor
            .get_option_probability("coinsanity", |value| value.as_str() == Some("coin"))?;

        extractor.with_weight(
            coinsanity_weight,
            |extractor: &mut Extractor| -> Result<()> {
                extractor.register_ranged_feature(
                    YamlFeature::CoinSanity,
                    "coinbundlequantity",
                    5,
                    50,
                    coin_value_to_u64,
                )?;

                Ok(())
            },
        )?;

        Ok(())
    }
}

fn coin_value_to_u64(value: &Value) -> Result<u64> {
    if let Some(value) = value.as_u64() {
        return Ok(value);
    }

    let Some(value) = value.as_str() else {
        Err(anyhow::anyhow!("Invalid coin option value."))?
    };

    Ok(match value {
        "low" => 5,
        "normal" => 20,
        "high" => 50,
        // If it's random, return the worst possible value
        _ => 1,
    })
}
