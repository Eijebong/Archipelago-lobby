use std::{collections::BTreeMap, path::Path};

use anyhow::{bail, Result};
use semver::Version;
use serde::{Deserialize, Serialize};

use crate::Index;

#[derive(Debug, Serialize, Deserialize)]
pub struct ManifestCommon {
    pub archipelago_version: Version,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct WorldDef {
    pub version: Version,
}

#[derive(Debug, Serialize, Deserialize)]
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
        for (name, world) in index.worlds().iter() {
            let Some(world_def) = world.get_latest_release() else {
                bail!(format!("World `{}` has no known release", name));
            };

            result.worlds.insert(
                name.to_string(),
                WorldDef {
                    version: world_def.0.clone(),
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
