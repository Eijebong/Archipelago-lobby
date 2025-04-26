use std::convert::Infallible;
use std::fmt::Display;

use apwm::{World, WorldOrigin, WorldTag};
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
        YamlFeature::AfterDark => "tag-ad.svg",
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
        YamlFeature::AfterDark => "After Dark game",
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

fn tag_to_icon(tag: &WorldTag) -> &str {
    match tag {
        WorldTag::AfterDark => "tag-ad.svg",
    }
}

fn tag_to_name(tag: &WorldTag) -> &str {
    match tag {
        WorldTag::AfterDark => "After Dark",
    }
}

pub fn world_tags(
    world_and_origin: &(&World, &WorldOrigin),
) -> askama::Result<impl Display, Infallible> {
    let mut tags = String::new();
    let (world, origin) = world_and_origin;
    if origin.is_supported() {
        tags += "<img src=\"/static/images/icons/tag-core-verified.svg\" title=\"Core verified world\"/>";
    }

    for tag in &world.tags {
        tags += &format!(
            "<img src=\"/static/images/icons/{}\" title=\"{}\"/>",
            tag_to_icon(tag),
            tag_to_name(tag),
        );
    }

    Ok(tags)
}
