use std::convert::Infallible;
use std::fmt::Display;

use itertools::Itertools;

use crate::db::Json;
use crate::extractor::{YamlFeature, YamlFeatures};

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
        YamlFeature::GrassSanity => "grasssanity.svg",
        YamlFeature::FishSanity => "feesh.svg",
        YamlFeature::CoinSanity => "coin.svg",
        YamlFeature::DeathWish => "dw.svg",
        YamlFeature::DeathWishWithBonus => "dw-bonus.svg",
    }
}

fn feature_to_name(feature: &YamlFeature) -> &str {
    match feature {
        YamlFeature::DeathLink => "Deathlink",
        YamlFeature::TrainerSanity => "Trainersanity",
        YamlFeature::DexSanity => "Dexsanity",
        YamlFeature::OrbSanity => "Extreme Orbsanity",
        YamlFeature::GrassSanity => "Grasssanity",
        YamlFeature::FishSanity => "Fishsanity",
        YamlFeature::CoinSanity => "Extreme Coinsanity",
        YamlFeature::DeathWish => "DeathWish",
        YamlFeature::DeathWishWithBonus => "DeathWish with bonus",
    }
}

pub fn markdown(text: &str) -> askama::Result<impl Display, Infallible>
where
{
    use comrak::{markdown_to_html, Options};

    let mut defaults = Options::default();
    defaults.extension.strikethrough = true;
    defaults.extension.tagfilter = true;
    defaults.extension.table = true;
    defaults.extension.autolink = true;
    defaults.render.escape = true;

    let s = markdown_to_html(text, &defaults);
    Ok(s)
}
