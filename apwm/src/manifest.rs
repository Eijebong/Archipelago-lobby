use std::{collections::BTreeMap, path::Path};

use anyhow::{bail, Result};
use semver::Version;
use serde::Deserialize;

use crate::Index;

#[derive(Debug, Deserialize)]
pub struct ManifestCommon {
    pub archipelago_version: Version,
}

#[derive(Debug, Deserialize)]
pub struct WorldDef {
    #[serde(default)]
    pub version: Option<Version>,
}

#[derive(Debug, Deserialize)]
pub struct Manifest {
    pub common: ManifestCommon,
    pub worlds: BTreeMap<String, WorldDef>,
}

impl Manifest {
    pub fn new(archipelago_version: Version) -> Self {
        Self {
            common: ManifestCommon {
                archipelago_version,
            },
            worlds: BTreeMap::new(),
        }
    }

    pub fn from_index_with_latest_versions(index: &Index) -> Result<Self> {
        let mut result = Self::new(index.archipelago_version.clone());
        for (name, world) in index.worlds.iter() {
            let world_def = world.get_latest_release();
            if world_def.as_ref().map(|def| &def.version).is_none()  {
                bail!(format!(
                    "World `{}` has no known release",
                    name
                ));
            }

            result.worlds.insert(
                name.to_string(),
                WorldDef {
                    version: world_def.map(|def| def.version.clone()),
                },
            );
        }

        Ok(result)
    }

    pub fn parse_file(path: &Path) -> Result<Self> {
        let content = std::fs::read_to_string(path)?;

        Manifest::from_str(&content)
    }

    pub fn from_str(content: &str) -> Result<Self> {
        let deser = toml::Deserializer::new(content);

        let manifest: Manifest = serde_path_to_error::deserialize(deser)?;
        Ok(manifest)
    }
}

