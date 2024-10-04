use std::collections::HashMap;

use ap_lobby::error::Result;
use apwm::{Index, Manifest, NewApworldPolicy, VersionReq};
use rocket::FromForm;

#[derive(Debug, FromForm)]
pub struct ManifestForm<'a> {
    new_apworld_policy: &'a str,
    enabled: HashMap<&'a str, bool>,
    version: HashMap<&'a str, &'a str>,
}

pub struct ManifestFormBuilder {
    index: Index,
    pub manifest: Manifest,
}

#[derive(Debug)]
pub struct ManifestFormRow<'a> {
    pub apworld_name: &'a str,
    pub display_name: &'a str,
    pub enabled: bool,
    pub supported: bool,
    pub current_version: VersionReq,
    pub valid_versions: Vec<VersionReq>,
}

impl ManifestFormBuilder {
    pub fn new(index: Index, manifest: Manifest) -> Self {
        ManifestFormBuilder { index, manifest }
    }

    fn valid_options_for_apworld(&self, apworld_name: &str) -> Vec<VersionReq> {
        let world = self
            .index
            .worlds
            .get(apworld_name)
            .expect("Tried to get a world that doesn't exist");
        let latest_versions = &[VersionReq::Latest][..];

        latest_versions
            .iter()
            .cloned()
            .chain(world.versions.iter().map(|(version, origin)| {
                if origin.is_supported() {
                    return VersionReq::LatestSupported;
                }
                VersionReq::Specific(version.clone())
            }))
            .collect::<Vec<_>>()
    }

    pub fn rows(&self) -> Vec<ManifestFormRow> {
        let mut rows = Vec::with_capacity(self.index.worlds.len());
        for (apworld_name, world) in &self.index.worlds {
            let enabled = self.manifest.is_enabled(apworld_name);
            let current_version = self.manifest.get_version_req(apworld_name);

            let row = ManifestFormRow {
                apworld_name,
                display_name: &world.display_name,
                supported: world.supported,
                enabled,
                current_version,
                valid_versions: self.valid_options_for_apworld(apworld_name),
            };

            rows.push(row);
        }
        rows.sort_by_key(|row| row.display_name);

        rows
    }
}

pub fn manifest_from_form(form: &ManifestForm, index: &Index) -> Result<Manifest> {
    let mut new_manifest = Manifest::new();

    let new_apworld_policy = match form.new_apworld_policy {
        "disable" => NewApworldPolicy::Disable,
        _ => NewApworldPolicy::Enable,
    };

    new_manifest.new_apworld_policy = new_apworld_policy;
    for world_name in index.worlds.keys() {
        let enabled = form.enabled.contains_key(world_name.as_str());
        if !enabled {
            new_manifest.add_version_req(world_name, VersionReq::Disabled);
            continue;
        }

        let Some(version_req) = form.version.get(world_name.as_str()) else {
            Err(anyhow::anyhow!("Invalid form. You have a world that is enabled but doesn't have a version requirement"))?
        };

        new_manifest.add_version_req(world_name, VersionReq::parse(version_req)?);
    }

    let (_, errors) = new_manifest.resolve_with(index);
    if !errors.is_empty() {
        log::error!("{:?}", errors);
        Err(anyhow::anyhow!(
            "Error while resolving your room requirements with the current index"
        ))?
    }

    Ok(new_manifest)
}
