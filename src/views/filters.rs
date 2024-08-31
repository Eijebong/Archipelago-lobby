use itertools::Itertools;

use ap_lobby::db::Json;
use ap_lobby::extractor::{YamlFeature, YamlFeatures};

pub fn yaml_features(features: &Json<YamlFeatures>) -> askama::Result<String> {
    let mut features_content = String::new();
    for (feature, probability) in features.0.iter().sorted() {
        features_content += &format!(
            "<img src=\"/static/images/icons/{}\" title=\"{}: {:.2}%\"/>",
            feature_to_icon(feature),
            feature_to_name(feature),
            *probability as f64 / 100.,
        );
    }

    Ok(format!(
        "<span class=\"yaml-features\">{}</span>",
        features_content
    ))
}

fn feature_to_icon(feature: &YamlFeature) -> &str {
    match feature {
        YamlFeature::DeathLink => "death-link.svg",
        YamlFeature::TrainerSanity => "trainersanity.svg",
        YamlFeature::DexSanity => "dexsanity.svg",
    }
}

fn feature_to_name(feature: &YamlFeature) -> &str {
    match feature {
        YamlFeature::DeathLink => "Deathlink",
        YamlFeature::TrainerSanity => "Trainersanity",
        YamlFeature::DexSanity => "Dexsanity",
    }
}
