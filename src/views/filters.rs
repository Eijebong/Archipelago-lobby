use itertools::Itertools;

use ap_lobby::db::Json;
use ap_lobby::extractor::{YamlFeature, YamlFeatures};

pub fn yaml_features(features: &Json<YamlFeatures>) -> askama::Result<String> {
    if features.is_empty() {
        return Ok(String::new());
    }

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
        YamlFeature::OrbSanity => "jd-orb.svg",
    }
}

fn feature_to_name(feature: &YamlFeature) -> &str {
    match feature {
        YamlFeature::DeathLink => "Deathlink",
        YamlFeature::TrainerSanity => "Trainersanity",
        YamlFeature::DexSanity => "Dexsanity",
        YamlFeature::OrbSanity => "Extreme Orbsanity",
    }
}

pub fn markdown(text: &str) -> askama::Result<String> {
    let parser = pulldown_cmark::Parser::new_ext(text, pulldown_cmark::Options::all());

    // Expect the output to be at least as big as the input
    let mut buf = String::with_capacity(text.len());
    pulldown_cmark::html::push_html(&mut buf, parser);

    Ok(buf)
}
